//! One-shot compilation session and fixed pass sequence.

use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::Target;
use crate::analysis::{AnalysisState, Mutation, control_flow, use_def};
use crate::codegen::{aarch64, allocation, amd64, emission};
use crate::diagnostic::{Diagnostic, Span};
use crate::frontend::ParseError;
use crate::ir::Module;
use crate::ir::internal::{ConType, Fn, Jmp, Op, Ref, Target as MachineTarget, Typ};
use crate::transform::{copy, fold, load, memory, simplify};

/// A compilation failure.
#[derive(Debug)]
pub enum CompileError {
    /// The source could not be parsed.
    Parse(ParseError),
    /// Parsed input violated a semantic compiler invariant.
    InvalidIr(Diagnostic),
    /// The compiler itself failed unexpectedly.
    Internal(String),
}

impl CompileError {
    pub fn diagnostic(&self) -> Option<&Diagnostic> {
        match self {
            Self::Parse(error) => Some(error.diagnostic()),
            Self::InvalidIr(diagnostic) => Some(diagnostic),
            Self::Internal(_) => None,
        }
    }
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(error) => error.fmt(f),
            Self::InvalidIr(diagnostic) => write!(f, "invalid IR: {diagnostic}"),
            Self::Internal(message) => write!(f, "internal compiler error: {message}"),
        }
    }
}

impl std::error::Error for CompileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Parse(error) => Some(error),
            Self::InvalidIr(_) | Self::Internal(_) => None,
        }
    }
}

impl From<ParseError> for CompileError {
    fn from(error: ParseError) -> Self {
        Self::Parse(error)
    }
}

/// Compile textual Kuma IR for an explicit target.
pub fn compile(source: &str, target: Target) -> Result<String, CompileError> {
    let module = crate::parse(source)?;
    compile_module(&module, target)
}

/// Compile an already parsed module. The module is never mutated and may be
/// reused for other targets or by other threads.
pub fn compile_module(module: &Module, target: Target) -> Result<String, CompileError> {
    let result = catch_unwind(AssertUnwindSafe(|| {
        Session::new(target).run(module.parsed.clone())
    }));
    match result {
        Ok(output) => output.map_err(CompileError::InvalidIr),
        Err(payload) => {
            let message = panic_message(payload.as_ref());
            if looks_like_invalid_ir(&message) {
                Err(CompileError::InvalidIr(Diagnostic::new(
                    message,
                    Span::new(0, 0),
                    1,
                    1,
                )))
            } else {
                Err(CompileError::Internal(message))
            }
        }
    }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else {
        "compiler invariant failed".to_owned()
    }
}

fn looks_like_invalid_ir(message: &str) -> bool {
    [
        "invalid",
        "undefined",
        "violates ssa",
        "defined more than once",
        "missing target",
        "empty function",
    ]
    .iter()
    .any(|needle| message.contains(needle))
}

/// The selected private backend adapter. Selection happens exactly once when
/// a session is constructed.
#[derive(Copy, Clone)]
enum Backend {
    Amd64(&'static MachineTarget),
    Aarch64(&'static MachineTarget),
}

impl Backend {
    fn for_target(target: Target) -> Self {
        match target {
            Target::Amd64SysV => Self::Amd64(&amd64::T_AMD64_SYSV),
            Target::Amd64Apple => Self::Amd64(&amd64::T_AMD64_APPLE),
            Target::Aarch64Elf => Self::Aarch64(&aarch64::T_AARCH64_ELF),
            Target::Aarch64Apple => Self::Aarch64(&aarch64::T_AARCH64_APPLE),
        }
    }

    fn target(self) -> &'static MachineTarget {
        match self {
            Self::Amd64(target) | Self::Aarch64(target) => target,
        }
    }

    fn maximum_aggregate_alignment(self) -> i32 {
        match self {
            Self::Amd64(_) => 4,
            Self::Aarch64(_) => 3,
        }
    }

    fn abi0(self, function: &mut Fn) {
        match self {
            Self::Amd64(target) => amd64::abi0(function, target),
            Self::Aarch64(target) => aarch64::abi0(function, target),
        }
    }

    fn abi1(self, function: &mut Fn, types: &[Typ]) {
        match self {
            Self::Amd64(target) => amd64::abi1(function, target, types),
            Self::Aarch64(target) => aarch64::abi1(function, target, types),
        }
    }

    fn select(self, function: &mut Fn, state: &mut emission::EmissionState) {
        match self {
            Self::Amd64(target) => amd64::isel(function, target, &mut state.floating_literals),
            Self::Aarch64(target) => aarch64::isel(function, target, &mut state.floating_literals),
        }
    }

    fn emit(self, function: &mut Fn, output: &mut String, state: &mut emission::EmissionState) {
        match self {
            Self::Amd64(target) => amd64::emitfn(function, target, output, state),
            Self::Aarch64(target) => aarch64::emitfn(function, target, output, state),
        }
    }
}

/// State with exactly one compilation lifetime.
struct Session {
    backend: Backend,
    output: String,
    emission: emission::EmissionState,
}

impl Session {
    fn new(target: Target) -> Self {
        Self {
            backend: Backend::for_target(target),
            output: String::new(),
            emission: emission::EmissionState::new(),
        }
    }

