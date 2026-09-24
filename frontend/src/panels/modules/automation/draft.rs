use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::{
    AutoProfitCloseConfig, AutoProfitCloseConfigPatch, AutomatedArbitrageConfig,
    AutomatedArbitrageConfigPatch, AutomationRuntimeStatus, StrategyKind,
};

#[derive(Clone, Copy)]
pub(super) struct AutomationConfigDraft {
    pub dirty: RwSignal<bool>,
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
    pub dirty: RwSignal<bool>,
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
    pub(super) fn new(
        status: RwSignal<LoadState<AutomationRuntimeStatus>>,
        saved: RwSignal<u64>,
    ) -> Self {
        let defaults = AutomatedArbitrageConfig::default();
        let draft = Self {
            dirty: RwSignal::new(false),
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
        let accepted = RwSignal::new(0_u64);
        let config = Memo::new(move |_| {
            status.with(|state| state.value().map(|value| value.config.clone()))
        });
        Effect::new(move |_| {
            let value = config.get();
            let saved = saved.get();
            if let Some(value) = value {
                if !hydrated.get_untracked()
                    || saved != accepted.get_untracked()
                    || !draft.dirty.get_untracked()
                {
                    draft.hydrate(&value);
                    draft.dirty.set(false);
                    hydrated.set(true);
                }
                accepted.set(saved);
            }
        });
        draft
    }

    pub(super) fn patch(self) -> Result<AutomatedArbitrageConfigPatch, String> {
        Ok(AutomatedArbitrageConfigPatch {
            strategy_kind: Some(self.strategy_kind.get_untracked()),
            canonical_symbols: Some(parse_canonical_symbols(
                &self.canonical_symbols.get_untracked(),
            )),
            capital_usd: Some(bounded("资金", self.capital, 1.0, 1_000_000.0, "USD")?),
            min_one_cycle_net_bps: Some(percent_to_bps(bounded(
                "最低费后净利",
                self.min_net,
                0.0001,
                100.0,
                "%",
            )?)),
            min_depth_usd: Some(bounded(
                "最低双腿深度",
                self.min_depth,
                1.0,
                1_000_000_000.0,
                "USD",
            )?),
            leverage: Some(bounded("杠杆", self.leverage, 1.0, 20.0, "倍")?),
            max_concurrent_runs: Some(integer("最大并发", self.concurrency, 1, 8)? as usize),
            cooldown_secs: Some(integer(
                "入场冷却",
                self.cooldown,
                shared_types::MIN_AUTOMATION_ENTRY_COOLDOWN_SECS,
                86_400,
            )?),
            ..AutomatedArbitrageConfigPatch::default()
        })
    }

    fn hydrate(self, config: &AutomatedArbitrageConfig) {
        self.strategy_kind.set(config.strategy_kind);
        self.canonical_symbols
            .set(config.canonical_symbols.join(", "));
        self.capital.set(config.capital_usd.to_string());
        self.min_net
            .set(bps_percent_text(config.min_one_cycle_net_bps));
        self.min_depth.set(config.min_depth_usd.to_string());
        self.leverage.set(config.leverage.to_string());
        self.concurrency.set(config.max_concurrent_runs.to_string());
        self.cooldown.set(config.cooldown_secs.to_string());
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
    pub(super) fn new(
        protection: RwSignal<LoadState<AutoProfitCloseConfig>>,
        saved: RwSignal<u64>,
    ) -> Self {
        let draft = Self {
            dirty: RwSignal::new(false),
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
        let accepted = RwSignal::new(0_u64);
        let config = Memo::new(move |_| protection.with(|state| state.value().cloned()));
        Effect::new(move |_| {
            let value = config.get();
            let saved = saved.get();
            if let Some(value) = value {
                if !hydrated.get_untracked()
                    || saved != accepted.get_untracked()
                    || !draft.dirty.get_untracked()
                {
                    draft.hydrate(&value);
                    draft.dirty.set(false);
                    hydrated.set(true);
                }
                accepted.set(saved);
            }
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

fn integer(label: &str, signal: RwSignal<String>, min: u64, max: u64) -> Result<u64, String> {
    signal
        .get_untracked()
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|value| (min..=max).contains(value))
        .ok_or_else(|| format!("{label}必须是 {min} 到 {max} 之间的整数"))
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
    use super::*;

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

    #[test]
    fn invalid_entry_numbers_are_rejected_instead_of_being_omitted() {
        let owner = Owner::new();
        owner.with(|| {
            let defaults = AutomatedArbitrageConfig::default();
            let draft = AutomationConfigDraft {
                dirty: RwSignal::new(false),
                strategy_kind: RwSignal::new(defaults.strategy_kind),
                canonical_symbols: RwSignal::new(String::new()),
                capital: RwSignal::new(defaults.capital_usd.to_string()),
                min_net: RwSignal::new(bps_percent_text(defaults.min_one_cycle_net_bps)),
                min_depth: RwSignal::new(defaults.min_depth_usd.to_string()),
                leverage: RwSignal::new(defaults.leverage.to_string()),
                concurrency: RwSignal::new(defaults.max_concurrent_runs.to_string()),
                cooldown: RwSignal::new(defaults.cooldown_secs.to_string()),
            };
            draft.capital.set(String::new());
            assert!(draft.patch().unwrap_err().contains("资金"));
            draft.capital.set("12.75".into());
            draft.cooldown.set("1.5".into());
            assert!(draft.patch().unwrap_err().contains("整数"));
            draft.cooldown.set("1".into());
            draft.min_net.set("0.0001".into());
            let patch = draft.patch().unwrap();
            assert_eq!(patch.capital_usd, Some(12.75));
            assert_eq!(patch.cooldown_secs, Some(1));
            assert_eq!(patch.min_one_cycle_net_bps, Some(0.01));
            draft.min_net.set("0".into());
            assert!(draft.patch().is_err());
        });
    }
}
