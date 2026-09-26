use leptos::prelude::*;

use shared_types::TradingRiskStatus;

#[path = "fields/protection.rs"]
mod protection;
pub(super) use protection::auto_pair_exit_fields;

#[derive(Clone, Copy)]
pub(super) struct AutoProfitCloseSignals {
    pub(super) enabled: RwSignal<bool>,
    pub(super) min_net_profit_usd: RwSignal<String>,
    pub(super) min_roi_pct: RwSignal<String>,
    pub(super) exit_buffer_pct: RwSignal<String>,
    pub(super) stop_loss_enabled: RwSignal<bool>,
    pub(super) max_net_loss_usd: RwSignal<String>,
    pub(super) max_loss_roi_pct: RwSignal<String>,
    pub(super) liquidation_guard_enabled: RwSignal<bool>,
    pub(super) liquidation_exit_distance_pct: RwSignal<String>,
    pub(super) confirmation_samples: RwSignal<String>,
    pub(super) cooldown_secs: RwSignal<String>,
}

impl AutoProfitCloseSignals {
    pub(super) fn new() -> Self {
        Self {
            enabled: RwSignal::new(false),
            min_net_profit_usd: RwSignal::new(String::new()),
            min_roi_pct: RwSignal::new(String::new()),
            exit_buffer_pct: RwSignal::new(String::new()),
            stop_loss_enabled: RwSignal::new(false),
            max_net_loss_usd: RwSignal::new(String::new()),
            max_loss_roi_pct: RwSignal::new(String::new()),
            liquidation_guard_enabled: RwSignal::new(false),
            liquidation_exit_distance_pct: RwSignal::new(String::new()),
            confirmation_samples: RwSignal::new(String::new()),
            cooldown_secs: RwSignal::new(String::new()),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct RiskThresholdSignals {
    pub(super) max_order: RwSignal<String>,
    pub(super) max_open: RwSignal<String>,
    pub(super) imbalance_pct: RwSignal<String>,
    pub(super) allowed_exchanges: RwSignal<String>,
    pub(super) allowed_symbols: RwSignal<String>,
}

#[derive(Clone, Copy)]
pub(super) struct RiskFormSignals {
    pub(super) initialized: RwSignal<bool>,
    pub(super) thresholds: RiskThresholdSignals,
    pub(super) auto_close: AutoProfitCloseSignals,
}

pub(super) fn apply_form(risk: &TradingRiskStatus, signals: RiskFormSignals) {
    signals
        .thresholds
        .max_order
        .set(risk.max_order_notional.to_string());
    signals
        .thresholds
        .max_open
        .set(risk.max_open_orders.to_string());
    signals
        .thresholds
        .imbalance_pct
        .set((risk.max_hedge_imbalance_pct * 100.0).to_string());
    signals
        .thresholds
        .allowed_exchanges
        .set(risk.allowed_exchanges.join(", "));
    signals
        .thresholds
        .allowed_symbols
        .set(risk.allowed_symbols.join(", "));
    signals
        .auto_close
        .enabled
        .set(risk.auto_profit_close.enabled);
    signals
        .auto_close
        .min_net_profit_usd
        .set(risk.auto_profit_close.min_net_profit_usd.to_string());
    signals
        .auto_close
        .min_roi_pct
        .set((risk.auto_profit_close.min_roi_bps / 100.0).to_string());
    signals
        .auto_close
        .exit_buffer_pct
        .set((risk.auto_profit_close.exit_buffer_bps / 100.0).to_string());
    signals
        .auto_close
        .stop_loss_enabled
        .set(risk.auto_profit_close.stop_loss_enabled);
    signals
        .auto_close
        .max_net_loss_usd
        .set(risk.auto_profit_close.max_net_loss_usd.to_string());
    signals
        .auto_close
        .max_loss_roi_pct
        .set((risk.auto_profit_close.max_loss_roi_bps / 100.0).to_string());
    signals
        .auto_close
        .liquidation_guard_enabled
        .set(risk.auto_profit_close.liquidation_guard_enabled);
    signals.auto_close.liquidation_exit_distance_pct.set(
        risk.auto_profit_close
            .liquidation_exit_distance_pct
            .to_string(),
    );
    signals
        .auto_close
        .confirmation_samples
        .set(risk.auto_profit_close.confirmation_samples.to_string());
    signals
        .auto_close
        .cooldown_secs
        .set(risk.auto_profit_close.cooldown_secs.to_string());
    signals.initialized.set(true);
}

pub(super) fn risk_threshold_fields(signals: RiskThresholdSignals) -> impl IntoView {
    view! {
        <div class="settings-section">
            <label class="settings-control">
                <span>"单笔名义上限 USD"</span>
                <input
                    inputmode="decimal"
                    prop:value=move || signals.max_order.get()
                    on:input=move |ev| signals.max_order.set(event_target_value(&ev))
                />
                <em>"必须大于 0"</em>
            </label>
            <label class="settings-control">
                <span>"最大挂单数"</span>
                <input
                    inputmode="numeric"
                    prop:value=move || signals.max_open.get()
                    on:input=move |ev| signals.max_open.set(event_target_value(&ev))
                />
                <em>"必须大于 0"</em>
            </label>
            <label class="settings-control">
                <span>"双腿偏差 %"</span>
                <input
                    inputmode="decimal"
                    prop:value=move || signals.imbalance_pct.get()
                    on:input=move |ev| signals.imbalance_pct.set(event_target_value(&ev))
                />
                <em>"0 到 100，输入 1 表示 1%"</em>
            </label>
            <label class="settings-control">
                <span>"允许交易所"</span>
                <input
                    prop:value=move || signals.allowed_exchanges.get()
                    on:input=move |ev| signals.allowed_exchanges.set(event_target_value(&ev))
                />
                <em>"逗号分隔，留空表示不限制"</em>
            </label>
            <label class="settings-control">
                <span>"允许品种"</span>
                <input
                    prop:value=move || signals.allowed_symbols.get()
                    on:input=move |ev| signals.allowed_symbols.set(event_target_value(&ev))
                />
                <em>"逗号分隔，留空表示不限制"</em>
            </label>
        </div>
    }
}
