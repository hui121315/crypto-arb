use rust_decimal::{prelude::FromPrimitive, Decimal};
use shared_types::{OnchainComparisonConfig, OnchainUsdValuation};

pub(crate) struct Valuation {
    pub evidence: OnchainUsdValuation,
    pub risk_bps: u16,
}

impl Valuation {
    pub(crate) fn read(
        source: &OnchainComparisonConfig,
        evidence: Option<&OnchainUsdValuation>,
        max_age_ms: i64,
        now_ms: i64,
    ) -> Result<Self, String> {
        super::super::usd_valuation::rate(evidence, &source.quote_token, max_age_ms, now_ms)
            .ok_or_else(|| {
                format!(
                    "{}/USD 汇率缺失或已过期，不能将跨链 Gas 换算为报价币；不会默认按 1 美元计价",
                    source.quote_token
                )
            })?;
        let risk_bps = if source.quote_token.eq_ignore_ascii_case("USD") {
            0
        } else {
            source.cross_chain.stablecoin_risk_bps
        };
        if risk_bps >= 10_000 {
            return Err("报价币汇率风险缓冲必须低于 100%".to_owned());
        }
        Ok(Self {
            evidence: evidence.expect("validated valuation").clone(),
            risk_bps,
        })
    }

    pub(crate) fn gas_raw(&self, usd: f64, decimals: u8) -> Result<u128, String> {
        if !usd.is_finite() || usd < 0.0 {
            return Err("Gas 美元成本必须是有效非负数".to_owned());
        }
        let gas = Decimal::from_f64(usd).ok_or("Gas 成本精度超出支持范围")?;
        let bid = Decimal::from_f64(self.evidence.usd_bid)
            .filter(|bid| *bid > Decimal::ZERO)
            .ok_or("报价币美元汇率精度超出支持范围")?;
        if usd > 0.0 && gas == Decimal::ZERO {
            return Err("Gas 成本低于支持精度，不能按零计费".to_owned());
        }
        let scale = 10_u128
            .checked_pow(u32::from(decimals))
            .ok_or("报价币精度超出支持范围")?;
        // Cancel decimal factors before multiplication, then round the exact
        // ratio upward. f64 raw-unit rounding loses wei at 18 decimals.
        let mut numerators = [
            gas.mantissa().unsigned_abs(),
            scale,
            10_u128.pow(bid.scale()),
            10_000,
        ];
        let mut denominators = [
            10_u128.pow(gas.scale()),
            bid.mantissa().unsigned_abs(),
            u128::from(10_000 - self.risk_bps),
        ];
        for numerator in &mut numerators {
            for denominator in &mut denominators {
                let (mut a, mut b) = (*numerator, *denominator);
                while b != 0 {
                    (a, b) = (b, a % b);
                }
                *numerator /= a;
                *denominator /= a;
            }
        }
        let numerator = numerators
            .into_iter()
            .try_fold(1_u128, u128::checked_mul)
            .ok_or("Gas 换算分子溢出")?;
        let denominator = denominators
            .into_iter()
            .try_fold(1_u128, u128::checked_mul)
            .ok_or("Gas 换算分母溢出")?;
        (numerator / denominator)
            .checked_add(u128::from(numerator % denominator != 0))
            .ok_or_else(|| "Gas 换算结果溢出".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::onchain_comparison::usd_valuation;

    #[test]
    fn gas_uses_actual_rate_and_exact_atomic_rounding() {
        let mut config = OnchainComparisonConfig::default();
        config.quote_token = "USDC".to_owned();
        config.cross_chain.stablecoin_risk_bps = 50;
        let evidence = usd_valuation::fixture("USDC", 0.8, 1_000);
        let valuation = Valuation::read(&config, Some(&evidence), 1_000, 1_100).unwrap();
        assert_eq!(valuation.gas_raw(1.0, 6).unwrap(), 1_256_282);
        assert_eq!(valuation.gas_raw(0.0, 6).unwrap(), 0);
        assert!(valuation.gas_raw(f64::NAN, 6).is_err());
        assert!(valuation.gas_raw(-1.0, 6).is_err());
        assert!(valuation.gas_raw(1.0, 255).is_err());

        config.quote_token = "WETH".to_owned();
        config.cross_chain.stablecoin_risk_bps = 0;
        let evidence = usd_valuation::fixture("WETH", 1.2, 1_000);
        let valuation = Valuation::read(&config, Some(&evidence), 1_000, 1_100).unwrap();
        assert_eq!(valuation.gas_raw(1.0, 18).unwrap(), 833_333_333_333_333_334);
    }

    #[test]
    fn stablecoin_name_never_replaces_missing_or_stale_valuation() {
        let mut config = OnchainComparisonConfig::default();
        config.quote_token = "USDC".to_owned();
        let mut evidence = usd_valuation::fixture("USDC", 0.9, 1_000);
        assert!(Valuation::read(&config, None, 100, 1_100).is_err());
        assert!(Valuation::read(&config, Some(&evidence), 100, 1_101).is_err());
        assert!(Valuation::read(&config, Some(&evidence), 100, 999).is_err());
        evidence.asset = "USDT".to_owned();
        assert!(Valuation::read(&config, Some(&evidence), 100, 1_100).is_err());
        evidence.asset = "USDC".to_owned();
        config.cross_chain.stablecoin_risk_bps = 10_000;
        assert!(Valuation::read(&config, Some(&evidence), 100, 1_100).is_err());
        config.quote_token = "USD".to_owned();
        evidence.asset = "USD".to_owned();
        evidence.source = "same_currency".to_owned();
        evidence.usd_bid = 1.0;
        evidence.usd_ask = 1.0;
        let valuation = Valuation::read(&config, Some(&evidence), 100, 1_100).unwrap();
        assert_eq!(valuation.risk_bps, 0);
        assert_eq!(valuation.gas_raw(1.0, 6).unwrap(), 1_000_000);
    }
}
