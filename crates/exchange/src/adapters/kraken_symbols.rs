//! Kraken native symbol conversion with the official XBT/BTC alias boundary.

const QUOTES: &[&str] = &["USDT", "USDC", "USD", "EUR", "GBP", "CAD", "AUD", "JPY"];

pub(super) fn canonical_asset(asset: &str) -> String {
    match asset.trim().to_ascii_uppercase().as_str() {
        "XBT" | "XXBT" => "BTC".to_owned(),
        "XDG" | "XXDG" => "DOGE".to_owned(),
        "XETH" => "ETH".to_owned(),
        "ZUSD" => "USD".to_owned(),
        "ZEUR" => "EUR".to_owned(),
        "ZGBP" => "GBP".to_owned(),
        "ZJPY" => "JPY".to_owned(),
        "ZCAD" => "CAD".to_owned(),
        "ZAUD" => "AUD".to_owned(),
        value => value.to_owned(),
    }
}

pub(super) fn funding_asset(asset: &str) -> String {
    match canonical_asset(asset).as_str() {
        "BTC" => "XBT".to_owned(),
        "DOGE" => "XDG".to_owned(),
        value => value.to_owned(),
    }
}

pub(super) fn canonical_symbol(symbol: &str) -> String {
    split_pair(symbol)
        .map(|(base, _)| canonical_asset(&base))
        .unwrap_or_else(|| canonical_asset(symbol))
}

pub(super) fn spot_symbol(symbol: &str) -> String {
    let (base, quote) =
        split_pair(symbol).unwrap_or_else(|| (canonical_asset(symbol), "USD".to_owned()));
    format!("{}/{}", canonical_asset(&base), canonical_asset(&quote))
}

pub(super) fn futures_symbol(symbol: &str) -> String {
    let upper = symbol.trim().to_ascii_uppercase();
    if upper.starts_with("PF_") || upper.starts_with("PI_") {
        return upper;
    }
    let (base, quote) =
        split_pair(&upper).unwrap_or_else(|| (canonical_asset(&upper), "USD".to_owned()));
    let native_base = if canonical_asset(&base) == "BTC" {
        "XBT".to_owned()
    } else {
        canonical_asset(&base)
    };
    format!("PF_{native_base}{}", canonical_asset(&quote))
}

pub(super) fn split_pair(symbol: &str) -> Option<(String, String)> {
    let upper = symbol.trim().to_ascii_uppercase();
    let body = upper
        .strip_prefix("PF_")
        .or_else(|| upper.strip_prefix("PI_"))
        .unwrap_or(&upper);
    if let Some((base, quote)) = body
        .split_once('/')
        .or_else(|| body.split_once(':'))
        .or_else(|| body.split_once('-'))
        .or_else(|| body.split_once('_'))
    {
        return (!base.is_empty() && !quote.is_empty())
            .then(|| (canonical_asset(base), canonical_asset(quote)));
    }
    QUOTES.iter().find_map(|quote| {
        body.strip_suffix(quote)
            .filter(|base| !base.is_empty())
            .map(|base| (canonical_asset(base), canonical_asset(quote)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_spot_v2_btc_and_futures_xbt_native_forms_distinct() {
        assert_eq!(spot_symbol("XBTUSD"), "BTC/USD");
        assert_eq!(spot_symbol("XDG/USD"), "DOGE/USD");
        assert_eq!(futures_symbol("BTC/USD"), "PF_XBTUSD");
        assert_eq!(canonical_symbol("PF_XBTUSD"), "BTC");
        assert_eq!(funding_asset("BTC"), "XBT");
        assert_eq!(funding_asset("XXDG"), "XDG");
    }
}
