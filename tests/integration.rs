//! End-to-end tests using IR fixtures with embedded C drivers.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use kuma::Target;

/// Return the target for the host platform.
fn make_target() -> Target {
    if cfg!(target_os = "macos") {
        #[cfg(target_arch = "x86_64")]
        {
            Target::Amd64Apple
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Target::Aarch64Apple
        }
    } else {
        Target::Amd64SysV
    }
}

struct SsaTestFile {
    source: String,
    c_driver: String,
    expected_output: String,
}

/// Split an SSA fixture into IR, driver, and expected-output sections.
fn parse_ssa_file(contents: &str) -> SsaTestFile {
    let mut source_lines = Vec::new();
    let mut driver_lines = Vec::new();
    let mut output_lines = Vec::new();

    #[derive(PartialEq)]
    enum Section {
        Ir,
        Driver,
        Output,
    }

    let mut section = Section::Ir;

    for line in contents.lines() {
        match section {
            Section::Ir => {
                if line.starts_with("# >>> driver") {
                    section = Section::Driver;
                } else if line.starts_with("# >>> output") {
                    section = Section::Output;
                } else {
                    source_lines.push(line);
                }
            }
            Section::Driver => {
                if line.starts_with("# <<<") {
                    section = Section::Ir;
                } else {
                    let stripped = line
                        .strip_prefix("# ")
                        .unwrap_or_else(|| line.strip_prefix('#').unwrap_or(line));
                    driver_lines.push(stripped);
                }
            }
            Section::Output => {
                if line.starts_with("# <<<") {
                    section = Section::Ir;
                } else {
                    let stripped = line
                        .strip_prefix("# ")
                        .unwrap_or_else(|| line.strip_prefix('#').unwrap_or(line));
                    let stripped = stripped.strip_suffix('#').unwrap_or(stripped);
                    output_lines.push(stripped);
                }
            }
        }
    }

    SsaTestFile {
        source: source_lines.join("\n"),
        c_driver: driver_lines.join("\n"),
        expected_output: output_lines.join("\n"),
    }
}

fn ssa_test_path(test_name: &str) -> PathBuf {
    let p = PathBuf::from(format!("tests/fixtures/{}.ssa", test_name));
    assert!(p.exists(), "Test file not found: {}", p.display());
    p
}

