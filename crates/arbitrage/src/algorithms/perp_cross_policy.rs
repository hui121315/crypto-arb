//! Executable-price alignment policy for pure cross-venue perpetual funding.

const PURE_FUNDING_MAX_GAP_BPS: f64 = 10.0;
const HARD_SANITY_MAX_GAP_BPS: f64 = 30.0;
const MAX_PRICE_TIMESTAMP_SKEW_MS: u64 = 2_000;
const MAX_OPENING_GAP_SHARE_OF_NET: f64 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PriceAlignmentClass {
    Aligned,
    HybridSpread,
    Invalid,
    Unproven,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PriceAlignmentInput {
    pub(crate) long_open_price: Option<f64>,
    pub(crate) short_open_price: Option<f64>,
    pub(crate) long_observed_at_ms: i64,
    pub(crate) short_observed_at_ms: i64,
    pub(crate) projected_net_funding_bps: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PriceAlignment {
    pub(crate) class: PriceAlignmentClass,
    pub(crate) passed: bool,
    pub(crate) signed_gap_bps: Option<f64>,
    pub(crate) absolute_gap_bps: Option<f64>,
    pub(crate) detail: String,
}

pub(crate) fn evaluate(input: PriceAlignmentInput) -> PriceAlignment {
    let Some((long, short)) = valid_prices(input.long_open_price, input.short_open_price) else {
        return result(
            PriceAlignmentClass::Unproven,
            false,
            None,
            "缺少双边可成交开仓价，不能核验永续跨所价格同轨".to_owned(),
        );
    };
    if !valid_timestamps(input.long_observed_at_ms, input.short_observed_at_ms) {
        return result(
            PriceAlignmentClass::Unproven,
            false,
            None,
            "双边价格缺少同窗时间证据，不能核验永续跨所价格同轨".to_owned(),
        );
    }
    let skew_ms = input
        .long_observed_at_ms
        .abs_diff(input.short_observed_at_ms);
    if skew_ms > MAX_PRICE_TIMESTAMP_SKEW_MS {
        return result(
            PriceAlignmentClass::Unproven,
            false,
            None,
            format!("双边可成交价时间差 {skew_ms}ms 超过 2000ms，同轨比较失效"),
        );
    }

    let reference = (long + short) * 0.5;
    let signed_gap_bps = (short - long) / reference * 10_000.0;
    let absolute_gap_bps = signed_gap_bps.abs();
    if absolute_gap_bps > HARD_SANITY_MAX_GAP_BPS {
        return result(
            PriceAlignmentClass::Invalid,
            false,
            Some(signed_gap_bps),
            format!(
                "双边可成交价格差 {:.4}% 超过 0.30%，可能是行情、合约身份或价格单位异常",
                absolute_gap_bps / 100.0
            ),
        );
    }
    if absolute_gap_bps > PURE_FUNDING_MAX_GAP_BPS {
        return result(
            PriceAlignmentClass::HybridSpread,
            false,
            Some(signed_gap_bps),
            format!(
                "双边可成交价格差 {:.4}% 超过纯资金费套利 0.10% 门槛，应转入永续价差观察",
                absolute_gap_bps / 100.0
            ),
        );
    }

    let Some(projected_net_bps) = input
        .projected_net_funding_bps
        .filter(|value| value.is_finite() && *value > 0.0)
    else {
        return result(
            PriceAlignmentClass::Unproven,
            false,
            Some(signed_gap_bps),
            "缺少扣除全成本后的原生结算净收益，不能计算不利价差预算".to_owned(),
        );
    };
    let opening_gap_limit_bps = projected_net_bps * MAX_OPENING_GAP_SHARE_OF_NET;
    if absolute_gap_bps > opening_gap_limit_bps + f64::EPSILON {
        return result(
            PriceAlignmentClass::HybridSpread,
            false,
            Some(signed_gap_bps),
            format!(
                "双边开仓价格差 {:.4}bps 超过净资金费预算的 50%（{:.4}bps），应转入永续价差观察",
                absolute_gap_bps, opening_gap_limit_bps
            ),
        );
    }

    result(
        PriceAlignmentClass::Aligned,
        true,
        Some(signed_gap_bps),
        format!(
            "双边可成交价格同轨：价差 {:.4}bps，时间差 {skew_ms}ms，价格偏离预算 {:.4}bps",
            signed_gap_bps, opening_gap_limit_bps
        ),
    )
}

fn valid_prices(long: Option<f64>, short: Option<f64>) -> Option<(f64, f64)> {
    let long = long.filter(|value| value.is_finite() && *value > f64::EPSILON)?;
    let short = short.filter(|value| value.is_finite() && *value > f64::EPSILON)?;
    Some((long, short))
}

fn valid_timestamps(long_ms: i64, short_ms: i64) -> bool {
    long_ms > 0 && short_ms > 0
}

fn result(
    class: PriceAlignmentClass,
    passed: bool,
    signed_gap_bps: Option<f64>,
    detail: String,
) -> PriceAlignment {
    PriceAlignment {
        class,
        passed,
        signed_gap_bps,
        absolute_gap_bps: signed_gap_bps.map(f64::abs),
        detail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_prices_pass_the_pure_funding_gate() {
        let proof = evaluate(input(100.0, 100.05, 20.0));

        assert!(proof.passed);
        assert_eq!(proof.class, PriceAlignmentClass::Aligned);
    }

    #[test]
    fn medium_gap_moves_to_spread_observation() {
        let proof = evaluate(input(100.0, 100.2, 20.0));

        assert!(!proof.passed);
        assert_eq!(proof.class, PriceAlignmentClass::HybridSpread);
    }

    #[test]
    fn large_gap_is_a_hard_sanity_failure() {
        let proof = evaluate(input(100.0, 100.4, 20.0));

        assert!(!proof.passed);
        assert_eq!(proof.class, PriceAlignmentClass::Invalid);
    }

    #[test]
    fn opening_gap_in_either_direction_must_fit_inside_half_the_projected_net() {
        let adverse = evaluate(input(100.04, 100.0, 6.0));
        let favorable = evaluate(input(100.0, 100.04, 6.0));
        let allowed = evaluate(input(100.0, 100.02, 6.0));

        assert!(!adverse.passed);
        assert_eq!(adverse.class, PriceAlignmentClass::HybridSpread);
        assert!(!favorable.passed);
        assert_eq!(favorable.class, PriceAlignmentClass::HybridSpread);
        assert!(allowed.passed);
    }

    #[test]
    fn asynchronous_prices_cannot_prove_alignment() {
        let mut value = input(100.0, 100.05, 20.0);
        value.short_observed_at_ms += 2_001;

        let proof = evaluate(value);

        assert!(!proof.passed);
        assert_eq!(proof.class, PriceAlignmentClass::Unproven);
    }

    fn input(long: f64, short: f64, net_bps: f64) -> PriceAlignmentInput {
        PriceAlignmentInput {
            long_open_price: Some(long),
            short_open_price: Some(short),
            long_observed_at_ms: 1_000_000,
            short_observed_at_ms: 1_000_010,
            projected_net_funding_bps: Some(net_bps),
        }
    }
}
