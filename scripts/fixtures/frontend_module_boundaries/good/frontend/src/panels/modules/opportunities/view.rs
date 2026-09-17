fn build(row: Row, execution_runtime: ExecutionRuntime) {
    execution_runtime.seed_selection(ExecutionSelectionSeed::from_opportunities(row.as_ref()));
}
