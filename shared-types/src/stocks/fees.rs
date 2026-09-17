use super::*;
use rust_decimal::Decimal;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum StockCexFeeBasis {
    OrderBookQuote {
        taker_bps: String,
        observed_at_ms: i64,
    },
    RfqIncluded {
        quote_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockCexFeeBudget {
    pub basis: StockCexFeeBasis,
    pub asset: String,
    // Additional to the bound price, not an assertion that an RFQ has zero fees.
    pub additional_fee: String,
    pub net_quote_change: String,
}

impl StockCexFeeBudget {
    pub fn calculate(notional: &str, side: StockRfqSide, basis: StockCexFeeBasis) -> Option<Self> {
        let n = Decimal::from_str_exact(notional)
            .ok()
            .filter(|n| *n > Decimal::ZERO)?;
        let fee = match &basis {
            StockCexFeeBasis::OrderBookQuote {
                taker_bps,
                observed_at_ms,
            } => {
                if *observed_at_ms <= 0 {
                    return None;
                }
                let rate = Decimal::from_str_exact(taker_bps)
                    .ok()
                    .filter(|v| *v >= Decimal::ZERO && *v < Decimal::from(10_000))?;
                let fee = n.checked_mul(rate)?.checked_div(Decimal::from(10_000))?;
                if rate > Decimal::ZERO && fee == Decimal::ZERO {
                    return None;
                }
                fee
            }
            StockCexFeeBasis::RfqIncluded { quote_id } => {
                if quote_id.is_empty() {
                    return None;
                }
                Decimal::ZERO
            }
        };
        let net = match side {
            StockRfqSide::Bid => -n.checked_add(fee)?,
            StockRfqSide::Ask => n.checked_sub(fee)?,
        };
        Some(Self {
            basis,
            asset: "USDC".into(),
            additional_fee: fee.normalize().to_string(),
            net_quote_change: net.normalize().to_string(),
        })
    }

    pub fn required(&self, side: StockRfqSide, shares: &str) -> Option<String> {
        let n = match side {
            StockRfqSide::Bid => -Decimal::from_str_exact(&self.net_quote_change).ok()?,
            StockRfqSide::Ask => Decimal::from_str_exact(shares).ok()?,
        };
        (n > Decimal::ZERO).then(|| n.normalize().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stock_fee_budget_reserves_quote_fee_without_charging_rfq_twice() {
        let spot = |rate: &str| StockCexFeeBasis::OrderBookQuote {
            taker_bps: rate.into(),
            observed_at_ms: 1000,
        };
        let bid = StockCexFeeBudget::calculate("12.02", StockRfqSide::Bid, spot("10")).unwrap();
        assert_eq!(bid.additional_fee, "0.01202");
        assert_eq!(
            bid.required(StockRfqSide::Bid, "0.02").as_deref(),
            Some("12.03202")
        );
        let ask = StockCexFeeBudget::calculate("12", StockRfqSide::Ask, spot("10")).unwrap();
        assert_eq!(ask.net_quote_change, "11.988");
        assert_eq!(
            ask.required(StockRfqSide::Ask, "0.02").as_deref(),
            Some("0.02")
        );
        let rfq = StockCexFeeBudget::calculate(
            "12",
            StockRfqSide::Bid,
            StockCexFeeBasis::RfqIncluded {
                quote_id: "original-quote".into(),
            },
        )
        .unwrap();
        assert_eq!(rfq.net_quote_change, "-12");
        assert_eq!(rfq.additional_fee, "0");
        for rate in ["unknown", "-1", "10000"] {
            assert!(StockCexFeeBudget::calculate("12", StockRfqSide::Bid, spot(rate)).is_none());
        }
        assert!(StockCexFeeBudget::calculate(
            "0.0000000000000000000000000001",
            StockRfqSide::Bid,
            spot("0.1")
        )
        .is_none());
        assert_eq!(
            StockCexFeeBudget::calculate("12", StockRfqSide::Bid, spot("0"))
                .unwrap()
                .additional_fee,
            "0"
        );
    }
}
