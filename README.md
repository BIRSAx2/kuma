# Kuma

Kuma is a lightweight compiler backend written in Rust and based on QBE. It
accepts a compact SSA-based intermediate representation and emits AMD64 or
AArch64 assembly.

## Usage

```rust
use kuma::{Target, compile};

let source = r#"
export function w $main() {
@start
    ret 0
}
"#;

let assembly = compile(source, Target::Amd64SysV).expect("compilation failed");
```

Kuma supports `Amd64SysV`, `Amd64Apple`, `Aarch64Elf`, and `Aarch64Apple`.
Targets are explicit, so a parsed module can be inspected once and compiled
repeatedly without sharing compiler state:

```rust
use kuma::{Target, compile_module, parse};

let source = r#"
function w $identity(w %value) {
@start
    ret %value
}
"#;
let module = parse(source).expect("valid IR");

for function in module.functions() {
    println!("{} has {} blocks", function.name(), function.basic_blocks().len());
}

let elf = compile_module(&module, Target::Aarch64Elf).expect("AArch64 assembly");
let sysv = compile_module(&module, Target::Amd64SysV).expect("AMD64 assembly");
assert!(!elf.is_empty() && !sysv.is_empty());
```

The public `ir` module is read-only. Optimization, analysis, register
allocation, and machine representations are private implementation details.

## C interface

Build the optional C-compatible library with:

```sh
cargo build --release --features ffi
```

[`include/kuma.h`](include/kuma.h) declares compile, assemble, and link
operations. Every operation takes an explicit `KumaTarget`. Kuma initializes
returned `KumaBuffer` values; callers release them with `kuma_buffer_free`.
Assembly and linking are available only for the target matching the Linux or
macOS host, while `kuma_compile` can emit assembly for every supported target.
The toolchain driver honors `CC` and otherwise uses `cc`.

## Development

```sh
cargo test --all-targets
cargo clippy --all-targets --all-features -- -D warnings
```

## License

Kuma is available under the [MIT License](LICENSE).
