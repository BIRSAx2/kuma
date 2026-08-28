# Kuma

Kuma is a lightweight compiler backend written in Rust and based on QBE. It
accepts a compact SSA-based intermediate representation and emits AMD64 or
AArch64 assembly.

## Usage

```rust
use kuma::{amd64::T_AMD64_SYSV, compile};

let source = r#"
export function w $main() {
@start
    ret 0
}
"#;

let assembly = compile(source, &T_AMD64_SYSV).expect("compilation failed");
```

Kuma provides System V and Apple targets for AMD64, and ELF and Apple targets
for AArch64.

## Development

```sh
cargo test --all-targets
cargo clippy --all-targets --all-features -- -D warnings
```

## License

Kuma is available under the [MIT License](LICENSE).
