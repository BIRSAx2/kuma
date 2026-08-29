//! Tests for Kuma's read-only semantic IR and typed facade.

use kuma::ir::{Constant, DataItem, Terminator, TypeMember, Value, ValueClass};
use kuma::{CompileError, Target, compile, compile_module, parse};

const SIMPLE: &str = r#"
export function w $add(w %left, w %right) {
@start
    %sum =w add %left, %right
    ret %sum
}
"#;

const BRANCH: &str = r#"
function w $choose(w %condition) {
@start
    jnz %condition, @yes, @no
@yes
    ret 1
@no
    ret 0
}
"#;

const PHI: &str = r#"
function w $select(w %condition) {
@start
    jnz %condition, @left, @right
@left
    jmp @join
@right
    jmp @join
@join
    %result =w phi @left 1, @right 2
    ret %result
}
"#;

fn simple_module() -> kuma::ir::Module {
    parse(SIMPLE).expect("simple module should parse")
}

#[test]
fn empty_module_has_no_declarations() {
    let module = parse("").expect("empty module");
    assert_eq!(module.functions().count(), 0);
    assert_eq!(module.type_definitions().count(), 0);
    assert_eq!(module.data_definitions().count(), 0);
}

#[test]
fn function_name_is_readable() {
    let module = simple_module();
    assert_eq!(module.functions().next().expect("function").name(), "add");
}

#[test]
fn function_id_is_source_ordered() {
    let module = simple_module();
    assert_eq!(module.functions().next().expect("function").id().index(), 0);
}

#[test]
fn function_id_is_printable() {
    let module = simple_module();
    assert_eq!(
        module
            .functions()
            .next()
            .expect("function")
            .id()
            .to_string(),
        "0"
    );
}

#[test]
fn function_lookup_accepts_owned_id() {
    let module = simple_module();
    let id = module.functions().next().expect("function").id();
    assert_eq!(module.function(id).expect("lookup").name(), "add");
}

#[test]
fn exported_linkage_is_visible() {
    let module = simple_module();
    assert!(
        module
            .functions()
            .next()
            .expect("function")
            .linkage()
            .is_exported()
    );
}

#[test]
fn ordinary_linkage_is_not_exported() {
    let module = parse(BRANCH).expect("branch module");
    assert!(
        !module
            .functions()
            .next()
            .expect("function")
            .linkage()
            .is_exported()
    );
}

#[test]
fn variadic_function_is_visible() {
    let module = parse("function $f(w %x, ...) {\n@start\n ret\n}\n").expect("variadic");
    assert!(module.functions().next().expect("function").is_variadic());
}

#[test]
fn ordinary_function_is_not_variadic() {
    assert!(
        !simple_module()
            .functions()
            .next()
            .expect("function")
            .is_variadic()
    );
}

#[test]
fn basic_blocks_preserve_source_order() {
    let module = parse(BRANCH).expect("branch module");
    let names: Vec<_> = module
        .functions()
        .next()
        .expect("function")
        .basic_blocks()
        .map(|block| block.name())
        .collect();
    assert_eq!(names, ["start", "yes", "no"]);
}

#[test]
fn basic_block_ids_are_source_ordered() {
    let module = parse(BRANCH).expect("branch module");
    let ids: Vec<_> = module
        .functions()
        .next()
        .expect("function")
        .basic_blocks()
        .map(|block| block.id().index())
        .collect();
    assert_eq!(ids, [0, 1, 2]);
}

#[test]
fn basic_block_lookup_uses_typed_id() {
    let module = parse(BRANCH).expect("branch module");
    let function = module.functions().next().expect("function");
    let id = function.basic_blocks().nth(1).expect("block").id();
    assert_eq!(function.basic_block(id).expect("lookup").name(), "yes");
}

#[test]
fn instruction_mnemonic_is_readable() {
    let module = simple_module();
    let add = module
        .functions()
        .next()
        .expect("function")
        .basic_blocks()
        .next()
        .expect("block")
        .instructions()
        .find(|instruction| instruction.mnemonic() == "add")
        .expect("add instruction");
    assert_eq!(add.mnemonic(), "add");
}

#[test]
fn instruction_class_is_typed() {
    let module = simple_module();
    let add = module
        .functions()
        .next()
        .expect("function")
        .basic_blocks()
        .next()
        .expect("block")
        .instructions()
        .find(|instruction| instruction.mnemonic() == "add")
        .expect("add instruction");
    assert_eq!(add.value_class(), Some(ValueClass::Word));
}

#[test]
fn instruction_result_uses_typed_id() {
    let module = simple_module();
    let result = module
        .functions()
        .next()
        .expect("function")
        .basic_blocks()
        .next()
        .expect("block")
        .instructions()
        .find(|instruction| instruction.mnemonic() == "add")
        .expect("add instruction")
        .result()
        .expect("result");
    assert!(result.index() > 0);
}

