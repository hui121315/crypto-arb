use super::fields::{apply_form, AutoProfitCloseSignals, RiskFormSignals, RiskThresholdSignals};
use super::form::{risk_patch_from_inputs, AutoProfitCloseInputs, RiskThresholdInputs};
use crate::panels::modules::settings::data::{
    current_trading_refresh, use_kill_switch_action, use_risk_config_save_action, KillSwitchAction,
    RiskConfigSaveAction, SettingsJournal,
};
use crate::state::{action_state::ActionState, trading_status::TradingStatusState};
use leptos::prelude::*;
use shared_types::{ActionRun, ActionRunKind, ActionRunStatus, RiskConfigPatch, TradingRiskStatus};

#[derive(Clone, Copy)]
pub(in crate::panels) struct RiskConfigRuntime {
    pub(super) form: RiskFormSignals,
    pub(super) dirty: RwSignal<bool>,
    pub(super) baseline: RwSignal<Option<TradingRiskStatus>>,
    pub(super) message: RwSignal<String>,
    pub(super) refresh: Callback<()>,
    pub(in crate::panels) recheck: Callback<()>,
    pub(super) save: RiskConfigSaveAction,
    pub(in crate::panels) kill: KillSwitchAction,
}

pub(in crate::panels) fn create_risk_config_runtime() -> RiskConfigRuntime {
    let journal = SettingsJournal::new("risk");
    let refresh = current_trading_refresh(journal);
    let save = use_risk_config_save_action(journal);
    let kill = use_kill_switch_action(journal);
    let dirty = RwSignal::new(false);
    let recheck = journal.recheck(Callback::new(move |run: ActionRun| {
        let result = crate::state::action_state::action_state_from_action_run(&run);
        if run.kind == ActionRunKind::TradingRiskConfigUpdate {
            save.state.set(result);
            save.receipt.set(None);
            if run.status == ActionRunStatus::Succeeded {
                dirty.set(false);
            }
        } else {
            kill.state.set(result);
        }
        refresh.run(());
    }));
    let runtime = RiskConfigRuntime {
        form: RiskFormSignals {
            initialized: RwSignal::new(false),
            thresholds: RiskThresholdSignals {
                max_order: RwSignal::new(String::new()),
                max_open: RwSignal::new(String::new()),
                imbalance_pct: RwSignal::new(String::new()),
                allowed_exchanges: RwSignal::new(String::new()),
                allowed_symbols: RwSignal::new(String::new()),
            },
            auto_close: AutoProfitCloseSignals::new(),
        },
        dirty,
        baseline: RwSignal::new(None),
        message: RwSignal::new("风控参数写入后端后生效。".into()),
        refresh,
        recheck,
        save,
        kill,
    };
    let status = expect_context::<TradingStatusState>().state;
    Effect::new(move |_| {
        let value = status.get();
        let pending = runtime.pending();
        if !runtime.form.initialized.get()
            || (!runtime.dirty.get() && !pending && !runtime.unresolved())
        {
            if let Some(value) = value.value() {
                if runtime.baseline.get_untracked().as_ref() != Some(&value.risk) {
                    runtime.apply(&value.risk);
                }
            }
        }
    });
    Effect::new(move |_| {
        if let Some(receipt) = runtime.save.receipt.get() {
            runtime.apply(&receipt.risk);
        }
    });
    Effect::new(move |previous: Option<u64>| {
        let connection = journal.connection.get();
        if previous.is_some_and(|old| old != connection) {
            runtime.dirty.set(false);
            runtime.baseline.set(None);
            runtime.form.initialized.set(false);
            refresh.run(());
        }
        connection
    });
    runtime
}

impl RiskConfigRuntime {
    pub(in crate::panels) fn save_exit(self, exit: shared_types::AutoProfitCloseConfigPatch) {
        self.save.submit.run(RiskConfigPatch {
            max_order_notional: None, max_open_orders: None, max_hedge_imbalance_pct: None,
            allowed_exchanges: None, allowed_symbols: None, protected_positions: None,
            auto_profit_close: Some(exit),
        });
    }

    pub(in crate::panels) fn save_state(self) -> ActionState {
        self.save.state.get()
    }

    pub(in crate::panels) fn pending(self) -> bool {
        self.save.journal.busy.get()
    }

    pub(super) fn unresolved(self) -> bool {
        self.save.journal.locked()
    }

    pub(super) fn save_unresolved(self) -> bool {
        self.save.journal.pending.with(|pending| {
            pending
                .as_ref()
                .is_some_and(|row| row.kind == ActionRunKind::TradingRiskConfigUpdate)
        })
    }

    pub(super) fn kill_unresolved(self) -> bool {
        self.kill.journal.pending.with(|pending| {
            pending
                .as_ref()
                .is_some_and(|row| row.kind == ActionRunKind::TradingKillSwitch)
        })
    }