fn run_ssa_test(test_name: &str) {
    let ssa_path = ssa_test_path(test_name);
    let contents = fs::read_to_string(&ssa_path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {}", ssa_path.display(), e));

    if let Some(first_line) = contents.lines().next()
        && first_line.starts_with("# skip")
    {
        let targets: Vec<&str> = first_line.split_whitespace().skip(2).collect();
        let current = match make_target() {
            Target::Amd64SysV => "amd64_sysv",
            Target::Amd64Apple => "amd64_apple",
            Target::Aarch64Elf => "arm64",
            Target::Aarch64Apple => "arm64_apple",
            _ => unreachable!("unknown Kuma target"),
        };
        if targets.iter().any(|&t| {
            t == current
                || t == "amd64" && current.starts_with("amd64")
                || t == "arm64" && current.starts_with("arm64")
        }) {
            eprintln!(
                "Skipping test '{}' (not supported on selected target)",
                test_name
            );
            return;
        }
    }

    let test_file = parse_ssa_file(&contents);

    let target = make_target();
    let asm = kuma::compile(&test_file.source, target)
        .unwrap_or_else(|e| panic!("kuma::compile failed for {}: {}", test_name, e));

    let tmp_dir = std::env::temp_dir().join(format!("kuma_test_{}", test_name));
    fs::create_dir_all(&tmp_dir).expect("Failed to create temp dir");

    let asm_path = tmp_dir.join("out.s");
    let drv_path = tmp_dir.join("driver.c");
    let exe_path = tmp_dir.join("test_exe");

    fs::write(&asm_path, &asm).unwrap_or_else(|e| panic!("Failed to write assembly: {}", e));

    let mut cc_args: Vec<&str> = vec!["-g", "-o"];
    let exe_str = exe_path.to_str().unwrap();
    let asm_str = asm_path.to_str().unwrap();
    let drv_str = drv_path.to_str().unwrap();

    cc_args.push(exe_str);

    let has_driver = !test_file.c_driver.trim().is_empty();
    if has_driver {
        fs::write(&drv_path, &test_file.c_driver)
            .unwrap_or_else(|e| panic!("Failed to write C driver: {}", e));
        cc_args.push(drv_str);
    }

    cc_args.push(asm_str);

    let cc_output = Command::new("cc")
        .args(&cc_args)
        .output()
        .expect("Failed to run cc (C compiler)");

    assert!(
        cc_output.status.success(),
        "cc failed for {}:\nstdout: {}\nstderr: {}",
        test_name,
        String::from_utf8_lossy(&cc_output.stdout),
        String::from_utf8_lossy(&cc_output.stderr),
    );

    let run_output = Command::new(&exe_path)
        .args(["a", "b", "c"])
        .output()
        .unwrap_or_else(|e| panic!("Failed to run test binary for {}: {}", test_name, e));

    let stdout = String::from_utf8_lossy(&run_output.stdout);
    let exit_code = run_output.status.code().unwrap_or(-1);

    let has_expected_output = !test_file.expected_output.trim().is_empty();

    if has_expected_output {
        let actual = stdout.trim_end();
        let expected = test_file.expected_output.trim_end();
        assert_eq!(
            actual, expected,
            "Output mismatch for test '{}'\n--- expected ---\n{}\n--- actual ---\n{}",
            test_name, expected, actual
        );
    } else {
        assert_eq!(
            exit_code, 0,
            "Test '{}' exited with code {} (expected 0)\nstdout: {}",
            test_name, exit_code, stdout
        );
    }
}

#[test]
fn test_parse_ssa_file_format() {
    let input = r#"
export
function w $add(w %a, w %b) {
@start
    %c =w add %a, %b
    ret %c
}

# >>> driver
# int add(int, int);
# int main() { return !(add(1, 2) == 3); }
# <<<

# >>> output
# hello world
# <<<
"#;

    let parsed = parse_ssa_file(input);

    assert!(parsed.source.contains("function w $add"));
    assert!(parsed.source.contains("ret %c"));
    assert!(!parsed.source.contains(">>> driver"));
    assert!(!parsed.source.contains(">>> output"));

    assert!(parsed.c_driver.contains("int add(int, int)"));
    assert!(parsed.c_driver.contains("int main()"));

    assert_eq!(parsed.expected_output.trim(), "hello world");
}

#[test]
fn test_parse_ssa_file_no_output() {
    let input = r#"
function w $f() {
@start
    ret 0
}

# >>> driver
# int f(void);
# int main() { return f(); }
# <<<
"#;

    let parsed = parse_ssa_file(input);
    assert!(parsed.c_driver.contains("int f(void)"));
    assert!(parsed.expected_output.trim().is_empty());
}

#[test]
fn test_parse_ssa_file_no_driver() {
    let input = r#"
function w $main() {
@start
    ret 0
}
"#;

    let parsed = parse_ssa_file(input);
    assert!(parsed.c_driver.trim().is_empty());
    assert!(parsed.expected_output.trim().is_empty());
    assert!(parsed.source.contains("function w $main"));
}

#[test]
fn test_ssa_test_files_exist() {
    let test_dir = Path::new("tests/fixtures");
    assert!(test_dir.exists(), "test fixture directory not found");

    let required = [
        "sum", "eucl", "collatz", "prime", "queen", "mandel", "abi1", "abi2", "abi3", "abi4",
        "abi5", "abi6", "abi7", "abi8", "fpcnv", "double", "load1", "load2", "load3", "mem1",
        "mem2", "mem3", "isel1", "isel2", "isel3", "spill1", "rega1",
    ];

    for name in &required {
        let p = test_dir.join(format!("{}.ssa", name));
        assert!(p.exists(), "Missing test file: {}.ssa", name);
    }
}

#[test]
fn test_sum() {
    run_ssa_test("sum");
}

#[test]
fn test_eucl() {
    run_ssa_test("eucl");
}

#[test]
fn test_euclc() {
    run_ssa_test("euclc");
}

#[test]
fn test_collatz() {
    run_ssa_test("collatz");
}

#[test]
fn test_prime() {
    run_ssa_test("prime");
}

#[test]
fn test_queen() {
    run_ssa_test("queen");
}

#[test]
fn test_mandel() {
    run_ssa_test("mandel");
}

#[test]
fn test_cprime() {
    run_ssa_test("cprime");
}

#[test]
fn test_abi1() {
    run_ssa_test("abi1");
}

#[test]
fn test_abi2() {
    run_ssa_test("abi2");
}

#[test]
fn test_abi3() {
    run_ssa_test("abi3");
}

#[test]
fn test_abi4() {
    run_ssa_test("abi4");
}

#[test]
fn test_abi5() {
    run_ssa_test("abi5");
}

#[test]
fn test_abi6() {
    run_ssa_test("abi6");
}

#[test]
fn test_abi7() {
    run_ssa_test("abi7");
}

#[test]
fn test_abi8() {
    run_ssa_test("abi8");
}

#[test]
fn test_fpcnv() {
    run_ssa_test("fpcnv");
}

#[test]
fn test_double() {
    run_ssa_test("double");
}

#[test]
fn test_load1() {
    run_ssa_test("load1");
}

#[test]
fn test_load2() {
    run_ssa_test("load2");
}

#[test]
fn test_load3() {
    run_ssa_test("load3");
}

#[test]
fn test_ldbits() {
    run_ssa_test("ldbits");
}

#[test]
fn test_ldhoist() {
    run_ssa_test("ldhoist");
}

#[test]
fn test_mem1() {
    run_ssa_test("mem1");
}

#[test]
fn test_mem2() {
    run_ssa_test("mem2");
}

#[test]
fn test_mem3() {
    run_ssa_test("mem3");
}

#[test]
fn test_isel1() {
    run_ssa_test("isel1");
}

#[test]
fn test_isel2() {
    run_ssa_test("isel2");
}

#[test]
fn test_isel3() {
    run_ssa_test("isel3");
}

#[test]
fn test_spill1() {
    run_ssa_test("spill1");
}

#[test]
fn test_rega1() {
    run_ssa_test("rega1");
}

#[test]
fn test_cmp1() {
    run_ssa_test("cmp1");
}

#[test]
fn test_fold1() {
    run_ssa_test("fold1");
}

#[test]
fn test_loop() {
    run_ssa_test("loop");
}

#[test]
fn test_philv() {
    run_ssa_test("philv");
}

#[test]
fn test_align() {
    run_ssa_test("align");
}

#[test]
fn test_conaddr() {
    run_ssa_test("conaddr");
}

#[test]
fn test_cup() {
    run_ssa_test("cup");
}

#[test]
fn test_dark() {
    run_ssa_test("dark");
}

#[test]
fn test_dynalloc() {
    run_ssa_test("dynalloc");
}

#[test]
fn test_echo() {
    run_ssa_test("echo");
}

#[test]
fn test_env() {
    run_ssa_test("env");
}

#[test]
fn test_fixarg() {
    run_ssa_test("fixarg");
}

#[test]
fn test_max() {
    run_ssa_test("max");
}

#[test]
fn test_puts10() {
    run_ssa_test("puts10");
}

#[test]
fn test_strcmp() {
    run_ssa_test("strcmp");
}

#[test]
fn test_strspn() {
    run_ssa_test("strspn");
}

#[test]
fn test_tls() {
    run_ssa_test("tls");
}

#[test]
fn test_vararg1() {
    run_ssa_test("vararg1");
}

#[test]
fn test_vararg2() {
    run_ssa_test("vararg2");
}

#[test]
fn test_showcase_coro() {
    let path = "/tmp/coro_minimal.ssa";
    if !std::path::Path::new(path).exists() {
        eprintln!("SKIP: {path} not found");
        return;
    }
    let input = std::fs::read_to_string(path).unwrap();
    let result = kuma::compile(&input, Target::Aarch64Apple);
    assert!(result.is_ok(), "Compilation failed: {:?}", result.err());
}
