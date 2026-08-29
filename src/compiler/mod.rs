//! Compilation session and compiler-owned support code.

mod session;

pub use session::{CompileError, compile, compile_module};
