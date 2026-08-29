use std::fmt;

use super::internal::{
    BlkId, Cls, Con, ConType, Dat, FieldType, Fn, Jmp, Lnk, OP_TABLE, Ref, SymType,
};
use crate::frontend::ParseResult;

macro_rules! semantic_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Copy, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(u32);

        impl $name {
            #[allow(dead_code)]
            pub(crate) fn new(index: usize) -> Self {
                Self(index as u32)
            }

            /// Return the source-order index represented by this ID.
            pub fn index(self) -> usize {
                self.0 as usize
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

semantic_id!(FunctionId, "A function in a parsed module.");
semantic_id!(BasicBlockId, "A basic block in a parsed function.");
semantic_id!(ValueId, "A named SSA value in a parsed function.");
semantic_id!(TypeId, "An aggregate type in a parsed module.");

/// Parsed semantic IR. Compilation clones its private compiler representation,
/// so a module can be inspected and compiled repeatedly or concurrently.
pub struct Module {
    pub(crate) parsed: ParseResult,
    functions: Vec<Function>,
    types: Vec<TypeDefinition>,
    data: Vec<DataDefinition>,
}

impl Module {
    pub(crate) fn from_parsed(parsed: ParseResult) -> Self {
        let functions = parsed
            .functions
            .iter()
            .enumerate()
            .map(|(index, function)| Function::from_internal(index, function))
            .collect();
        let types = parsed
            .types
            .iter()
            .enumerate()
            .map(|(index, definition)| TypeDefinition::from_internal(index, definition))
            .collect();
        let data = parsed
            .data
            .iter()
            .filter_map(|items| DataDefinition::from_internal(items))
            .collect();
        Self {
            parsed,
            functions,
            types,
            data,
        }
    }

    /// Functions in source order.
    pub fn functions(&self) -> impl ExactSizeIterator<Item = &Function> {
        self.functions.iter()
    }

    /// Aggregate type definitions in source order.
    pub fn type_definitions(&self) -> impl ExactSizeIterator<Item = &TypeDefinition> {
        self.types.iter()
    }

    /// Data definitions in source order.
    pub fn data_definitions(&self) -> impl ExactSizeIterator<Item = &DataDefinition> {
        self.data.iter()
    }

    /// Look up a function ID that originated from this module.
    pub fn function(&self, id: FunctionId) -> Option<&Function> {
        self.functions.get(id.index())
    }

    /// Look up a type ID that originated from this module.
    pub fn type_definition(&self, id: TypeId) -> Option<&TypeDefinition> {
        self.types.get(id.index())
    }
}

impl fmt::Debug for Module {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Module")
            .field("functions", &self.functions)
            .field("types", &self.types)
            .field("data", &self.data)
            .finish()
    }
}

/// Linkage attributes on a function or data definition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Linkage {
    exported: bool,
    thread_local: bool,
    alignment: Option<u8>,
    section: Option<String>,
    section_flags: Option<String>,
}

impl Linkage {
    fn from_internal(linkage: &Lnk) -> Self {
        Self {
            exported: linkage.export,
            thread_local: linkage.thread,
            alignment: (linkage.align != 0).then_some(linkage.align),
            section: linkage.sec.clone(),
            section_flags: linkage.secf.clone(),
        }
    }

    pub fn is_exported(&self) -> bool {
        self.exported
    }

    pub fn is_thread_local(&self) -> bool {
        self.thread_local
    }

    pub fn alignment(&self) -> Option<u8> {
        self.alignment
    }

    pub fn section(&self) -> Option<&str> {
        self.section.as_deref()
    }

    pub fn section_flags(&self) -> Option<&str> {
        self.section_flags.as_deref()
    }
}

/// One parsed function.
#[derive(Clone, Debug)]
pub struct Function {
    id: FunctionId,
    name: String,
    linkage: Linkage,
    variadic: bool,
    blocks: Vec<BasicBlock>,
}

impl Function {
    fn from_internal(index: usize, function: &Fn) -> Self {
        let blocks = function
            .blks
            .iter()
            .enumerate()
            .map(|(block_index, block)| BasicBlock::from_internal(block_index, block, function))
            .collect();
        Self {
            id: FunctionId::new(index),
            name: function.name.clone(),
            linkage: Linkage::from_internal(&function.lnk),
            variadic: function.vararg,
            blocks,
        }
    }

