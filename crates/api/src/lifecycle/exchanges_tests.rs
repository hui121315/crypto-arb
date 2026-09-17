use super::*;

#[test]
fn builds_hyperliquid_builder_adapter_aliases() -> Result<(), String> {
    let adapter = build_adapter(" Hyperliquid:XYZ ")?;

    assert_eq!(adapter.name(), "hyperliquid:xyz");
    assert_eq!(adapter.normalize_symbol("xyz:CBRS"), "CBRS");
    assert_eq!(adapter.to_exchange_symbol("CBRS"), "xyz:CBRS");
    Ok(())
}

#[test]
fn refresh_rejects_unknown_exchange_without_string_fanout() -> anyhow::Result<()> {
    let state = test_state()?;
    let result = refresh(&state, "unknown");
    assert!(result.is_err());
    Ok(())
}

fn test_state() -> anyhow::Result<AppState> {
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(AppState::new(common::config::AppConfig::default()))
}
