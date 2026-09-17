//! 资金费率标准化与异常值检测。
//!
//! - **EWMA 平滑**：指数加权移动平均，平滑短期噪声
//! - **z-score 异常值检测**：识别偏离均值过多的异常费率

use shared_types::FundingRateData;

const MIN_OUTLIER_SAMPLES: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NormalizationEvidence {
    pub sample_count: usize,
    pub excluded_sample_count: usize,
    pub min_sample_count: usize,
    pub ewma_span: u32,
    pub missing_reason: Option<&'static str>,
}

/// EWMA 平滑器。`span` 越大，对历史的权重越高。
#[derive(Debug)]
pub struct EwmaSmoother {
    /// 权重 alpha = 2 / (span + 1)
    alpha: f64,
}

impl EwmaSmoother {
    pub fn new(span: u32) -> Self {
        let s = span.max(1) as f64;
        Self {
            alpha: 2.0 / (s + 1.0),
        }
    }

    /// 计算 EWMA。`series` 为按时间升序排列的历史费率。
    pub fn smooth(&self, series: &[f64]) -> Option<f64> {
        if series.is_empty() {
            return None;
        }
        let mut value = series[0];
        for &x in &series[1..] {
            value = self.alpha * x + (1.0 - self.alpha) * value;
        }
        Some(value)
    }
}

/// 异常值检测：基于 z-score 阈值。返回 `(mean, std, is_outlier)`。
pub fn detect_outlier(series: &[f64], current: f64, zscore_threshold: f64) -> (f64, f64, bool) {
    let clean = finite_samples(series);
    if clean.len() < MIN_OUTLIER_SAMPLES || !current.is_finite() {
        return (0.0, 0.0, false);
    }
    let n = clean.len() as f64;
    let mean: f64 = clean.iter().sum::<f64>() / n;
    let var: f64 = clean.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0);
    let std = var.sqrt();
    if std == 0.0 {
        return (mean, 0.0, false);
    }
    let z = ((current - mean) / std).abs();
    (mean, std, z > zscore_threshold)
}

/// 在原始 [`FundingRateData`] 上应用 EWMA + 异常值检测，写入 `smoothed_rate` / `rate_std` / `is_outlier`。
pub fn apply_normalization(
    data: &mut FundingRateData,
    historical: &[f64],
    span: u32,
    zscore_threshold: f64,
) {
    let _ = apply_normalization_with_evidence(data, historical, span, zscore_threshold);
}

pub fn apply_normalization_with_evidence(
    data: &mut FundingRateData,
    historical: &[f64],
    span: u32,
    zscore_threshold: f64,
) -> NormalizationEvidence {
    let clean = finite_samples(historical);
    let evidence = normalization_evidence(historical.len(), clean.len(), span);
    let smoother = EwmaSmoother::new(span);
    let smoothed = smoother.smooth(&clean);
    let (_mean, std, is_outlier) = detect_outlier(&clean, data.rate_8h, zscore_threshold);

    data.smoothed_rate = smoothed;
    data.rate_std = if clean.len() >= MIN_OUTLIER_SAMPLES {
        Some(std)
    } else {
        None
    };
    data.is_outlier = is_outlier;
    evidence
}

fn normalization_evidence(
    original_count: usize,
    sample_count: usize,
    span: u32,
) -> NormalizationEvidence {
    let excluded_sample_count = original_count.saturating_sub(sample_count);
    NormalizationEvidence {
        sample_count,
        excluded_sample_count,
        min_sample_count: MIN_OUTLIER_SAMPLES,
        ewma_span: span.max(1),
        missing_reason: normalization_missing_reason(sample_count, excluded_sample_count),
    }
}

fn normalization_missing_reason(
    sample_count: usize,
    excluded_sample_count: usize,
) -> Option<&'static str> {
    if sample_count == 0 {
        Some("empty_history")
    } else if sample_count < MIN_OUTLIER_SAMPLES {
        Some("insufficient_samples")
    } else if excluded_sample_count > 0 {
        Some("excluded_non_finite_samples")
    } else {
        None
    }
}