    pub fn id(&self) -> FunctionId {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn linkage(&self) -> &Linkage {
        &self.linkage
    }

    pub fn is_variadic(&self) -> bool {
        self.variadic
    }

    pub fn basic_blocks(&self) -> impl ExactSizeIterator<Item = &BasicBlock> {
        self.blocks.iter()
    }

    pub fn basic_block(&self, id: BasicBlockId) -> Option<&BasicBlock> {
        self.blocks.get(id.index())
    }
}

/// One source-level basic block.
#[derive(Clone, Debug)]
pub struct BasicBlock {
    id: BasicBlockId,
    name: String,
    phis: Vec<Phi>,
    instructions: Vec<Instruction>,
    terminator: Terminator,
}

impl BasicBlock {
    fn from_internal(index: usize, block: &super::internal::Blk, function: &Fn) -> Self {
        Self {
            id: BasicBlockId::new(index),
            name: block.name.clone(),
            phis: block
                .phi
                .iter()
                .map(|phi| Phi::from_internal(phi, function))
                .collect(),
            instructions: block
                .ins
                .iter()
                .map(|instruction| Instruction::from_internal(instruction, function))
                .collect(),
            terminator: Terminator::from_internal(block, function),
        }
    }

    pub fn id(&self) -> BasicBlockId {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn phis(&self) -> impl ExactSizeIterator<Item = &Phi> {
        self.phis.iter()
    }

    pub fn instructions(&self) -> impl ExactSizeIterator<Item = &Instruction> {
        self.instructions.iter()
    }

    pub fn terminator(&self) -> &Terminator {
        &self.terminator
    }
}

/// Scalar value classes supported by textual Kuma IR.
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub enum ValueClass {
    Word,
    Long,
    Single,
    Double,
}

impl ValueClass {
    fn from_internal(class: Cls) -> Option<Self> {
        match class {
            Cls::Kw => Some(Self::Word),
            Cls::Kl => Some(Self::Long),
            Cls::Ks => Some(Self::Single),
            Cls::Kd => Some(Self::Double),
            Cls::Kx => None,
        }
    }
}

/// A source-level constant. Its variant carries exactly the relevant payload.
#[derive(Clone, Debug, PartialEq)]
pub enum Constant {
    Undefined,
    Integer(i64),
    Single(f32),
    Double(f64),
    Symbol {
        name: String,
        thread_local: bool,
        offset: i64,
    },
}

impl Constant {
    fn from_internal(constant: &Con, function: &Fn) -> Self {
        match constant.typ {
            ConType::Undef => Self::Undefined,
            ConType::Bits if constant.flt == 1 => Self::Single(constant.bits.s()),
            ConType::Bits if constant.flt == 2 => Self::Double(constant.bits.d()),
            ConType::Bits => Self::Integer(constant.bits.i()),
            ConType::Addr => Self::Symbol {
                name: function
                    .strs
                    .get(constant.sym.id as usize)
                    .cloned()
                    .unwrap_or_default(),
                thread_local: constant.sym.typ == SymType::Thr,
                offset: constant.bits.i(),
            },
        }
    }
}

/// An operand in parsed semantic IR.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Temporary(ValueId),
    Constant(Constant),
    Type(TypeId),
}

impl Value {
    fn from_internal(reference: Ref, function: &Fn) -> Option<Self> {
        match reference {
            Ref::R => None,
            Ref::Tmp(id) => Some(Self::Temporary(ValueId(id.0))),
            Ref::Con(id) => function
                .cons
                .get(id.0 as usize)
                .map(|constant| Self::Constant(Constant::from_internal(constant, function))),
            Ref::Int(value) => Some(Self::Constant(Constant::Integer(value as i64))),
            Ref::Typ(id) => Some(Self::Type(TypeId(id.0))),
            // These forms are introduced only by private compiler passes and
            // cannot occur in parsed semantic IR.
            Ref::Slot(_) | Ref::Call(_) | Ref::Mem(_) => None,
        }
    }
}

/// A source instruction. Opcode representation is private to the compiler;
/// callers see its stable textual mnemonic and typed operands.
#[derive(Clone, Debug)]
pub struct Instruction {
    mnemonic: &'static str,
    value_class: Option<ValueClass>,
    result: Option<ValueId>,
    operands: Vec<Value>,
}

impl Instruction {
    fn from_internal(instruction: &super::internal::Ins, function: &Fn) -> Self {
        let result = match instruction.to {
            Ref::Tmp(id) => Some(ValueId(id.0)),
            _ => None,
        };
        Self {
            mnemonic: OP_TABLE[instruction.op as usize].name,
            value_class: ValueClass::from_internal(instruction.cls),
            result,
            operands: instruction
                .arg
                .iter()
                .filter_map(|operand| Value::from_internal(*operand, function))
                .collect(),
        }
    }

