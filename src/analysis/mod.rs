//! Derived compiler analyses.

pub(crate) mod alias;
pub(crate) mod control_flow;
pub(crate) mod liveness;
mod state;
pub(crate) mod use_def;

pub(crate) use state::{AnalysisState, Mutation};
