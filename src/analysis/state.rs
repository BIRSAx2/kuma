//! Session-owned lifecycle for derived compiler analyses.

use crate::ir::internal::{Fn, Target};

use super::{alias, control_flow, liveness, use_def};

/// The shape of an IR mutation determines which derived results become stale.
#[derive(Copy, Clone)]
pub(crate) enum Mutation {
    Instructions,
    ControlFlow,
}

/// Validity state for analyses attached to the function currently being
/// compiled. Analysis storage is private compiler data; this owner makes the
/// rebuild and invalidation order explicit at the session seam.
#[derive(Default)]
pub(crate) struct AnalysisState {
    control_flow: bool,
    predecessors: bool,
    uses: bool,
    aliases: bool,
    liveness: bool,
    loops: bool,
    spill_costs: bool,
}

impl AnalysisState {
    pub(crate) fn rebuild_control_flow(&mut self, function: &mut Fn) {
        control_flow::fillrpo(function);
        self.control_flow = true;
        self.predecessors = false;
        self.liveness = false;
        self.loops = false;
        self.spill_costs = false;
    }

    pub(crate) fn rebuild_predecessors(&mut self, function: &mut Fn) {
        control_flow::fillpreds(function);
        self.predecessors = true;
    }

    pub(crate) fn rebuild_uses(&mut self, function: &mut Fn) {
        use_def::filluse(function);
        self.uses = true;
        self.aliases = false;
        self.liveness = false;
        self.spill_costs = false;
    }

    pub(crate) fn rebuild_aliases(&mut self, function: &mut Fn) {
        debug_assert!(self.uses, "alias analysis requires current use-def data");
        alias::fillalias(function);
        self.aliases = true;
    }

    pub(crate) fn rebuild_liveness(&mut self, function: &mut Fn, target: &Target) {
        debug_assert!(self.control_flow, "liveness requires current control flow");
        liveness::filllive(function, target);
        self.liveness = true;
        self.spill_costs = false;
    }

    pub(crate) fn rebuild_loops(&mut self, function: &mut Fn) {
        debug_assert!(
            self.control_flow,
            "loop analysis requires current control flow"
        );
        control_flow::fillloop(function);
        self.loops = true;
        self.spill_costs = false;
    }

    pub(crate) fn mark_spill_costs(&mut self) {
        debug_assert!(self.liveness && self.loops);
        self.spill_costs = true;
    }

    pub(crate) fn require_aliases(&self) {
        debug_assert!(self.aliases, "transform requires current alias analysis");
    }

    pub(crate) fn invalidate(&mut self, mutation: Mutation) {
        self.uses = false;
        self.aliases = false;
        self.liveness = false;
        self.spill_costs = false;
        if matches!(mutation, Mutation::ControlFlow) {
            self.control_flow = false;
            self.predecessors = false;
            self.loops = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_state() -> AnalysisState {
        AnalysisState {
            control_flow: true,
            predecessors: true,
            uses: true,
            aliases: true,
            liveness: true,
            loops: true,
            spill_costs: true,
        }
    }

    #[test]
    fn instruction_mutation_preserves_graph_analyses() {
        let mut state = valid_state();
        state.invalidate(Mutation::Instructions);
        assert!(state.control_flow && state.predecessors && state.loops);
        assert!(!state.uses && !state.aliases && !state.liveness && !state.spill_costs);
    }

    #[test]
    fn control_flow_mutation_invalidates_all_derived_results() {
        let mut state = valid_state();
        state.invalidate(Mutation::ControlFlow);
        assert!(!state.control_flow && !state.predecessors && !state.loops);
        assert!(!state.uses && !state.aliases && !state.liveness && !state.spill_costs);
    }
}