    pub fn mnemonic(&self) -> &'static str {
        self.mnemonic
    }

    pub fn value_class(&self) -> Option<ValueClass> {
        self.value_class
    }

    pub fn result(&self) -> Option<ValueId> {
        self.result
    }

    pub fn operands(&self) -> impl ExactSizeIterator<Item = &Value> {
        self.operands.iter()
    }
}

/// One incoming edge/value pair for a phi node.
#[derive(Clone, Debug, PartialEq)]
pub struct PhiInput {
    predecessor: BasicBlockId,
    value: Value,
}

impl PhiInput {
    pub fn predecessor(&self) -> BasicBlockId {
        self.predecessor
    }

    pub fn value(&self) -> &Value {
        &self.value
    }
}

/// A phi node with paired inputs; parallel predecessor/value vectors are not
/// exposed by the semantic IR.
#[derive(Clone, Debug)]
pub struct Phi {
    result: ValueId,
    value_class: ValueClass,
    inputs: Vec<PhiInput>,
}

impl Phi {
    fn from_internal(phi: &super::internal::Phi, function: &Fn) -> Self {
        let result = match phi.to {
            Ref::Tmp(id) => ValueId(id.0),
            _ => ValueId(u32::MAX),
        };
        let inputs = phi
            .blks
            .iter()
            .copied()
            .zip(phi.args.iter().copied())
            .filter_map(|(block, value)| {
                Value::from_internal(value, function).map(|value| PhiInput {
                    predecessor: BasicBlockId(block.0),
                    value,
                })
            })
            .collect();
        Self {
            result,
            value_class: ValueClass::from_internal(phi.cls).unwrap_or(ValueClass::Word),
            inputs,
        }
    }

    pub fn result(&self) -> ValueId {
        self.result
    }

    pub fn value_class(&self) -> ValueClass {
        self.value_class
    }

    pub fn inputs(&self) -> impl ExactSizeIterator<Item = &PhiInput> {
        self.inputs.iter()
    }
}

/// A basic-block terminator whose variant determines its valid payload.
#[derive(Clone, Debug, PartialEq)]
pub enum Terminator {
    Return(Option<Value>),
    Jump(BasicBlockId),
    Branch {
        condition: Value,
        if_nonzero: BasicBlockId,
        if_zero: BasicBlockId,
    },
    Halt,
}

impl Terminator {
    fn from_internal(block: &super::internal::Blk, function: &Fn) -> Self {
        let block_id = |id: Option<BlkId>| BasicBlockId(id.unwrap_or(BlkId::NONE).0);
        match block.jmp.typ {
            Jmp::Ret0 => Self::Return(None),
            jump if jump.is_ret() => Self::Return(Value::from_internal(block.jmp.arg, function)),
            Jmp::Jmp_ => Self::Jump(block_id(block.s1)),
            Jmp::Jnz => Self::Branch {
                condition: Value::from_internal(block.jmp.arg, function)
                    .unwrap_or(Value::Constant(Constant::Undefined)),
                if_nonzero: block_id(block.s1),
                if_zero: block_id(block.s2),
            },
            Jmp::Hlt | Jmp::Jxxx => Self::Halt,
            _ => Self::Branch {
                condition: Value::Constant(Constant::Undefined),
                if_nonzero: block_id(block.s1),
                if_zero: block_id(block.s2),
            },
        }
    }
}

/// Primitive and aggregate members of a type definition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TypeMember {
    Byte(u32),
    Half(u32),
    Word(u32),
    Long(u32),
    Single(u32),
    Double(u32),
    Padding(u32),
    Type { definition: TypeId, count: u32 },
}

impl TypeMember {
    fn merge_repetition(&mut self, next: &Self) -> bool {
        match (self, next) {
            (Self::Byte(count), Self::Byte(next))
            | (Self::Half(count), Self::Half(next))
            | (Self::Word(count), Self::Word(next))
            | (Self::Long(count), Self::Long(next))
            | (Self::Single(count), Self::Single(next))
            | (Self::Double(count), Self::Double(next)) => {
                *count += next;
                true
            }
            (
                Self::Type { definition, count },
                Self::Type {
                    definition: next_definition,
                    count: next_count,
                },
            ) if definition == next_definition => {
                *count += next_count;
                true
            }
            _ => false,
        }
    }
}

