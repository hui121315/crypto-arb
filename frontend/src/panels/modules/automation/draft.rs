use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::{
    AutoProfitCloseConfig, AutoProfitCloseConfigPatch, AutomatedArbitrageConfig,
    AutomatedArbitrageConfigPatch, AutomationRuntimeStatus, StrategyKind,
};

#[derive(Clone, Copy)]
pub(super) struct AutomationConfigDraft {
    pub strategy_kind: RwSignal<StrategyKind>,
    pub canonical_symbols: RwSignal<String>,
    pub capital: RwSignal<String>,
    pub min_net: RwSignal<String>,
    pub min_depth: RwSignal<String>,
    pub leverage: RwSignal<String>,
    pub concurrency: RwSignal<String>,
    pub cooldown: RwSignal<String>,
}

#[derive(Clone, Copy)]
pub(super) struct AutomationProtectionDraft {
    pub take_profit: RwSignal<bool>,
    pub min_profit_usd: RwSignal<String>,
    pub min_profit_bps: RwSignal<String>,
    pub stop_loss: RwSignal<bool>,
    pub max_loss_usd: RwSignal<String>,
    pub max_loss_bps: RwSignal<String>,
    pub liquidation_guard: RwSignal<bool>,
    pub liquidation_distance_pct: RwSignal<String>,
}

impl AutomationConfigDraft {
    pub(super) fn new(status: RwSignal<LoadState<AutomationRuntimeStatus>>) -> Self {
        let defaults = AutomatedArbitrageConfig::default();
        let draft = Self {
            strategy_kind: RwSignal::new(defaults.strategy_kind),
            canonical_symbols: RwSignal::new(defaults.canonical_symbols.join(", ")),
            capital: RwSignal::new(defaults.capital_usd.to_string()),
            min_net: RwSignal::new(bps_percent_text(defaults.min_one_cycle_net_bps)),
            min_depth: RwSignal::new(defaults.min_depth_usd.to_string()),
            leverage: RwSignal::new(defaults.leverage.to_string()),
            concurrency: RwSignal::new(defaults.max_concurrent_runs.to_string()),
            cooldown: RwSignal::new(defaults.cooldown_secs.to_string()),
        };
        let hydrated = RwSignal::new(false);
        Effect::new(move |_| {
            if hydrated.get_untracked() {
                return;
            }
            let did_hydrate = status.with(|state| {
                let Some(value) = state.value() else {
                    return false;
                };
                draft.hydrate(value);
                true
            });
            hydrated.set(did_hydrate);
        });
        draft
    }

    pub(super) fn patch(self) -> AutomatedArbitrageConfigPatch {
        AutomatedArbitrageConfigPatch {
            strategy_kind: Some(self.strategy_kind.get_untracked()),
            canonical_symbols: Some(parse_canonical_symbols(
                &self.canonical_symbols.get_untracked(),
            )),
            capital_usd: self.capital.get_untracked().parse().ok(),
            min_one_cycle_net_bps: finite_number(&self.min_net.get_untracked()).map(percent_to_bps),
            min_depth_usd: self.min_depth.get_untracked().parse().ok(),
            leverage: self.leverage.get_untracked().parse().ok(),
            max_concurrent_runs: self.concurrency.get_untracked().parse().ok(),
            cooldown_secs: self.cooldown.get_untracked().parse().ok(),
            ..AutomatedArbitrageConfigPatch::default()
        }
    }

    fn hydrate(self, status: &AutomationRuntimeStatus) {
        self.strategy_kind.set(status.config.strategy_kind);
        self.canonical_symbols
            .set(status.config.canonical_symbols.join(", "));
        self.capital.set(status.config.capital_usd.to_string());
        self.min_net
            .set(bps_percent_text(status.config.min_one_cycle_net_bps));
        self.min_depth.set(status.config.min_depth_usd.to_string());
        self.leverage.set(status.config.leverage.to_string());
        self.concurrency
            .set(status.config.max_concurrent_runs.to_string());
        self.cooldown.set(status.config.cooldown_secs.to_string());
    }
}

fn parse_canonical_symbols(value: &str) -> Vec<String> {
    value
        .split(|character: char| character == ',' || character == '，' || character.is_whitespace())
        .map(str::trim)
        .filter(|symbol| !symbol.is_empty())
        .map(str::to_owned)
        .collect()
}

