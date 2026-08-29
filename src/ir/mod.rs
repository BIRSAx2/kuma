//! Read-only semantic intermediate representation.

pub(crate) mod builder;
pub(crate) mod internal;
mod semantic;

pub use semantic::{
    BasicBlock, BasicBlockId, Constant, DataDefinition, DataItem, Function, FunctionId,
    Instruction, Linkage, Module, Phi, PhiInput, Terminator, TypeDefinition, TypeId, TypeMember,
    TypeVariant, Value, ValueClass, ValueId,
};
