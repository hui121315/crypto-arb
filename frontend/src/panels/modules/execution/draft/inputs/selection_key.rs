use super::ExecutionSelection;

pub(super) fn execution_selection_key(selection: &ExecutionSelection) -> String {
    let opportunity_id = selection.opportunity_id.trim();
    if opportunity_id.is_empty() {
        return "empty".to_owned();
    }
    let long_market = selection
        .long_market_evidence
        .as_ref()
        .map(|evidence| format!("{}:{}", evidence.venue.trim(), evidence.symbol.trim()))
        .unwrap_or_else(|| selection.long_leg_label.trim().to_owned());
    let short_market = selection
        .short_market_evidence
        .as_ref()
        .map(|evidence| format!("{}:{}", evidence.venue.trim(), evidence.symbol.trim()))
        .unwrap_or_else(|| selection.short_leg_label.trim().to_owned());
    [
        "v2",
        opportunity_id,
        selection.opportunity_snapshot_id.trim(),
        &long_market,
        selection.long_price_label.trim(),
        &short_market,
        selection.short_price_label.trim(),
    ]
    .into_iter()
    .map(selection_key_part)
    .collect::<Vec<_>>()
    .join("|")
}

fn selection_key_part(value: &str) -> String {
    format!("{}:{value}", value.len())
}
