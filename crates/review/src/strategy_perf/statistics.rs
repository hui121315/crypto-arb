use super::*;

pub(super) fn sharpe_ratio(pnl: &[f64]) -> f64 {
    let std = std_dev(pnl, mean(pnl));
    if std <= f64::EPSILON {
        0.0
    } else {
        mean(pnl) / std * CRYPTO_TRADING_DAYS_PER_YEAR.sqrt()
    }
}

pub(super) fn sortino_ratio(pnl: &[f64]) -> f64 {
    let downside: Vec<f64> = pnl.iter().copied().filter(|value| *value < 0.0).collect();
    let std = std_dev(&downside, 0.0);
    if std <= f64::EPSILON {
        0.0
    } else {
        mean(pnl) / std * CRYPTO_TRADING_DAYS_PER_YEAR.sqrt()
    }
}

pub(super) fn max_drawdown_pct(pnl: &[f64]) -> f64 {
    let mut peak = 0.0_f64;
    let mut equity = 0.0_f64;
    let mut max_drawdown = 0.0_f64;
    for value in pnl {
        equity += value;
        peak = peak.max(equity);
        if peak > f64::EPSILON {
            max_drawdown = max_drawdown.max((peak - equity) / peak * 100.0);
        }
    }
    max_drawdown
}

pub(super) fn max_drawdown_usd(pnl: &[f64]) -> f64 {
    let mut peak = 0.0_f64;
    let mut equity = 0.0_f64;
    let mut max_drawdown = 0.0_f64;
    for value in pnl {
        equity += value;
        peak = peak.max(equity);
        max_drawdown = max_drawdown.max(peak - equity);
    }
    max_drawdown
}

pub(super) fn avg_holding_hours(rows: &[&ExecutedTrade]) -> f64 {
    let mut total = 0.0;
    let mut count = 0;
    for row in rows {
        if let Some(minutes) = row.holding_minutes {
            total += minutes as f64 / 60.0;
            count += 1;
        }
    }
    if count == 0 {
        0.0
    } else {
        total / count as f64
    }
}

pub(super) fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f64>() / values.len() as f64
    }
}

pub(super) fn std_dev(values: &[f64], avg: f64) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    let variance = values
        .iter()
        .map(|value| (value - avg).powi(2))
        .sum::<f64>()
        / values.len() as f64;
    variance.sqrt()
}

pub(super) fn pct(n: f64, d: f64) -> f64 {
    if d.abs() > f64::EPSILON {
        n / d * 100.0
    } else {
        0.0
    }
}
