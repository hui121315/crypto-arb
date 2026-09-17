//! Gate `CrossEx` route identity parsing.

use crate::error::{ExchangeError, ExchangeResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CrossExBusiness {
    Spot,
    Future,
    Margin,
}

impl CrossExBusiness {
    pub(super) const fn as_native(self) -> &'static str {
        match self {
            Self::Spot => "SPOT",
            Self::Future => "FUTURE",
            Self::Margin => "MARGIN",
        }
    }

    pub(super) const fn product_type(self) -> &'static str {
        match self {
            Self::Spot => "spot",
            Self::Future => "perp",
            Self::Margin => "margin",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CrossExRoute {
    pub(super) native_symbol: String,
    pub(super) underlying_venue: String,
    pub(super) business: CrossExBusiness,
    pub(super) base: String,
    pub(super) quote: String,
}

impl CrossExRoute {
    pub(super) fn parse(value: &str) -> ExchangeResult<Self> {
        let value = value.trim().to_ascii_uppercase();
        let parts = value.split('_').collect::<Vec<_>>();
        if parts.len() != 4 || parts.iter().any(|part| part.is_empty()) {
            return Err(ExchangeError::Parse(format!(
                "invalid Gate CrossEx route {value:?}; expected EXCHANGE_BUSINESS_BASE_QUOTE"
            )));
        }
        let business = match parts[1] {
            "SPOT" => CrossExBusiness::Spot,
            "FUTURE" => CrossExBusiness::Future,
            "MARGIN" => CrossExBusiness::Margin,
            other => {
                return Err(ExchangeError::Parse(format!(
                    "unsupported Gate CrossEx business type {other:?}"
                )))
            }
        };
        let underlying_venue = parts[0].to_owned();
        let base = parts[2].to_owned();
        let quote = parts[3].to_owned();
        Ok(Self {
            native_symbol: value,
            underlying_venue,
            business,
            base,
            quote,
        })
    }

    pub(super) fn venue(&self) -> String {
        format!(
            "gate_crossex:{}",
            self.underlying_venue.to_ascii_lowercase()
        )
    }

    pub(super) fn display_symbol(&self) -> String {
        let product = match self.business {
            CrossExBusiness::Spot => "Spot",
            CrossExBusiness::Future => "Perp",
            CrossExBusiness::Margin => "Margin",
        };
        format!(
            "{}/{} {product} via CrossEx {}",
            self.base, self.quote, self.underlying_venue
        )
    }
}

pub(super) fn route_from_scoped_symbol(
    symbol: &str,
    business: CrossExBusiness,
    default_underlying: &str,
) -> ExchangeResult<CrossExRoute> {
    if symbol.matches('_').count() == 3 {
        return CrossExRoute::parse(symbol);
    }
    let (underlying, base) = symbol
        .trim()
        .split_once(':')
        .map_or((default_underlying, symbol.trim()), |(underlying, base)| {
            (underlying, base)
        });
    CrossExRoute::parse(&format!(
        "{}_{}_{}_USDT",
        underlying,
        business.as_native(),
        base
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_preserves_underlying_venue_identity() {
        let route = CrossExRoute::parse("KRAKEN_FUTURE_BTC_USD").unwrap();
        assert_eq!(route.venue(), "gate_crossex:kraken");
        assert_eq!(route.base, "BTC");
        assert_eq!(route.quote, "USD");
        assert_eq!(route.business, CrossExBusiness::Future);
    }

    #[test]
    fn scoped_symbol_compiles_without_guessing_the_underlying() {
        let route = route_from_scoped_symbol("okx:ETH", CrossExBusiness::Future, "GATE").unwrap();
        assert_eq!(route.native_symbol, "OKX_FUTURE_ETH_USDT");
    }

    #[test]
    fn malformed_route_fails_closed() {
        assert!(CrossExRoute::parse("BTCUSDT").is_err());
        assert!(CrossExRoute::parse("GATE_OPTION_BTC_USDT").is_err());
    }
}
