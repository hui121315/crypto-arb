use super::*;
use crate::panels::modules::opportunity_format::missing_quote_label;

#[test]
fn formats_reference_price_for_order_input() {
    for value in [101.234, 1.23456, 0.123456789, 0.00000000123456789] {
        assert_eq!(format_price(value).parse::<f64>().unwrap(), value);
    }
    assert_eq!(format_price(0.0), missing_quote_label());
    assert_eq!(format_price(f64::NAN), missing_quote_label());
}

#[test]
fn notional_tracks_capital_and_leverage() {
    assert_eq!(notional_text("750", "2"), "1500");
    assert_eq!(notional_text("750", "0"), "750");
    assert_eq!(notional_text("10.25", "2"), "20.5");
    assert_eq!(notional_text("0.25", "2"), "0.5");
    assert_eq!(notional_text("12.5", "1.5"), "18.75");
}

#[test]
fn draft_restore_requires_same_opportunity_key() {
    assert!(can_restore_draft(Some("quote-key-1"), "quote-key-1"));
    assert!(!can_restore_draft(Some("quote-key-1"), "quote-key-2"));
    assert!(!can_restore_draft(None, "quote-key-1"));
}

#[test]
fn empty_selection_uses_stable_storage_key() {
    assert_eq!(
        execution_selection_key(&ExecutionSelection::empty()),
        "empty"
    );
}

#[test]
fn draft_restore_key_changes_with_market_snapshot_and_quote() {
    let mut selection = ExecutionSelection::empty();
    selection.opportunity_id = "opp-1".into();
    selection.opportunity_snapshot_id = "snapshot-1".into();
    selection.long_leg_label = "gate long".into();
    selection.short_leg_label = "kucoin short".into();
    selection.long_price_label = "0.12760000".into();
    selection.short_price_label = "0.12766000".into();
    let first = execution_selection_key(&selection);

    assert_eq!(first, execution_selection_key(&selection));
    selection.opportunity_snapshot_id = "snapshot-2".into();
    assert_ne!(first, execution_selection_key(&selection));

    let second = execution_selection_key(&selection);
    selection.long_price_label = "0.12900000".into();
    assert_ne!(second, execution_selection_key(&selection));
}

#[test]
fn margin_key_trims_input_values() {
    assert_eq!(margin_key(" 750 ", " 2 "), "750:2");
}

#[test]
fn in_memory_draft_survives_only_a_snapshot_change() {
    let mut selection = ExecutionSelection::empty();
    selection.opportunity_id = "opp-1".into();
    selection.opportunity_snapshot_id = "snapshot-1".into();
    selection.long_leg_label = "gate long".into();
    selection.short_leg_label = "kucoin short".into();
    selection.long_price_label = "100".into();
    let market_key = execution_selection_market_key(&selection);
    let storage_key = execution_selection_key(&selection);
    selection.opportunity_snapshot_id = "snapshot-2".into();
    assert_eq!(market_key, execution_selection_market_key(&selection));
    assert_ne!(storage_key, execution_selection_key(&selection));
    for changed in [
        ExecutionSelection { opportunity_id: "opp-2".into(), ..selection.clone() },
        ExecutionSelection { long_leg_label: "kucoin long".into(), ..selection.clone() },
        ExecutionSelection { long_price_label: "101".into(), ..selection.clone() },
    ] {
        assert_ne!(market_key, execution_selection_market_key(&changed));
    }
}
