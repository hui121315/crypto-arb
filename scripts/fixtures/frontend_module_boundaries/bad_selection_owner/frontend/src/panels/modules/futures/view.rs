fn build(opp: Opportunity, execution_runtime: ExecutionRuntime) {
    execution_runtime.seed_selection(opp.execution_seed());
    let duplicate = RwSignal::new(ExecutionSelection::empty());
}
