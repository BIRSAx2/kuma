//! Kuma is a lightweight compiler backend for a compact SSA-based IR.
//!
//! The crate's interface is intentionally small: parse source into read-only
//! semantic IR, then compile that module for an explicit [`Target`]. Lowering,
//! analyses, register allocation, and machine backends are implementation
//! details.

mod analysis;
mod codegen;
mod compiler;
mod diagnostic;
mod frontend;
pub mod ir;
mod transform;

#[cfg(feature = "ffi")]
mod ffi;

pub use compiler::{CompileError, compile, compile_module};
pub use diagnostic::{Diagnostic, Span};
pub use frontend::{ParseError, parse};

/// A supported assembly target.
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum Target {
    /// AMD64 using the System V ABI and ELF object format.
    Amd64SysV,
    /// AMD64 using the Apple ABI and Mach-O object format.
    Amd64Apple,
    /// AArch64 using the ELF object format.
    Aarch64Elf,
    /// AArch64 using the Apple ABI and Mach-O object format.
    Aarch64Apple,
}
