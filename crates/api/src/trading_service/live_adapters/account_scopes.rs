use super::*;
use crate::services::trading_credentials::credential_fingerprint_from_parts;
use shared_types::FeeProduct;
use std::collections::HashMap;

pub(in crate::trading_service) fn from_credentials(
    credentials: &AdapterCredentials,
) -> HashMap<(String, FeeProduct), String> {
    let mut scopes = HashMap::new();
    let mut add =
        |venue: &str, public: &str, secret: &str, extra: Option<&str>, products: &[FeeProduct]| {
            let fingerprint = credential_fingerprint_from_parts(venue, public, secret, extra);
            for product in products {
                scopes.insert(
                    (venue.to_owned(), *product),
                    format!("mainnet:{fingerprint}"),
                );
            }
        };
    let products = [FeeProduct::Spot, FeeProduct::Perp, FeeProduct::Unknown];
    for (venue, value) in [
        ("binance", credentials.binance_live.as_ref()),
        ("bybit", credentials.bybit_live.as_ref()),
        ("gate", credentials.gate_live.as_ref()),
        ("gate_crossex", credentials.gate_crossex_live.as_ref()),
    ] {
        if let Some((key, secret)) = value {
            add(venue, key, secret, None, &products);
        }
    }
    for (venue, value) in [
        ("bitget", credentials.bitget_live.as_ref()),
        ("kucoin", credentials.kucoin_live.as_ref()),
        ("okx", credentials.okx_live.as_ref()),
    ] {
        if let Some((key, secret, passphrase)) = value {
            add(venue, key, secret, Some(passphrase), &products);
        }
    }
    if let Some(value) = &credentials.kraken_live {
        if let Some((key, secret)) = &value.spot {
            add("kraken", key, secret, None, &[FeeProduct::Spot]);
        }
        if let Some((key, secret)) = &value.futures {
            add("kraken", key, secret, None, &[FeeProduct::Perp]);
        }
    }
    if let Some(value) = &credentials.hyperliquid_live {
        for market in HYPERLIQUID_LIVE_MARKETS {
            add(
                market.venue(),
                &value.account_address,
                &value.private_key,
                value.vault_address.as_deref(),
                &products,
            );
        }
    }
    scopes
}
