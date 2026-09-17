use leptos::prelude::*;

use super::AutoProfitCloseSignals;

pub(in crate::panels::modules::settings) fn auto_pair_exit_fields(
    signals: AutoProfitCloseSignals,
) -> impl IntoView {
    view! {
        <div class="settings-protection-stack">
            {take_profit_fields(signals)}
            {stop_loss_fields(signals)}
            {liquidation_guard_fields(signals)}
            {execution_guard_fields(signals)}
        </div>
    }
}

fn take_profit_fields(signals: AutoProfitCloseSignals) -> impl IntoView {
    view! {
        <div class="settings-protection-group">
            <strong>"止盈"</strong>
            <div class="settings-section">
                <label class="settings-control toggle">
                    <span>"自动止盈并平双边"</span>
                    <input
                        type="checkbox"
                        prop:checked=move || signals.enabled.get()
                        on:change=move |ev| signals.enabled.set(event_target_checked(&ev))
                    />
                    <em>"净利润与净收益率必须同时达到阈值"</em>
                </label>
                <label class="settings-control">
                    <span>"最低净利润 USD"</span>
                    <input
                        inputmode="decimal"
                        prop:value=move || signals.min_net_profit_usd.get()
                        on:input=move |ev| signals.min_net_profit_usd.set(event_target_value(&ev))
                    />
                    <em>"已扣开平仓成本、滑点与安全缓冲"</em>
                </label>
                <label class="settings-control">
                    <span>"最低净收益率 %"</span>
                    <input
                        inputmode="decimal"
                        prop:value=move || signals.min_roi_pct.get()
                        on:input=move |ev| signals.min_roi_pct.set(event_target_value(&ev))
                    />
                    <em>"0.10 表示 0.10%"</em>
                </label>
            </div>
        </div>
    }
}

fn stop_loss_fields(signals: AutoProfitCloseSignals) -> impl IntoView {
    view! {
        <div class="settings-protection-group">
            <strong>"止损"</strong>
            <div class="settings-section">
                <label class="settings-control toggle">
                    <span>"自动止损并平双边"</span>
                    <input
                        type="checkbox"
                        prop:checked=move || signals.stop_loss_enabled.get()
                        on:change=move |ev| {
                            signals.stop_loss_enabled.set(event_target_checked(&ev))
                        }
                    />
                    <em>"净亏损或亏损率任一达到阈值即触发"</em>
                </label>
                <label class="settings-control">
                    <span>"最大净亏损 USD"</span>
                    <input
                        inputmode="decimal"
                        prop:value=move || signals.max_net_loss_usd.get()
                        on:input=move |ev| signals.max_net_loss_usd.set(event_target_value(&ev))
                    />
                    <em>"使用双腿成本净额，不按单腿浮亏误判"</em>
                </label>
                <label class="settings-control">
                    <span>"最大亏损率 %"</span>
                    <input
                        inputmode="decimal"
                        prop:value=move || signals.max_loss_roi_pct.get()
                        on:input=move |ev| signals.max_loss_roi_pct.set(event_target_value(&ev))
                    />
                    <em>"1.00 表示双腿匹配名义价值的 1%"</em>
                </label>
            </div>
        </div>
    }
}

fn liquidation_guard_fields(signals: AutoProfitCloseSignals) -> impl IntoView {
    view! {
        <div class="settings-protection-group">
            <strong>"单腿强平保护"</strong>
            <div class="settings-section">
                <label class="settings-control toggle">
                    <span>"任一腿接近强平时平双边"</span>
                    <input
                        type="checkbox"
                        prop:checked=move || signals.liquidation_guard_enabled.get()
                        on:change=move |ev| {
                            signals.liquidation_guard_enabled.set(event_target_checked(&ev))
                        }
                    />
                    <em>"实盘仅使用触发腿交易所返回的实际强平距离"</em>
                </label>
                <label class="settings-control">
                    <span>"自动退出距离 %"</span>
                    <input
                        inputmode="decimal"
                        prop:value=move || signals.liquidation_exit_distance_pct.get()
                        on:input=move |ev| {
                            signals
                                .liquidation_exit_distance_pct
                                .set(event_target_value(&ev))
                        }
                    />
                    <em>"任一腿距离小于等于此值时触发，默认 8%"</em>
                </label>
            </div>
        </div>
    }
}

fn execution_guard_fields(signals: AutoProfitCloseSignals) -> impl IntoView {
    view! {
        <div class="settings-protection-group">
            <strong>"执行保护"</strong>
            <div class="settings-section">
                <label class="settings-control">
                    <span>"平仓安全缓冲 %"</span>
                    <input
                        inputmode="decimal"
                        prop:value=move || signals.exit_buffer_pct.get()
                        on:input=move |ev| signals.exit_buffer_pct.set(event_target_value(&ev))
                    />
                    <em>"按双腿当前总名义价值计提"</em>
                </label>
                <label class="settings-control">
                    <span>"连续确认样本"</span>
                    <input
                        inputmode="numeric"
                        prop:value=move || signals.confirmation_samples.get()
                        on:input=move |ev| {
                            signals.confirmation_samples.set(event_target_value(&ev))
                        }
                    />
                    <em>"2 到 30；止盈、止损与接近强平防单帧误触，已经越线立即退出"</em>
                </label>
                <label class="settings-control">
                    <span>"触发冷却 秒"</span>
                    <input
                        inputmode="numeric"
                        prop:value=move || signals.cooldown_secs.get()
                        on:input=move |ev| signals.cooldown_secs.set(event_target_value(&ev))
                    />
                    <em>"10 到 3600；只冷却同一组配对，不延迟其他仓位保护"</em>
                </label>
            </div>
        </div>
    }
}