    pub(super) fn edit(self) {
        if self.pending() || self.unresolved() {
            return;
        }
        self.dirty.set(true);
        self.save.state.set(ActionState::Idle);
        self.message.set("存在未保存修改。".into());
    }

    pub(super) fn apply(self, risk: &TradingRiskStatus) {
        apply_form(risk, self.form);
        self.baseline.set(Some(risk.clone()));
        self.message.set("当前配置与后台一致。".into());
        if self.dirty.get_untracked() {
            self.dirty.set(false);
        }
    }

    pub(super) fn patch(
        self,
        current: &TradingRiskStatus,
    ) -> Result<Option<RiskConfigPatch>, String> {
        let t = self.form.thresholds;
        let a = self.form.auto_close;
        let mut patch = risk_patch_from_inputs(
            &RiskThresholdInputs {
                max_order: t.max_order.get_untracked(),
                max_open: t.max_open.get_untracked(),
                imbalance_pct: t.imbalance_pct.get_untracked(),
                allowed_exchanges: t.allowed_exchanges.get_untracked(),
                allowed_symbols: t.allowed_symbols.get_untracked(),
            },
            &AutoProfitCloseInputs {
                enabled: a.enabled.get_untracked(),
                min_net_profit_usd: a.min_net_profit_usd.get_untracked(),
                min_roi_pct: a.min_roi_pct.get_untracked(),
                exit_buffer_pct: a.exit_buffer_pct.get_untracked(),
                stop_loss_enabled: a.stop_loss_enabled.get_untracked(),
                max_net_loss_usd: a.max_net_loss_usd.get_untracked(),
                max_loss_roi_pct: a.max_loss_roi_pct.get_untracked(),
                liquidation_guard_enabled: a.liquidation_guard_enabled.get_untracked(),
                liquidation_exit_distance_pct: a.liquidation_exit_distance_pct.get_untracked(),
                confirmation_samples: a.confirmation_samples.get_untracked(),
                cooldown_secs: a.cooldown_secs.get_untracked(),
            },
        )?;
        let baseline = self.baseline.get_untracked().ok_or("尚未读取原风控配置")?;
        // Unedited percentage text must not become a write through f64 unit round-trips.
        if t.imbalance_pct.get_untracked().trim()
            == (baseline.max_hedge_imbalance_pct * 100.0).to_string()
        {
            patch.max_hedge_imbalance_pct = None;
        }
        if let Some(exit) = patch.auto_profit_close.as_mut() {
            let before = &baseline.auto_profit_close;
            if a.min_roi_pct.get_untracked().trim() == (before.min_roi_bps / 100.0).to_string() {
                exit.min_roi_bps = None;
            }
            if a.exit_buffer_pct.get_untracked().trim()
                == (before.exit_buffer_bps / 100.0).to_string()
            {
                exit.exit_buffer_bps = None;
            }
            if a.max_loss_roi_pct.get_untracked().trim()
                == (before.max_loss_roi_bps / 100.0).to_string()
            {
                exit.max_loss_roi_bps = None;
            }
        }
        changed_patch(patch, &baseline, current)
    }
}

fn changed_patch(
    mut patch: RiskConfigPatch,
    before: &TradingRiskStatus,
    current: &TradingRiskStatus,
) -> Result<Option<RiskConfigPatch>, String> {
    let mut changed = false;
    macro_rules! field {
        ($patch:ident, $before:ident, $current:ident, $field:ident) => {
            if $patch.$field.as_ref().is_none_or(|value| value == &$before.$field || value == &$current.$field) {
                $patch.$field = None;
            } else if $before.$field != $current.$field {
                return Err("后台同一参数已变化，请载入最新配置后重试；当前输入已保留。".into());
            } else { changed = true; }
        };
    }
    field!(patch, before, current, max_order_notional);
    field!(patch, before, current, max_open_orders);
    field!(patch, before, current, max_hedge_imbalance_pct);
    field!(patch, before, current, allowed_exchanges);
    field!(patch, before, current, allowed_symbols);
    if let Some(mut exit) = patch.auto_profit_close.take() {
        let before = &before.auto_profit_close;
        let current = &current.auto_profit_close;
        let parent_changed = changed;
        changed = false;
        field!(exit, before, current, enabled);
        field!(exit, before, current, min_net_profit_usd);
        field!(exit, before, current, min_roi_bps);
        field!(exit, before, current, exit_buffer_bps);
        field!(exit, before, current, stop_loss_enabled);
        field!(exit, before, current, max_net_loss_usd);
        field!(exit, before, current, max_loss_roi_bps);
        field!(exit, before, current, liquidation_guard_enabled);
        field!(exit, before, current, liquidation_exit_distance_pct);
        field!(exit, before, current, confirmation_samples);
        field!(exit, before, current, cooldown_secs);
        if changed {
            patch.auto_profit_close = Some(exit);
        }
        changed |= parent_changed;
    }
    Ok(changed.then_some(patch))
}
