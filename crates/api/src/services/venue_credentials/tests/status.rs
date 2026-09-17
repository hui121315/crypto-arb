use super::*;

#[test]
fn lists_all_execution_venues() {
    let status = status();

    assert_eq!(
        status
            .venues
            .iter()
            .map(|venue| venue.venue.as_str())
            .collect::<Vec<_>>(),
        vec![
            "binance",
            "okx",
            "bybit",
            "bitget",
            "gate",
            "gate_crossex",
            "kucoin",
            "hyperliquid",
            "kraken",
        ]
    );
    let hyperliquid = status
        .venues
        .iter()
        .find(|venue| venue.venue == "hyperliquid");
    assert!(hyperliquid.is_some_and(|venue| {
        venue
            .fields
            .iter()
            .any(|field| field.key == "account_address")
            && venue.fields.iter().any(|field| {
                field.key == "vault_address"
                    && field.env_key == "HYPERLIQUID_VAULT_ADDRESS"
                    && !field.secret
                    && !field.required
            })
            && !venue
                .missing_fields
                .iter()
                .any(|field| field == "vault_address")
    }));
}