fn finite_samples(series: &[f64]) -> Vec<f64> {
    series
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn ewma_single_value_returns_self() {
        let s = EwmaSmoother::new(10);
        assert_eq!(s.smooth(&[0.5]), Some(0.5));
    }

    #[test]
    fn ewma_constant_series_returns_constant() {
        let s = EwmaSmoother::new(5);
        let v = s.smooth(&[1.0; 100]).unwrap();
        assert!((v - 1.0).abs() < 1e-12);
    }

    #[test]
    fn ewma_responds_to_recent_value() {
        let s = EwmaSmoother::new(2); // alpha = 2/3
                                      // 序列：0,0,0,0,1 → 最近值贡献最大
        let v = s.smooth(&[0.0, 0.0, 0.0, 0.0, 1.0]).unwrap();
        // 估算：alpha*1 + (1-alpha)*0 = 2/3
        assert!((v - 2.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn ewma_empty_returns_none() {
        let s = EwmaSmoother::new(10);
        assert!(s.smooth(&[]).is_none());
    }

    #[test]
    fn detect_outlier_in_normal_range() {
        let series: Vec<f64> = (1..=10).map(|i| i as f64 * 0.0001).collect();
        let (mean, std, is_out) = detect_outlier(&series, 0.0005, 3.0);
        assert!(mean > 0.0);
        assert!(std > 0.0);
        assert!(!is_out);
    }

    #[test]
    fn detect_outlier_uses_sample_variance() {
        let (_, std, _) = detect_outlier(&[1.0, 2.0, 3.0], 2.0, 3.0);

        assert!((std - 1.0).abs() < 1e-12);
    }

    #[test]
    fn detect_outlier_extreme() {
        let series = vec![0.0001, 0.00012, 0.0001, 0.00011, 0.00009];
        let (_, _, is_out) = detect_outlier(&series, 0.01, 3.0);
        assert!(is_out, "0.01 应被识别为异常");
    }

    #[test]
    fn detect_outlier_short_series_returns_false() {
        let (_, _, is_out) = detect_outlier(&[0.0001, 0.0002], 0.99, 3.0);
        assert!(!is_out, "样本不足应返回 false");
    }

    #[test]
    fn detect_outlier_ignores_non_finite_samples() {
        let series = [0.0001, f64::NAN, 0.00012, f64::INFINITY, 0.00011];
        let (mean, std, is_out) = detect_outlier(&series, 0.01, 3.0);

        assert!(mean.is_finite());
        assert!(std.is_finite());
        assert!(is_out);
    }

    #[test]
    fn apply_normalization_writes_back_fields() {
        let mut data = FundingRateData {
            symbol: "BTC".into(),
            exchange: "binance".into(),
            rate: 0.0001,
            rate_8h: 0.0001,
            predicted_rate: None,
            next_funding_time: 0,
            funding_interval: 8,
            volume_24h: 0.0,
            timestamp: 0,
            smoothed_rate: None,
            rate_std: None,
            is_outlier: false,
        };
        let history: Vec<f64> = vec![0.0001, 0.00012, 0.00009, 0.00011, 0.0001];
        apply_normalization(&mut data, &history, 5, 3.0);
        assert!(data.smoothed_rate.is_some());
        assert!(data.rate_std.is_some());
        assert!(!data.is_outlier);
    }

    #[test]
    fn apply_normalization_reports_sample_evidence() {
        let mut data = FundingRateData {
            symbol: "BTC".into(),
            exchange: "binance".into(),
            rate: 0.0001,
            rate_8h: 0.0001,
            predicted_rate: None,
            next_funding_time: 0,
            funding_interval: 8,
            volume_24h: 0.0,
            timestamp: 0,
            smoothed_rate: None,
            rate_std: None,
            is_outlier: false,
        };
        let history = [0.0001, f64::NAN, 0.00011, f64::INFINITY, 0.00012];
        let evidence = apply_normalization_with_evidence(&mut data, &history, 5, 3.0);

        assert_eq!(evidence.sample_count, 3);
        assert_eq!(evidence.excluded_sample_count, 2);
        assert_eq!(evidence.min_sample_count, 3);
        assert_eq!(evidence.ewma_span, 5);
        assert_eq!(evidence.missing_reason, Some("excluded_non_finite_samples"));
        assert!(data.smoothed_rate.is_some_and(f64::is_finite));
        assert!(data.rate_std.is_some_and(f64::is_finite));
    }
}