/// One alternative in a struct or union definition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TypeVariant {
    members: Vec<TypeMember>,
}

impl TypeVariant {
    pub fn members(&self) -> impl ExactSizeIterator<Item = &TypeMember> {
        self.members.iter()
    }
}

/// One named aggregate type definition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TypeDefinition {
    id: TypeId,
    name: String,
    opaque: bool,
    union: bool,
    alignment: Option<u32>,
    size: u64,
    variants: Vec<TypeVariant>,
}

impl TypeDefinition {
    fn from_internal(index: usize, definition: &super::internal::Typ) -> Self {
        let variants = definition
            .fields
            .iter()
            .map(|fields| {
                let mut members = Vec::new();
                for member in fields.iter().filter_map(|field| match field.typ {
                    FieldType::End => None,
                    // The internal parser expands repeated members into one field
                    // per occurrence and stores the primitive's byte width in
                    // `len`.  The semantic representation exposes occurrence
                    // counts instead, so every emitted primitive member has a
                    // count of one.
                    FieldType::Fb => Some(TypeMember::Byte(1)),
                    FieldType::Fh => Some(TypeMember::Half(1)),
                    FieldType::Fw => Some(TypeMember::Word(1)),
                    FieldType::Fl => Some(TypeMember::Long(1)),
                    FieldType::Fs => Some(TypeMember::Single(1)),
                    FieldType::Fd => Some(TypeMember::Double(1)),
                    FieldType::FPad => Some(TypeMember::Padding(field.len)),
                    FieldType::FTyp => Some(TypeMember::Type {
                        definition: TypeId(field.len),
                        count: 1,
                    }),
                }) {
                    if !members
                        .last_mut()
                        .is_some_and(|previous: &mut TypeMember| previous.merge_repetition(&member))
                    {
                        members.push(member);
                    }
                }
                TypeVariant { members }
            })
            .collect();
        Self {
            id: TypeId::new(index),
            name: definition.name.clone(),
            opaque: definition.is_dark,
            union: definition.is_union,
            alignment: (definition.align > 0).then_some(1u32 << definition.align),
            size: definition.size,
            variants,
        }
    }

    pub fn id(&self) -> TypeId {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn is_opaque(&self) -> bool {
        self.opaque
    }

    pub fn is_union(&self) -> bool {
        self.union
    }

    pub fn alignment(&self) -> Option<u32> {
        self.alignment
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn variants(&self) -> impl ExactSizeIterator<Item = &TypeVariant> {
        self.variants.iter()
    }
}

/// One initialized item in a data definition.
#[derive(Clone, Debug, PartialEq)]
pub enum DataItem {
    Byte(i64),
    Half(i64),
    Word(i64),
    Long(i64),
    Zero(u64),
    String(String),
    Symbol { name: String, offset: i64 },
    Single(f32),
    Double(f64),
}

/// One named data definition.
#[derive(Clone, Debug, PartialEq)]
pub struct DataDefinition {
    name: String,
    linkage: Linkage,
    items: Vec<DataItem>,
}

impl DataDefinition {
    fn from_internal(data: &[Dat]) -> Option<Self> {
        let start = data.first()?;
        let name = start.name.clone()?;
        let linkage = Linkage::from_internal(start.lnk.as_ref()?);
        let items = data
            .iter()
            .filter_map(|entry| match &entry.item {
                super::internal::DatItem::Start | super::internal::DatItem::End => None,
                super::internal::DatItem::Byte(value) => Some(DataItem::Byte(*value)),
                super::internal::DatItem::Half(value) => Some(DataItem::Half(*value)),
                super::internal::DatItem::Word(value) => Some(DataItem::Word(*value)),
                super::internal::DatItem::Long(value) => Some(DataItem::Long(*value)),
                super::internal::DatItem::Zero(size) => Some(DataItem::Zero(*size)),
                super::internal::DatItem::Str(value) => Some(DataItem::String(value.clone())),
                super::internal::DatItem::Ref { name, off } => Some(DataItem::Symbol {
                    name: name.clone(),
                    offset: *off,
                }),
                super::internal::DatItem::FltD(value) => Some(DataItem::Double(*value)),
                super::internal::DatItem::FltS(value) => Some(DataItem::Single(*value)),
            })
            .collect();
        Some(Self {
            name,
            linkage,
            items,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn linkage(&self) -> &Linkage {
        &self.linkage
    }

    pub fn items(&self) -> impl ExactSizeIterator<Item = &DataItem> {
        self.items.iter()
    }
}