impl AutomationProtectionDraft {
    pub(super) fn new(protection: RwSignal<LoadState<AutoProfitCloseConfig>>) -> Self {
        let draft = Self {
            take_profit: RwSignal::new(false),
            min_profit_usd: RwSignal::new("5".to_owned()),
            min_profit_bps: RwSignal::new("0.1".to_owned()),
            stop_loss: RwSignal::new(false),
            max_loss_usd: RwSignal::new("25".to_owned()),
            max_loss_bps: RwSignal::new("1".to_owned()),
            liquidation_guard: RwSignal::new(false),
            liquidation_distance_pct: RwSignal::new("8".to_owned()),
        };
        let hydrated = RwSignal::new(false);
        Effect::new(move |_| {
            if hydrated.get_untracked() {
                return;
            }
            let did_hydrate = protection.with(|state| {
                let Some(value) = state.value() else {
                    return false;
                };
                draft.hydrate(value);
                true
            });
            hydrated.set(did_hydrate);
        });
        draft
    }

    pub(super) fn patch(self) -> Result<AutoProfitCloseConfigPatch, String> {
        Ok(AutoProfitCloseConfigPatch {
            enabled: Some(self.take_profit.get_untracked()),
            min_net_profit_usd: Some(positive("最低净利润", self.min_profit_usd)?),
            min_roi_bps: Some(percent_to_bps(bounded(
                "最低收益",
                self.min_profit_bps,
                0.0001,
                100.0,
                "%",
            )?)),
            stop_loss_enabled: Some(self.stop_loss.get_untracked()),
            max_net_loss_usd: Some(positive("最大净亏损", self.max_loss_usd)?),
            max_loss_roi_bps: Some(percent_to_bps(bounded(
                "最大亏损",
                self.max_loss_bps,
                0.0001,
                100.0,
                "%",
            )?)),
            liquidation_guard_enabled: Some(self.liquidation_guard.get_untracked()),
            liquidation_exit_distance_pct: Some(bounded(
                "强平退出距离",
                self.liquidation_distance_pct,
                0.1,
                100.0,
                "%",
            )?),
            ..AutoProfitCloseConfigPatch::default()
        })
    }

    fn hydrate(self, config: &AutoProfitCloseConfig) {
        self.take_profit.set(config.enabled);
        self.min_profit_usd
            .set(config.min_net_profit_usd.to_string());
        self.min_profit_bps
            .set(bps_percent_text(config.min_roi_bps));
        self.stop_loss.set(config.stop_loss_enabled);
        self.max_loss_usd.set(config.max_net_loss_usd.to_string());
        self.max_loss_bps
            .set(bps_percent_text(config.max_loss_roi_bps));
        self.liquidation_guard.set(config.liquidation_guard_enabled);
        self.liquidation_distance_pct
            .set(config.liquidation_exit_distance_pct.to_string());
    }
}

fn positive(label: &str, signal: RwSignal<String>) -> Result<f64, String> {
    let value = number(label, signal)?;
    if value > 0.0 {
        Ok(value)
    } else {
        Err(format!("{label}必须大于 0"))
    }
}

fn bounded(
    label: &str,
    signal: RwSignal<String>,
    min: f64,
    max: f64,
    unit: &str,
) -> Result<f64, String> {
    let value = number(label, signal)?;
    if (min..=max).contains(&value) {
        Ok(value)
    } else {
        Err(format!("{label}必须在 {min} 到 {max} {unit} 之间"))
    }
}

fn number(label: &str, signal: RwSignal<String>) -> Result<f64, String> {
    finite_number(&signal.get_untracked()).ok_or_else(|| format!("{label}必须是有效数字"))
}

pub(super) fn finite_number(value: &str) -> Option<f64> {
    value
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

const fn percent_to_bps(value: f64) -> f64 {
    value * 100.0
}

fn bps_percent_text(value: f64) -> String {
    (value / 100.0).to_string()
}

#[cfg(test)]
mod tests {
    use super::{bps_percent_text, finite_number, percent_to_bps};

    #[test]
    fn percentage_controls_round_trip_backend_basis_points() {
        assert_eq!(percent_to_bps(0.1), 10.0);
        assert_eq!(percent_to_bps(1.0), 100.0);
        assert_eq!(bps_percent_text(10.0), "0.1");
        assert_eq!(bps_percent_text(100.0), "1");
    }

    #[test]
    fn protection_parser_accepts_fractional_input() {
        assert_eq!(finite_number("0.01"), Some(0.01));
        assert_eq!(finite_number(".5"), Some(0.5));
    }
}
