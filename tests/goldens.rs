//! Compact assembly characterization goldens for representative programs.

use kuma::{Target, compile};

fn source_fixture(name: &str) -> String {
    let contents = std::fs::read_to_string(format!("tests/fixtures/{name}.ssa"))
        .unwrap_or_else(|error| panic!("failed to read {name}: {error}"));
    contents
        .lines()
        .take_while(|line| !line.starts_with("# >>> driver"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn stable_hash(text: &str) -> u64 {
    text.as_bytes()
        .iter()
        .fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
}

#[test]
fn representative_assembly_is_characterized_for_every_target() {
    let cases = [
        ("arithmetic", "sum"),
        ("control_flow", "collatz"),
        ("data", "align"),
        ("calls", "abi1"),
        ("varargs", "vararg1"),
        ("tls", "tls"),
    ];
    let targets = [
        ("amd64_sysv", Target::Amd64SysV),
        ("amd64_apple", Target::Amd64Apple),
        ("aarch64_elf", Target::Aarch64Elf),
        ("aarch64_apple", Target::Aarch64Apple),
    ];
    let expected = [
        [
            0xa534_8d72_987b_03e2,
            0x9081_b5f6_35ab_ecb1,
            0xa526_9b02_444d_a018,
            0x07eb_f673_d738_c4f2,
        ],
        [
            0x3f9d_9042_3f91_b0c0,
            0x71b9_7b0d_4ed9_0d5e,
            0x71bb_3cf4_011f_7fc0,
            0xad9f_7ce1_a664_aa97,
        ],
        [
            0x45fd_a740_d80b_8f46,
            0xffd6_07bb_e447_3840,
            0x3417_6b2d_16d5_4b57,
            0x79b3_49ee_00a7_52d4,
        ],
        [
            0x556d_3a03_59a2_fc02,
            0xd6b1_53c8_5251_5bb6,
            0xddb9_a491_4394_fd7d,
            0xc5e5_1004_4af7_9fa6,
        ],
        [
            0xb5ff_623d_9a09_4a90,
            0x45dd_9b48_10e1_7460,
            0x3013_482e_d938_2605,
            0x8831_513b_12ca_cabf,
        ],
        [
            0x3f30_bd90_9bd1_0693,
            0xb6c7_aa00_74a3_63fd,
            0xd6d8_c405_10d5_1c90,
            0xe35f_ca7e_6ccb_c391,
        ],
    ];
    let mut mismatches = Vec::new();

    for (case_index, (case_name, fixture)) in cases.into_iter().enumerate() {
        let source = source_fixture(fixture);
        for (target_index, (target_name, target)) in targets.into_iter().enumerate() {
            let assembly = compile(&source, target)
                .unwrap_or_else(|error| panic!("{case_name}/{target_name}: {error}"));
            let actual = stable_hash(&assembly);
            if actual != expected[case_index][target_index] {
                mismatches.push(format!("{case_name}/{target_name}: 0x{actual:016x}"));
            }
        }
    }

    assert!(
        mismatches.is_empty(),
        "assembly golden mismatch:\n{}",
        mismatches.join("\n")
    );
}
