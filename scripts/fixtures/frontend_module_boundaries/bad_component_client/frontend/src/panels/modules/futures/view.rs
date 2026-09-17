fn build(opp: Opportunity, execution_runtime: ExecutionRuntime) {
    execution_runtime.seed_selection(opp.execution_seed());
    use_global().client.load();
}
