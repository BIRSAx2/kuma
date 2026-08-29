//! Parser tests for small IR snippets and fixture files.

use kuma::{ir::Module, parse};

fn parse_expect(input: &str, n_types: usize, n_data: usize, n_funcs: usize) -> Module {
    let result = parse(input).expect("source should parse");
    assert_eq!(
        result.type_definitions().count(),
        n_types,
        "expected {} types, got {}",
        n_types,
        result.type_definitions().count()
    );
    assert_eq!(
        result.data_definitions().count(),
        n_data,
        "expected {} data groups, got {}",
        n_data,
        result.data_definitions().count()
    );
    assert_eq!(
        result.functions().count(),
        n_funcs,
        "expected {} functions, got {}",
        n_funcs,
        result.functions().count()
    );
    result
}

#[test]
fn parse_empty_input() {
    let result = parse("").expect("empty source should parse");
    assert!(result.type_definitions().next().is_none());
    assert!(result.data_definitions().next().is_none());
    assert!(result.functions().next().is_none());
}

#[test]
fn parse_whitespace_only() {
    let result = parse("  \n\n  \t  \n").expect("whitespace should parse");
    assert!(result.type_definitions().next().is_none());
    assert!(result.data_definitions().next().is_none());
    assert!(result.functions().next().is_none());
}

#[test]
fn parse_comments_only() {
    let result = parse("# this is a comment\n# another comment\n").expect("comments should parse");
    assert!(result.type_definitions().next().is_none());
    assert!(result.data_definitions().next().is_none());
    assert!(result.functions().next().is_none());
}

#[test]
fn parse_empty_function() {
    let input = "function $empty() {\n@start\n    ret\n}\n";
    let result = parse_expect(input, 0, 0, 1);
    let f = result.functions().next().expect("function");
    assert_eq!(f.name(), "empty");
}

#[test]
fn parse_function_with_return_type() {
    let input = "function w $retw() {\n@start\n    ret 42\n}\n";
    let result = parse_expect(input, 0, 0, 1);
    let f = result.functions().next().expect("function");
    assert_eq!(f.name(), "retw");
}

#[test]
fn parse_function_with_params() {
    let input = "function w $add(w %a, w %b) {\n@start\n    %c =w add %a, %b\n    ret %c\n}\n";
    let result = parse_expect(input, 0, 0, 1);
    let f = result.functions().next().expect("function");
    assert_eq!(f.name(), "add");
    let instruction_count: usize = f
        .basic_blocks()
        .map(|block| block.instructions().count())
        .sum();
    assert!(
        instruction_count >= 3,
        "parameters and add should be represented"
    );
}

#[test]
fn parse_function_with_arithmetic() {
    let input = r#"
function w $arith(w %x) {
@start
    %a =w add %x, 1
    %b =w sub %a, 2
    %c =w mul %b, 3
    %d =w div %c, 4
    %e =w rem %d, 5
    ret %e
}
"#;
    let result = parse_expect(input, 0, 0, 1);
    let f = result.functions().next().expect("function");
    assert_eq!(f.name(), "arith");

    let start_blk = f
        .basic_blocks()
        .find(|block| block.name() == "start")
        .expect("start block");
    assert!(
        start_blk.instructions().count() >= 5,
        "expected at least 5 instructions, got {}",
        start_blk.instructions().count()
    );
}

#[test]
fn parse_function_with_control_flow() {
    let input = r#"
function w $max(w %a, w %b) {
@start
    %c =w csgtw %a, %b
    jnz %c, @left, @right
@left
    ret %a
@right
    ret %b
}
"#;
    let result = parse_expect(input, 0, 0, 1);
    let f = result.functions().next().expect("function");
    assert_eq!(f.name(), "max");
    assert!(
        f.basic_blocks().count() >= 3,
        "expected at least 3 blocks, got {}",
        f.basic_blocks().count()
    );
}