#[test]
fn instruction_operands_are_values() {
    let module = simple_module();
    let operand_count = module
        .functions()
        .next()
        .expect("function")
        .basic_blocks()
        .next()
        .expect("block")
        .instructions()
        .find(|instruction| instruction.mnemonic() == "add")
        .expect("add instruction")
        .operands()
        .count();
    assert_eq!(operand_count, 2);
}

#[test]
fn return_terminator_carries_value() {
    let module = simple_module();
    let terminator = module
        .functions()
        .next()
        .expect("function")
        .basic_blocks()
        .next()
        .expect("block")
        .terminator();
    assert!(matches!(
        terminator,
        Terminator::Return(Some(Value::Temporary(_)))
    ));
}

#[test]
fn void_return_has_no_value() {
    let module = parse("function $f() {\n@start\n ret\n}\n").expect("void function");
    let terminator = module
        .functions()
        .next()
        .unwrap()
        .basic_blocks()
        .next()
        .unwrap()
        .terminator();
    assert_eq!(terminator, &Terminator::Return(None));
}

#[test]
fn jump_terminator_carries_one_target() {
    let module = parse(PHI).expect("phi module");
    let function = module.functions().next().expect("function");
    let left = function
        .basic_blocks()
        .find(|block| block.name() == "left")
        .unwrap();
    assert!(matches!(left.terminator(), Terminator::Jump(_)));
}

#[test]
fn branch_terminator_carries_condition_and_targets() {
    let module = parse(BRANCH).expect("branch module");
    let block = module
        .functions()
        .next()
        .unwrap()
        .basic_blocks()
        .next()
        .unwrap();
    assert!(matches!(block.terminator(), Terminator::Branch { .. }));
}

#[test]
fn halt_terminator_is_typed() {
    let module = parse("function $f() {\n@start\n hlt\n}\n").expect("halt function");
    let block = module
        .functions()
        .next()
        .unwrap()
        .basic_blocks()
        .next()
        .unwrap();
    assert_eq!(block.terminator(), &Terminator::Halt);
}

#[test]
fn phi_nodes_are_exposed() {
    let module = parse(PHI).expect("phi module");
    let join = module
        .functions()
        .next()
        .unwrap()
        .basic_blocks()
        .find(|b| b.name() == "join")
        .unwrap();
    assert_eq!(join.phis().count(), 1);
}

#[test]
fn phi_result_is_typed() {
    let module = parse(PHI).expect("phi module");
    let join = module
        .functions()
        .next()
        .unwrap()
        .basic_blocks()
        .find(|b| b.name() == "join")
        .unwrap();
    assert!(join.phis().next().unwrap().result().index() > 0);
}

#[test]
fn phi_class_is_typed() {
    let module = parse(PHI).expect("phi module");
    let join = module
        .functions()
        .next()
        .unwrap()
        .basic_blocks()
        .find(|b| b.name() == "join")
        .unwrap();
    assert_eq!(join.phis().next().unwrap().value_class(), ValueClass::Word);
}

#[test]
fn phi_inputs_are_paired() {
    let module = parse(PHI).expect("phi module");
    let join = module
        .functions()
        .next()
        .unwrap()
        .basic_blocks()
        .find(|b| b.name() == "join")
        .unwrap();
    assert_eq!(join.phis().next().unwrap().inputs().count(), 2);
}

#[test]
fn phi_input_has_predecessor_and_value() {
    let module = parse(PHI).expect("phi module");
    let join = module
        .functions()
        .next()
        .unwrap()
        .basic_blocks()
        .find(|b| b.name() == "join")
        .unwrap();
    let input = join.phis().next().unwrap().inputs().next().unwrap();
    assert!(input.predecessor().index() > 0);
    assert!(matches!(
        input.value(),
        Value::Constant(Constant::Integer(1))
    ));
}

const TYPES: &str = r#"
type :pair = align 16 { w, l }
type :choice = { { w } { l } }
type :opaque = align 8 { 0 }
"#;

#[test]
fn type_names_preserve_source_order() {
    let module = parse(TYPES).expect("types");
    let names: Vec<_> = module.type_definitions().map(|ty| ty.name()).collect();
    assert_eq!(names, ["pair", "choice", "opaque"]);
}

#[test]
fn type_id_is_source_ordered() {
    let module = parse(TYPES).expect("types");
    assert_eq!(module.type_definitions().nth(1).unwrap().id().index(), 1);
}

#[test]
fn type_lookup_uses_typed_id() {
    let module = parse(TYPES).expect("types");
    let id = module.type_definitions().nth(1).unwrap().id();
    assert_eq!(module.type_definition(id).unwrap().name(), "choice");
}

