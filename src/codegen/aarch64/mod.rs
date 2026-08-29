//! AArch64 backend adapter.

mod abi;
mod emission;
mod registers;
mod selection;

pub(crate) use abi::{abi0, abi1, argregs, retregs};
pub(crate) use emission::emitfn;
pub(crate) use registers::{T_AARCH64_APPLE, T_AARCH64_ELF};
pub(crate) use selection::isel;