#[test]
fn parse_phi_nodes() {
    let input = r#"
function w $loop_sum(w %n) {
@start
    jmp @loop
@loop
    %i =w phi @start 0, @body %i1
    %s =w phi @start 0, @body %s1
    %c =w csltw %i, %n
    jnz %c, @body, @end
@body
    %s1 =w add %s, %i
    %i1 =w add %i, 1
    jmp @loop
@end
    ret %s
}
"#;
    let result = parse_expect(input, 0, 0, 1);
    let f = result.functions().next().expect("function");
    assert_eq!(f.name(), "loop_sum");

    let mut found_phi_block = false;
    for blk in f.basic_blocks() {
        if blk.name() == "loop" {
            assert_eq!(
                blk.phis().count(),
                2,
                "expected 2 phi nodes in @loop, got {}",
                blk.phis().count()
            );
            found_phi_block = true;
        }
    }
    assert!(found_phi_block, "did not find @loop block");
}

#[test]
fn parse_call_instruction() {
    let input = r#"
function $caller() {
@start
    %r =w call $add(w 1, w 2)
    ret
}
"#;
    let result = parse_expect(input, 0, 0, 1);
    let f = result.functions().next().expect("function");
    assert_eq!(f.name(), "caller");
}

#[test]
fn parse_jmp_instruction() {
    let input = r#"
function $jumpy() {
@start
    jmp @end
@end
    ret
}
"#;
    let result = parse_expect(input, 0, 0, 1);
    let f = result.functions().next().expect("function");
    assert_eq!(f.name(), "jumpy");
    assert!(f.basic_blocks().count() >= 2, "expected at least 2 blocks");
}

#[test]
fn parse_simple_type() {
    let input = "type :pair = { w, w }\n";
    let result = parse_expect(input, 1, 0, 0);
    let typ = result.type_definitions().next().expect("type");
    assert_eq!(typ.name(), "pair");
}

#[test]
fn parse_type_with_alignment() {
    let input = "type :vec = align 16 { w, w, w, w }\n";
    let result = parse_expect(input, 1, 0, 0);
    let typ = result.type_definitions().next().expect("type");
    assert_eq!(typ.name(), "vec");
    assert_eq!(typ.alignment(), Some(16));
}

#[test]
fn parse_type_with_byte_array() {
    let input = "type :mem = { b 17 }\n";
    let result = parse_expect(input, 1, 0, 0);
    let typ = result.type_definitions().next().expect("type");
    assert_eq!(typ.name(), "mem");
}

#[test]
fn parse_type_with_multiple_fields() {
    let input = "type :mixed = { w, l, s, d }\n";
    let result = parse_expect(input, 1, 0, 0);
    let typ = result.type_definitions().next().expect("type");
    assert_eq!(typ.name(), "mixed");
}

#[test]
fn parse_data_definition() {
    let input = r#"data $msg = { b "hello\n", b 0 }
"#;
    let result = parse_expect(input, 0, 1, 0);
    assert!(
        result
            .data_definitions()
            .next()
            .expect("data definition")
            .items()
            .next()
            .is_some()
    );
}

#[test]
fn parse_data_with_numbers() {
    let input = "data $arr = { w 1, w 2, w 3 }\n";
    let result = parse_expect(input, 0, 1, 0);
    assert!(
        result
            .data_definitions()
            .next()
            .expect("data definition")
            .items()
            .next()
            .is_some()
    );
}

#[test]
fn parse_data_with_zero_fill() {
    let input = "data $buf = { z 1024 }\n";
    let result = parse_expect(input, 0, 1, 0);
    assert!(
        result
            .data_definitions()
            .next()
            .expect("data definition")
            .items()
            .next()
            .is_some()
    );
}

#[test]
fn parse_export_function() {
    let input = "export function $visible() {\n@start\n    ret\n}\n";
    let result = parse_expect(input, 0, 0, 1);
    let f = result.functions().next().expect("function");
    assert_eq!(f.name(), "visible");
    assert!(f.linkage().is_exported(), "function should be exported");
}

#[test]
fn parse_non_export_function() {
    let input = "function $hidden() {\n@start\n    ret\n}\n";
    let result = parse_expect(input, 0, 0, 1);
    let f = result.functions().next().expect("function");
    assert_eq!(f.name(), "hidden");
    assert!(
        !f.linkage().is_exported(),
        "function should not be exported"
    );
}

#[test]
fn parse_section_data() {
    let input = r#"section ".rodata" data $ro = { b "readonly", b 0 }
"#;
    let _result = parse_expect(input, 0, 1, 0);
}

