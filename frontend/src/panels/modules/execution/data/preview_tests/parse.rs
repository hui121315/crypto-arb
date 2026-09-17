use super::*;

#[test]
fn parse_number_keeps_scientific_notation() {
    assert_eq!(parse_number("1e3"), Some(1_000.0));
    assert_eq!(parse_number("$1,250.5"), Some(1_250.5));
    assert_eq!(parse_number(missing_quote_label()), None);
}

#[test]
fn quantity_from_notional_requires_positive_notional_evidence() {
    assert_eq!(
        quantity_from_notional_text("BTC-PERP", "25000", "1000"),
        "0.0400 BTC-PERP"
    );
    assert_eq!(
        quantity_from_notional_text("BTC-PERP", missing_quote_label(), "750"),
        "$750 名义"
    );
    for missing in ["", "bad", "0", "-5"] {
        let label = quantity_from_notional_text("BTC-PERP", "25000", missing);
        assert_eq!(label, "名义缺证据");
        assert!(!label.contains("0.0000"));
        assert!(!label.contains("$0"));
    }
}
