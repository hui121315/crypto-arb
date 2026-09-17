use super::*;
use pretty_assertions::assert_eq;
use rust_decimal::Decimal;
use shared_types::SpotTick;

fn tick(symbol: &str) -> SpotTick {
    SpotTick {
        venue: "binance".into(),
        symbol: symbol.into(),
        bid: Decimal::new(100, 0),
        ask: Decimal::new(101, 0),
        last: Decimal::new(100, 0),
        bid_size: None,
        ask_size: None,
        volume_24h: Decimal::ZERO,
        exchange_ts_ms: None,
        received_at_ms: 1,
    }
}

fn req(symbols: &[&str]) -> Vec<String> {
    symbols.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn listed_symbol_present_in_ticks_is_listed() {
    let ticks = vec![tick("BTC/USDT"), tick("ETH/USDT")];
    let plan = plan_symbol_coverage("binance", &req(&["BTC/USDT"]), VenueListing::Listed(&ticks));
    assert_eq!(plan.venue, "binance");
    assert_eq!(plan.entries.len(), 1);
    assert_eq!(plan.entries[0].status, SymbolCoverageStatus::Listed);
}

#[test]
fn listed_symbol_matches_across_delimiter_and_compact_forms() {
    let ticks = vec![tick("BTC/USDT")];
    for form in ["BTC-USDT", "BTC_USDT", "BTCUSDT", "btc/usdt"] {
        let plan = plan_symbol_coverage("binance", &req(&[form]), VenueListing::Listed(&ticks));
        assert_eq!(
            plan.entries[0].status,
            SymbolCoverageStatus::Listed,
            "form {form} should be listed"
        );
    }
}

#[test]
fn requested_but_absent_symbol_is_unlisted_not_dropped() {
    let ticks = vec![tick("BTC/USDT")];
    let plan = plan_symbol_coverage(
        "binance",
        &req(&["DOGE/USDT"]),
        VenueListing::Listed(&ticks),
    );
    assert_eq!(plan.entries.len(), 1);
    assert_eq!(plan.entries[0].status, SymbolCoverageStatus::Unlisted);
}

#[test]
fn failed_fetch_marks_resolvable_symbols_failed() {
    let plan = plan_symbol_coverage(
        "binance",
        &req(&["BTC/USDT", "ETH/USDT"]),
        VenueListing::Failed,
    );
    assert_eq!(plan.entries.len(), 2);
    assert_eq!(plan.count(SymbolCoverageStatus::Failed), 2);
}

#[test]
fn unparseable_symbol_is_unsupported_even_when_fetch_failed() {
    // No recognizable quote asset -> adapter cannot build a market; retrying
    // the fetch would not help, so it stays Unsupported rather than Failed.
    let plan = plan_symbol_coverage("binance", &req(&["FOOBAR", ""]), VenueListing::Failed);
    assert_eq!(plan.count(SymbolCoverageStatus::Unsupported), 2);
}

#[test]
fn every_requested_symbol_is_classified_exactly_once() {
    let ticks = vec![tick("BTC/USDT")];
    let requested = req(&["BTC/USDT", "DOGE/USDT", "FOOBAR"]);
    let plan = plan_symbol_coverage("binance", &requested, VenueListing::Listed(&ticks));
    assert_eq!(plan.entries.len(), requested.len());
    assert_eq!(plan.count(SymbolCoverageStatus::Listed), 1);
    assert_eq!(plan.count(SymbolCoverageStatus::Unlisted), 1);
    assert_eq!(plan.count(SymbolCoverageStatus::Unsupported), 1);
    let echoed: Vec<&str> = plan.entries.iter().map(|e| e.symbol.as_str()).collect();
    assert_eq!(echoed, vec!["BTC/USDT", "DOGE/USDT", "FOOBAR"]);
}