#[test]
fn parse_multiple_functions() {
    let input = r#"
function w $f1() {
@start
    ret 1
}

function w $f2() {
@start
    ret 2
}

function w $f3() {
@start
    ret 3
}
"#;
    let result = parse_expect(input, 0, 0, 3);
    assert_eq!(result.functions().next().expect("function 0").name(), "f1");
    assert_eq!(result.functions().nth(1).expect("function 1").name(), "f2");
    assert_eq!(result.functions().nth(2).expect("function 2").name(), "f3");
}

#[test]
fn parse_mixed_definitions() {
    let input = r#"
type :pair = { w, w }

data $msg = { b "hi", b 0 }

export
function w $main() {
@start
    ret 0
}
"#;
    let result = parse_expect(input, 1, 1, 1);
    assert_eq!(
        result.type_definitions().next().expect("type").name(),
        "pair"
    );
    assert_eq!(
        result.functions().next().expect("function 0").name(),
        "main"
    );
}

#[test]
fn parse_alloc_and_store_load() {
    let input = r#"
function w $mem() {
@start
    %p =l alloc4 4
    storew 42, %p
    %v =w loadw %p
    ret %v
}
"#;
    let result = parse_expect(input, 0, 0, 1);
    let f = result.functions().next().expect("function");
    assert_eq!(f.name(), "mem");
}

#[test]
fn parse_long_operations() {
    let input = r#"
function l $longadd(l %a, l %b) {
@start
    %c =l add %a, %b
    ret %c
}
"#;
    let result = parse_expect(input, 0, 0, 1);
    let f = result.functions().next().expect("function");
    assert_eq!(f.name(), "longadd");
}

#[test]
fn parse_float_operations() {
    let input = r#"
function s $fadd(s %a, s %b) {
@start
    %c =s add %a, %b
    ret %c
}
"#;
    let result = parse_expect(input, 0, 0, 1);
    let f = result.functions().next().expect("function");
    assert_eq!(f.name(), "fadd");
}

#[test]
fn parse_double_operations() {
    let input = r#"
function d $dadd(d %a, d %b) {
@start
    %c =d add %a, %b
    ret %c
}
"#;
    let result = parse_expect(input, 0, 0, 1);
    let f = result.functions().next().expect("function");
    assert_eq!(f.name(), "dadd");
}

#[test]
fn parse_extension_ops() {
    let input = r#"
function l $extend(w %x) {
@start
    %a =l extsw %x
    %b =l extuw %x
    ret %a
}
"#;
    let result = parse_expect(input, 0, 0, 1);
    let f = result.functions().next().expect("function");
    assert_eq!(f.name(), "extend");
}

#[test]
fn parse_sum_ssa() {
    let contents = std::fs::read_to_string("tests/fixtures/sum.ssa").expect("sum.ssa not found");
    let source: String = contents
        .lines()
        .take_while(|l| !l.starts_with("# >>> driver"))
        .collect::<Vec<_>>()
        .join("\n");
    let result = parse(&source).expect("sum fixture should parse");
    assert_eq!(
        result.functions().count(),
        1,
        "sum.ssa should have 1 function"
    );
    assert_eq!(result.functions().next().expect("function 0").name(), "sum");
}

#[test]
fn parse_eucl_ssa() {
    let contents = std::fs::read_to_string("tests/fixtures/eucl.ssa").expect("eucl.ssa not found");
    let source: String = contents
        .lines()
        .take_while(|l| !l.starts_with("# >>> driver"))
        .collect::<Vec<_>>()
        .join("\n");
    let result = parse(&source).expect("eucl fixture should parse");
    assert_eq!(result.functions().count(), 1);
    assert_eq!(
        result.functions().next().expect("function 0").name(),
        "test"
    );
}

#[test]
fn parse_abi1_ssa() {
    let contents = std::fs::read_to_string("tests/fixtures/abi1.ssa").expect("abi1.ssa not found");
    let source: String = contents
        .lines()
        .take_while(|l| !l.starts_with("# >>> driver"))
        .collect::<Vec<_>>()
        .join("\n");
    let result = parse(&source).expect("abi1 fixture should parse");
    assert_eq!(
        result.type_definitions().count(),
        1,
        "abi1.ssa should have 1 type"
    );
    assert_eq!(
        result.functions().count(),
        2,
        "abi1.ssa should have 2 functions"
    );
}