    fn run(mut self, parsed: crate::frontend::ParseResult) -> Result<String, Diagnostic> {
        for data_group in &parsed.data {
            let mut state = emission::DatState::new();
            for data in data_group {
                emission::emitdat(
                    data,
                    &mut state,
                    Some(self.backend.target()),
                    &mut self.output,
                );
            }
        }

        for mut function in parsed.functions {
            self.compile_function(&mut function, &parsed.types)?;
        }

        emission::emitfin(
            &self.emission.floating_literals,
            self.backend.target(),
            &mut self.output,
        );
        Ok(self.output)
    }

    fn compile_function(&mut self, function: &mut Fn, types: &[Typ]) -> Result<(), Diagnostic> {
        let mut analyses = AnalysisState::default();
        self.validate_function(function, types)?;
        self.backend.abi0(function);

        analyses.rebuild_control_flow(function);
        analyses.rebuild_predecessors(function);
        analyses.rebuild_uses(function);

        memory::promote(function);
        analyses.invalidate(Mutation::Instructions);
        analyses.rebuild_uses(function);

        use_def::ssa(function).map_err(invalid_ir)?;
        analyses.invalidate(Mutation::Instructions);
        analyses.rebuild_uses(function);
        use_def::ssacheck(function).map_err(invalid_ir)?;

        analyses.rebuild_aliases(function);
        analyses.require_aliases();
        load::loadopt(function);
        analyses.invalidate(Mutation::Instructions);
        analyses.rebuild_uses(function);

        analyses.rebuild_aliases(function);
        analyses.require_aliases();
        memory::coalesce(function);
        analyses.invalidate(Mutation::Instructions);
        analyses.rebuild_uses(function);
        use_def::ssacheck(function).map_err(invalid_ir)?;

        copy::copy(function);
        analyses.invalidate(Mutation::Instructions);
        analyses.rebuild_uses(function);

        fold::fold(function);
        analyses.invalidate(Mutation::ControlFlow);
        self.backend.abi1(function, types);
        analyses.invalidate(Mutation::Instructions);
        simplify::simpl(function);
        analyses.invalidate(Mutation::ControlFlow);

        analyses.rebuild_control_flow(function);
        analyses.rebuild_predecessors(function);
        analyses.rebuild_uses(function);
        self.backend.select(function, &mut self.emission);
        analyses.invalidate(Mutation::Instructions);

        analyses.rebuild_control_flow(function);
        analyses.rebuild_liveness(function, self.backend.target());
        analyses.rebuild_loops(function);
        allocation::spill::fillcost(function);
        analyses.mark_spill_costs();

        allocation::spill::spill(function, self.backend.target());
        allocation::register::rega(function, self.backend.target());
        analyses.invalidate(Mutation::ControlFlow);

        analyses.rebuild_control_flow(function);
        control_flow::simpljmp(function);
        analyses.invalidate(Mutation::ControlFlow);
        analyses.rebuild_predecessors(function);
        analyses.rebuild_control_flow(function);

        assert!(
            !function.rpo.is_empty(),
            "function must have at least one block"
        );
        debug_assert_eq!(
            function.rpo[0], function.start,
            "first RPO block must be the entry block"
        );
        self.backend
            .emit(function, &mut self.output, &mut self.emission);
        Ok(())
    }

    fn validate_function(&self, function: &Fn, types: &[Typ]) -> Result<(), Diagnostic> {
        let validate_type = |index: usize| -> Result<(), Diagnostic> {
            let Some(definition) = types.get(index) else {
                return Err(invalid_ir(format!(
                    "function ${} references unknown type {index}",
                    function.name
                )));
            };
            if definition.align > self.backend.maximum_aggregate_alignment() {
                return Err(invalid_ir(format!(
                    "type :{} has alignment unsupported by the selected target",
                    definition.name
                )));
            }
            Ok(())
        };

        if function.retty >= 0 {
            validate_type(function.retty as usize)?;
        }

        for block in &function.blks {
            if block.jmp.typ == Jmp::Jxxx {
                return Err(invalid_ir(format!(
                    "block @{} is referenced but never defined",
                    block.name
                )));
            }
            if block.jmp.typ == Jmp::Jmp_ && block.s1.is_none() {
                return Err(invalid_ir(format!(
                    "jump in @{} is missing its target",
                    block.name
                )));
            }
            if block.jmp.typ == Jmp::Jnz && (block.s1.is_none() || block.s2.is_none()) {
                return Err(invalid_ir(format!(
                    "branch in @{} is missing a target",
                    block.name
                )));
            }

            for phi in &block.phi {
                if phi.args.len() != phi.blks.len() {
                    return Err(invalid_ir(format!(
                        "phi in @{} has unpaired inputs",
                        block.name
                    )));
                }
            }

            for instruction in &block.ins {
                for reference in instruction.arg {
                    if let Ref::Typ(id) = reference {
                        validate_type(id.0 as usize)?;
                    }
                }
                if matches!(instruction.op, Op::Alloc4 | Op::Alloc8 | Op::Alloc16)
                    && let Ref::Con(id) = instruction.arg[0]
                {
                    let constant = &function.cons[id.0 as usize];
                    let size = constant.bits.i();
                    if constant.typ != ConType::Bits || size < 0 || size >= (i32::MAX - 15) as i64 {
                        return Err(invalid_ir(format!(
                            "invalid allocation size {size} in @{}",
                            block.name
                        )));
                    }
                }
            }
        }

        Ok(())
    }
}

fn invalid_ir(message: String) -> Diagnostic {
    Diagnostic::new(message, Span::new(0, 0), 1, 1)
}