#[test]
fn type_alignment_is_in_bytes() {
    let module = parse(TYPES).expect("types");
    assert_eq!(
        module.type_definitions().next().unwrap().alignment(),
        Some(16)
    );
}

#[test]
fn type_size_is_available() {
    let module = parse(TYPES).expect("types");
    assert!(module.type_definitions().next().unwrap().size() >= 16);
}

#[test]
fn struct_has_one_variant() {
    let module = parse(TYPES).expect("types");
    assert_eq!(
        module.type_definitions().next().unwrap().variants().count(),
        1
    );
}

#[test]
fn struct_members_are_typed() {
    let module = parse(TYPES).expect("types");
    let ty = module.type_definitions().next().unwrap();
    let members: Vec<_> = ty.variants().next().unwrap().members().collect();
    assert!(matches!(members[0], TypeMember::Word(1)));
    assert!(matches!(members[1], TypeMember::Padding(4)));
    assert!(matches!(members[2], TypeMember::Long(1)));
}

#[test]
fn repeated_type_members_are_grouped() {
    let module = parse("type :bytes = { b 4 }\n").expect("array type");
    let member = module
        .type_definitions()
        .next()
        .unwrap()
        .variants()
        .next()
        .unwrap()
        .members()
        .next()
        .unwrap();
    assert!(matches!(member, TypeMember::Byte(4)));
}

#[test]
fn union_is_identified() {
    let module = parse(TYPES).expect("types");
    assert!(module.type_definitions().nth(1).unwrap().is_union());
}

#[test]
fn opaque_type_is_identified() {
    let module = parse(TYPES).expect("types");
    assert!(module.type_definitions().nth(2).unwrap().is_opaque());
}

const DATA: &str = r#"
export data $items = { b 1, h 2, w 3, l 4, z 5, b "x", l $symbol+8, s s_1.5, d d_2.5 }
"#;

fn data_items() -> Vec<DataItem> {
    parse(DATA)
        .expect("data")
        .data_definitions()
        .next()
        .expect("definition")
        .items()
        .cloned()
        .collect()
}

#[test]
fn data_name_is_readable() {
    let module = parse(DATA).expect("data");
    assert_eq!(module.data_definitions().next().unwrap().name(), "items");
}

#[test]
fn data_linkage_is_readable() {
    let module = parse(DATA).expect("data");
    assert!(
        module
            .data_definitions()
            .next()
            .unwrap()
            .linkage()
            .is_exported()
    );
}

#[test]
fn byte_data_is_typed() {
    assert!(data_items().iter().any(|item| item == &DataItem::Byte(1)));
}

#[test]
fn half_data_is_typed() {
    assert!(data_items().iter().any(|item| item == &DataItem::Half(2)));
}

#[test]
fn word_data_is_typed() {
    assert!(data_items().iter().any(|item| item == &DataItem::Word(3)));
}

#[test]
fn long_data_is_typed() {
    assert!(data_items().iter().any(|item| item == &DataItem::Long(4)));
}

#[test]
fn zero_data_is_typed() {
    assert!(data_items().iter().any(|item| item == &DataItem::Zero(5)));
}

#[test]
fn string_data_is_typed() {
    assert!(
        data_items()
            .iter()
            .any(|item| item == &DataItem::String("x".into()))
    );
}

#[test]
fn symbol_data_is_typed() {
    assert!(
        data_items()
            .iter()
            .any(|item| matches!(item, DataItem::Symbol { name, offset: 8 } if name == "symbol"))
    );
}

#[test]
fn single_data_is_typed() {
    assert!(
        data_items()
            .iter()
            .any(|item| item == &DataItem::Single(1.5))
    );
}

#[test]
fn double_data_is_typed() {
    assert!(
        data_items()
            .iter()
            .any(|item| item == &DataItem::Double(2.5))
    );
}

#[test]
fn parse_error_has_message() {
    let error = parse("function").expect_err("invalid source");
    assert!(!error.diagnostic().message().is_empty());
}

#[test]
fn parse_error_has_one_based_line() {
    let error = parse("\nfunction").expect_err("invalid source");
    assert_eq!(error.diagnostic().line(), 2);
}

#[test]
fn parse_error_has_one_based_column() {
    let error = parse("function").expect_err("invalid source");
    assert!(error.diagnostic().column() >= 1);
}

#[test]
fn parse_error_has_byte_span() {
    let error = parse("function").expect_err("invalid source");
    assert!(error.diagnostic().span().start() <= "function".len());
}

#[test]
fn parse_error_display_is_contextual() {
    let error = parse("function").expect_err("invalid source");
    assert!(error.to_string().contains("parse error"));
}

#[test]
fn compile_propagates_parse_error() {
    assert!(matches!(
        compile("function", Target::Amd64SysV),
        Err(CompileError::Parse(_))
    ));
}

#[test]
fn malformed_source_never_unwinds() {
    for source in [
        "function",
        "function w $missing_body()",
        "data $unterminated = { b \"text",
        "type :bad = { nonsense }",
        "function $bad() {\n@start\n jnz 1, @only\n}\n",
    ] {
        let result = std::panic::catch_unwind(|| parse(source));
        assert!(result.is_ok(), "parser unwound for {source:?}");
        assert!(result.unwrap().is_err(), "parser accepted {source:?}");
    }
}

#[test]
fn invalid_ssa_is_a_compile_error() {
    let source = "function w $bad() {\n@start\n ret %undefined\n}\n";
    assert!(matches!(
        compile(source, Target::Amd64SysV),
        Err(CompileError::InvalidIr(_))
    ));
}

#[test]
fn validation_errors_never_unwind() {
    let cases = [
        (
            "function $bad() {\n@start\n %p =l alloc4 -1\n ret\n}\n",
            Target::Amd64SysV,
        ),
        (
            "function $bad() {\n@start\n jmp @missing\n}\n",
            Target::Amd64SysV,
        ),
        (
            "type :wide = align 16 { l }\nfunction $bad(:wide %value) {\n@start\n ret\n}\n",
            Target::Aarch64Elf,
        ),
    ];

    for (source, target) in cases {
        let result = std::panic::catch_unwind(|| compile(source, target));
        assert!(result.is_ok(), "validation unwound for {source:?}");
        assert!(matches!(result.unwrap(), Err(CompileError::InvalidIr(_))));
    }
}

macro_rules! target_compilation_test {
    ($name:ident, $target:expr) => {
        #[test]
        fn $name() {
            let assembly = compile(SIMPLE, $target).expect("compile target");
            assert!(assembly.contains("add"));
        }
    };
}

target_compilation_test!(compile_amd64_sysv, Target::Amd64SysV);
target_compilation_test!(compile_amd64_apple, Target::Amd64Apple);
target_compilation_test!(compile_aarch64_elf, Target::Aarch64Elf);
target_compilation_test!(compile_aarch64_apple, Target::Aarch64Apple);

#[test]
fn repeated_compilation_is_deterministic() {
    let first = compile(SIMPLE, Target::Amd64SysV).expect("first");
    let second = compile(SIMPLE, Target::Amd64SysV).expect("second");
    assert_eq!(first, second);
}

#[test]
fn interleaved_compilation_is_isolated() {
    let first = compile(SIMPLE, Target::Amd64SysV).expect("first");
    let _other = compile(SIMPLE, Target::Aarch64Elf).expect("other");
    let second = compile(SIMPLE, Target::Amd64SysV).expect("second");
    assert_eq!(first, second);
}

#[test]
fn concurrent_compilation_is_isolated() {
    let expected = compile(SIMPLE, Target::Amd64SysV).expect("expected");
    let handles: Vec<_> = (0..8)
        .map(|_| std::thread::spawn(|| compile(SIMPLE, Target::Amd64SysV).expect("thread")))
        .collect();
    for handle in handles {
        assert_eq!(handle.join().expect("thread panicked"), expected);
    }
}

#[test]
fn parsed_module_compiles_repeatedly() {
    let module = simple_module();
    let first = compile_module(&module, Target::Amd64SysV).expect("first");
    let second = compile_module(&module, Target::Amd64SysV).expect("second");
    assert_eq!(first, second);
}

#[test]
fn parsed_module_compiles_for_multiple_targets() {
    let module = simple_module();
    let amd64 = compile_module(&module, Target::Amd64SysV).expect("amd64");
    let aarch64 = compile_module(&module, Target::Aarch64Elf).expect("aarch64");
    assert_ne!(amd64, aarch64);
}

#[test]
fn function_declarations_preserve_order() {
    let module =
        parse("function $a() {\n@x\n ret\n}\nfunction $b() {\n@y\n ret\n}\n").expect("functions");
    assert_eq!(
        module.functions().map(|f| f.name()).collect::<Vec<_>>(),
        ["a", "b"]
    );
}

#[test]
fn data_declarations_preserve_order() {
    let module = parse("data $a = { w 1 }\ndata $b = { w 2 }\n").expect("data");
    assert_eq!(
        module
            .data_definitions()
            .map(|data| data.name())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
}

#[test]
fn thread_local_data_linkage_is_visible() {
    let module = parse("thread data $value = { w 1 }\n").expect("TLS data");
    assert!(
        module
            .data_definitions()
            .next()
            .unwrap()
            .linkage()
            .is_thread_local()
    );
}
