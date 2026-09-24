use leptos::prelude::*;
use shared_types::{
    onchain_quote_provider_supported, onchain_quote_providers_independent,
    OnchainComparisonConfigPatch, OnchainComparisonQuality, OnchainRpcMode, ONCHAIN_CHAIN_PRESETS,
    ONCHAIN_QUOTE_PROVIDERS,
};

use crate::panels::shared::onchain_access_credentials_editor;
use crate::state::load_state::LoadState;

use super::super::data::OnchainData;
use super::super::draft::OnchainConfigDraft;
use super::super::format::{chain_label, provider_label, retry_after_label};
use super::cex_pair_select::{cex_pair_apply_problem, cex_pair_select};
use super::config_tasks::{configuration_task_tabs, OnchainConfigTask};
use super::rpc_control::{rpc_apply_problem, rpc_control};
use super::spread_alert::{spread_alert_control, spread_alert_quick_control};
use super::token_identity_fields::{identity_apply_problem, token_identity_fields};

pub(in crate::panels::modules::onchain) fn command_rail(
    draft: OnchainConfigDraft,
    data: OnchainData,
    active_task: RwSignal<OnchainConfigTask>,
) -> AnyView {
    view! {
        <aside class="onchain-command-rail" aria-label="链上套利监控配置">
            <header class="workbench-rail-header">
                <div>
                    <small class="onchain-ticket-kicker">"监控配置"</small>
                    <strong>{move || applied_pair_label(draft, data)}</strong>
                    <span>{move || applied_source_label(draft, data)}</span>
                </div>
                <div class="onchain-rail-header-tools">
                    <span
                        class=move || applied_state_class(draft, data)
                        title=move || applied_state_title(draft, data)
                    >{move || applied_state_label(draft, data)}</span>
                    {rail_monitor_tools(draft, data)}
                </div>
            </header>
            <fieldset class="onchain-config-fields" disabled=move || data.saving.get() || data.state.with(|state| state.value().is_none())>
            <div class="onchain-config-sources">{ticket_context_controls(draft, data)}</div>
            {configuration_task_tabs(active_task, draft, data)}
            <div class="onchain-command-scroll">
                <div
                    id="onchain-config-panel-market"
                    class="onchain-command-task-panel"
                    role="tabpanel"
                    aria-labelledby="onchain-config-tab-market"
                    hidden=move || active_task.get() != OnchainConfigTask::Market
                >
                    {market_configuration(draft, data, active_task)}
                </div>
                <div
                    id="onchain-config-panel-connectivity"
                    class="onchain-command-task-panel"
                    role="tabpanel"
                    aria-labelledby="onchain-config-tab-connectivity"
                    hidden=move || active_task.get() != OnchainConfigTask::Connectivity
                >
                    {connectivity_configuration(draft, data)}
                </div>
                <div
                    id="onchain-config-panel-alerts"
                    class="onchain-command-task-panel"
                    role="tabpanel"
                    aria-labelledby="onchain-config-tab-alerts"
                    hidden=move || active_task.get() != OnchainConfigTask::Alerts
                >
                    {alert_configuration(draft, data)}
                </div>
            </div>
            </fieldset>
            <div
                class="onchain-command-footer"
                aria-busy=move || data.saving.get().to_string()
            >
                {rail_problems(draft, data)}
                {rail_apply_action(draft, data)}
            </div>
        </aside>
    }
    .into_any()
}

fn applied_pair_label(draft: OnchainConfigDraft, data: OnchainData) -> String {
    data.state.with(|state| {
        state.value().map_or_else(
            || format!("{} / {}", draft.base_token.get(), draft.quote_token.get()),
            |snapshot| {
                format!(
                    "{} / {}",
                    snapshot.config.base_token, snapshot.config.quote_token
                )
            },
        )
    })
}

fn applied_source_label(draft: OnchainConfigDraft, data: OnchainData) -> String {
    data.state.with(|state| {
        state.value().map_or_else(
            || {
                format!(
                    "草稿 · {} · {} · {}",
                    chain_label(&draft.chain.get()),
                    provider_label(&draft.provider.get()),
                    draft.venue.get().to_uppercase(),
                )
            },
            |snapshot| {
                format!(
                    "已应用 · {} · {} · {}",
                    chain_label(&snapshot.config.chain),
                    provider_label(&snapshot.config.provider),
                    snapshot.config.cex_venue.to_uppercase(),
                )
            },
        )
    })
}

fn applied_state_label(draft: OnchainConfigDraft, data: OnchainData) -> &'static str {
    if data.saving.get() { return "处理中"; }
    data.state.with(|state| match state {
        LoadState::Loading => "读取中",
        LoadState::Error(_) => "读取失败",
        LoadState::Stale { .. } => "状态待确认",
        LoadState::Ready(snapshot) if !draft.matches_applied_config(&snapshot.config) => "草稿待应用",
        LoadState::Ready(snapshot) if !snapshot.config.enabled => "已暂停",
        LoadState::Ready(snapshot) => match snapshot.quality {
            OnchainComparisonQuality::Pending => "等待报价",
            OnchainComparisonQuality::Stale => "报价陈旧",
            OnchainComparisonQuality::UpstreamUnavailable => "来源异常",
            OnchainComparisonQuality::MappingInvalid => "映射待核验",
            OnchainComparisonQuality::ValuationPending => "估值待确认",
            OnchainComparisonQuality::Disabled => "状态待确认",
            _ => "监控中",
        },
    })
}

fn applied_state_class(draft: OnchainConfigDraft, data: OnchainData) -> &'static str {
    match applied_state_label(draft, data) {
        "已暂停" => "read-only-flag is-paused",
        "监控中" | "读取中" => "read-only-flag",
        _ => "read-only-flag is-pending",
    }
}

fn applied_state_title(draft: OnchainConfigDraft, data: OnchainData) -> &'static str {
    match applied_state_label(draft, data) {
        "草稿待应用" => "左侧输入与当前运行配置不同；右侧仍按已应用配置更新",
        "监控中" => "配置已启用；双源状态与实际报价时效见市场区",
        "已暂停" => "当前配置已保存，但双源监控已暂停",
        "读取中" => "正在读取当前运行配置",
        "处理中" => "正在等待后端确认，暂不能再次修改配置",
        "读取失败" | "状态待确认" => "无法确认最新运行状态；旧快照仅供参考，不可据此构建",
        "报价陈旧" => "报价已超过有效期，等待新的双源数据",
        "来源异常" => "报价来源异常，等待恢复或手动重读",
        _ => "当前运行状态",
    }
}

fn market_configuration(
    draft: OnchainConfigDraft,
    data: OnchainData,
    active_task: RwSignal<OnchainConfigTask>,
) -> AnyView {
    let open_alert_rules = Callback::new(move |()| active_task.set(OnchainConfigTask::Alerts));
    view! {
        <section class="onchain-rail-section onchain-market-config">
            <header class="onchain-rail-section-heading">
                <div><strong>"市场配置"</strong><span>"资产、交易对与统一资金"</span></div>
            </header>
            <div class="onchain-market-workspace">
                {market_identity_editor(draft, data)}
                <div class="onchain-market-trade-stack">
                    {cex_pair_select(draft, data)}
                    {market_budget_fields(draft)}
                </div>
                <div class="onchain-market-alert-row">
                    {spread_alert_quick_control(draft, data, open_alert_rules)}
                </div>
                <div class="onchain-market-advanced-row">
                    {advanced_fields(draft)}
                </div>
            </div>
        </section>
    }
    .into_any()
}

fn ticket_context_controls(draft: OnchainConfigDraft, data: OnchainData) -> AnyView {
    view! {
        <div class="onchain-ticket-context" aria-label="链上来源">
            <label>
                <span>"链"</span>
                <select
                    prop:value=move || draft.chain.get()
                    on:change=move |event| {
                        draft.apply_chain_preset(&event_target_value(&event));
                        data.reset_token_states();
                    }
                >
                    {ONCHAIN_CHAIN_PRESETS.iter().map(|preset| {
                        view! { <option value=preset.id>{preset.label}</option> }
                    }).collect_view()}
                </select>
            </label>
            <label>
                <span>"Provider"</span>
                <select
                    prop:value=move || draft.provider.get()
                    on:change=move |event| draft.apply_provider(&event_target_value(&event))
                >
                    {move || ONCHAIN_QUOTE_PROVIDERS.iter()
                        .filter(|provider| onchain_quote_provider_supported(provider.id, &draft.chain.get()))
                        .map(|provider| {
                            let provider_id = provider.id;
                            view! {
                                <option
                                    value=provider_id
                                    prop:selected=move || draft.provider.get().eq_ignore_ascii_case(provider_id)
                                >{provider.label}</option>
                            }
                        })
                        .collect_view()}
                </select>
            </label>
        </div>
        {dex_cross_control(draft)}
        {cross_chain_control(draft, data)}
    }
    .into_any()
}

fn dex_cross_control(draft: OnchainConfigDraft) -> impl IntoView {
    let has_peer = move || {
        ONCHAIN_QUOTE_PROVIDERS.iter().any(|provider| {
            onchain_quote_provider_supported(provider.id, &draft.chain.get())
                && onchain_quote_providers_independent(&draft.provider.get(), provider.id)
        })
    };
    view! {
        <div class="onchain-dex-cross-control" aria-label="链上对链上报价">
            <label class="onchain-dex-cross-toggle">
                <input
                    type="checkbox"
                    aria-label="启用链上对链上监控"
                    prop:checked=move || draft.dex_compare_enabled.get()
                    disabled=move || !has_peer()
                    on:change=move |event| {
                        draft.set_dex_compare_enabled(event_target_checked(&event));
                    }
                />
                <span class="onchain-switch" aria-hidden="true"></span>
                <span>
                    <strong>"DEX 对比"</strong>
                    <small>{move || if has_peer() { "同链第二报价源" } else { "暂无独立第二源" }}</small>
                </span>
            </label>
            <label class="onchain-dex-peer-select">
                <span class="sr-only">"第二 DEX Provider"</span>
                <select
                    aria-label="第二 DEX Provider"
                    disabled=move || !draft.dex_compare_enabled.get() || !has_peer()
                    prop:value=move || draft.peer_provider.get()
                    on:change=move |event| draft.apply_peer_provider(&event_target_value(&event))
                >
                    {move || ONCHAIN_QUOTE_PROVIDERS.iter()
                        .filter(|provider| {
                            onchain_quote_provider_supported(provider.id, &draft.chain.get())
                                && onchain_quote_providers_independent(&draft.provider.get(), provider.id)
                        })
                        .map(|provider| view! {
                            <option value=provider.id>{provider.label}</option>
                        })
                        .collect_view()}
                </select>
            </label>
        </div>
    }
}

#[derive(Clone, PartialEq, Eq)]
struct CrossChainTarget {
    item_id: String,
    label: String,
}

fn cross_chain_targets(data: OnchainData, source_chain: &str) -> Vec<CrossChainTarget> {
    data.state.with(|state| {
        state.value().map_or_else(Vec::new, |snapshot| {
            snapshot
                .batch
                .items
                .iter()
                .filter(|item| !item.config.chain.eq_ignore_ascii_case(source_chain))
                .map(|item| {
                    let wallet = if item.config.wallet_address.trim().is_empty() {
                        " · 缺钱包"
                    } else {
                        ""
                    };
                    CrossChainTarget {
                        item_id: item.item_id.clone(),
                        label: format!(
                            "{} · {}/{} · {}{}",
                            chain_label(&item.config.chain),
                            item.config.base_token,
                            item.config.quote_token,
                            provider_label(&item.config.provider),
                            wallet,
                        ),
                    }
                })
                .collect()
        })
    })
}

fn cross_chain_control(draft: OnchainConfigDraft, data: OnchainData) -> impl IntoView {
    let targets = Memo::new(move |_| cross_chain_targets(data, &draft.chain.get()));
    let selected_available = move || targets.with(|targets| targets.iter().any(|target| target.item_id == draft.cross_chain_peer_item_id.get()));
    let missing_selected = move || !draft.cross_chain_peer_item_id.get().is_empty() && !selected_available();
    view! {
        <div class="onchain-cross-chain-control" aria-label="跨链闭环监控">
            <label class="onchain-cross-chain-toggle">
                <input
                    type="checkbox"
                    aria-label="启用跨链闭环监控"
                    prop:checked=move || draft.cross_chain_enabled.get()
                    disabled=move || data.saving.get() || (!draft.cross_chain_enabled.get() && !selected_available())
                    on:change=move |event| {
                        draft.set_cross_chain_enabled(event_target_checked(&event));
                    }
                />
                <span class="onchain-switch" aria-hidden="true"></span>
                <span>
                    <strong>"跨链闭环"</strong>
                    <small>{move || if missing_selected() { "目标不可用 · 重新选择或关闭" }
                        else if selected_available() { "LI.FI 往返最小到账" }
                        else if targets.with(Vec::is_empty) { "先加入另一条链市场" }
                        else { "先选择目标链市场" }}</small>
                </span>
            </label>
            <label class="onchain-cross-chain-peer-select">
                <span class="sr-only">"目标链市场"</span>
                <select
                    aria-label="跨链目标市场"
                    disabled=move || data.saving.get() || (targets.with(Vec::is_empty) && draft.cross_chain_peer_item_id.get().is_empty())
                    prop:value=move || draft.cross_chain_peer_item_id.get()
                    on:change=move |event| draft.apply_cross_chain_peer(&event_target_value(&event))
                >
                    <option value="" prop:selected=move || draft.cross_chain_peer_item_id.get().is_empty()>"选择目标链市场"</option>
                    {move || missing_selected().then(|| view! {
                        <option value=move || draft.cross_chain_peer_item_id.get() disabled prop:selected=true>"原目标已移除或与当前链相同"</option>
                    })}
                    {move || targets.get()
                        .into_iter()
                        .map(move |target| {
                            let item_id = target.item_id.clone();
                            view! { <option value=target.item_id
                                prop:selected=move || draft.cross_chain_peer_item_id.get() == item_id>{target.label}</option> }
                        })
                        .collect_view()}
                </select>
            </label>
        </div>
    }
}

fn market_identity_editor(draft: OnchainConfigDraft, data: OnchainData) -> impl IntoView {
    view! {
        <section class="onchain-market-identity-editor" aria-label="链上资产">
            <header>
                <span>
                    <small>"链上资产"</small>
                    <strong>{move || market_identity_summary(draft)}</strong>
                </span>
                <em class=move || market_identity_state_class(draft, data)>
                    {move || market_identity_state_label(draft, data)}
                </em>
            </header>
            <div class="onchain-market-identity-body">
                {token_identity_fields(draft, data)}
            </div>
        </section>
    }
}

fn market_identity_summary(draft: OnchainConfigDraft) -> String {
    let token_with_precision = |token: String, decimals: String| {
        let token = token.trim().to_ascii_uppercase();
        let token = if token.is_empty() {
            "--"
        } else {
            token.as_str()
        };
        let decimals = decimals.trim();
        if decimals.is_empty() {
            format!("{token} · 精度读取中")
        } else {
            format!("{token} · {decimals}位")
        }
    };
    format!(
        "{} / {}",
        token_with_precision(draft.base_token.get(), draft.base_decimals.get()),
        token_with_precision(draft.quote_token.get(), draft.quote_decimals.get()),
    )
}

fn market_identity_state_label(draft: OnchainConfigDraft, data: OnchainData) -> &'static str {
    data.state.with(|state| match state.value() {
        None => "读取中",
        Some(snapshot)
            if draft.matches_applied_config(&snapshot.config)
                && snapshot.config.base_identity_resolved
                && snapshot.config.quote_identity_resolved =>
        {
            "已识别"
        }
        Some(_) => "待核验",
    })
}

fn market_identity_state_class(draft: OnchainConfigDraft, data: OnchainData) -> &'static str {
    match market_identity_state_label(draft, data) {
        "已识别" => "is-positive",
        "待核验" => "is-warning",
        _ => "is-neutral",
    }
}

fn connectivity_configuration(draft: OnchainConfigDraft, data: OnchainData) -> AnyView {
    let signer_provider =
        RwSignal::new(chain_signer_provider(&draft.chain.get_untracked()).to_owned());
    let bridge_provider = RwSignal::new("lifi".to_owned());
    Effect::new(move |_| {
        signer_provider.set(chain_signer_provider(&draft.chain.get()).to_owned());
    });
    view! {
        <section class="onchain-rail-section onchain-connectivity-config">
            <header class="onchain-rail-section-heading">
                <div><strong>"执行接入"</strong><span>"RPC → 公开地址 → 当前链签名器 → 报价 API"</span></div>
            </header>
            {access_overview(data)}
            <div class="onchain-connectivity-body">
                {rpc_control(draft, data)}
                {wallet_access_control(draft, data)}
                {onchain_access_credentials_editor(
                    signer_provider,
                    draft.provider,
                    bridge_provider,
                    draft.cross_chain_enabled,
                )}
            </div>
        </section>
    }
    .into_any()
}

fn wallet_access_control(draft: OnchainConfigDraft, data: OnchainData) -> impl IntoView {
    view! {
        <details
            class="onchain-wallet-access"
            aria-label="链上公开钱包地址"
            open=move || wallet_access_state(draft, data).0 != "签名匹配"
        >
            <summary>
                <div>
                    <small>"01 · 公开身份"</small>
                    <strong>{move || format!("{} 钱包地址", chain_label(&draft.chain.get()))}</strong>
                </div>
                <span class=move || wallet_access_state(draft, data).1>
                    {move || wallet_access_state(draft, data).0}
                </span>
            </summary>
            <div class="onchain-wallet-access-body">
                <label class="workbench-field">
                    <span>"公开地址"</span>
                    <input
                        type="text"
                        autocomplete="off"
                        spellcheck="false"
                        placeholder=move || wallet_address_placeholder(&draft.chain.get())
                        aria-invalid=move || (!draft.wallet_address.get().trim().is_empty()
                            && !wallet_address_shape_valid(&draft.chain.get(), &draft.wallet_address.get()))
                            .to_string()
                        bind:value=draft.wallet_address
                    />
                </label>
                <p>"只填公开地址读取 Base、Quote 与 Gas 余额；私钥在签名器中单独保存。"</p>
                {move || wallet_access_problem(draft, data).map(|problem| view! {
                    <div class="onchain-wallet-access-problem">{problem}</div>
                })}
            </div>
        </details>
    }
}

fn access_overview(data: OnchainData) -> impl IntoView {
    view! {
        <dl class="onchain-access-overview" aria-label="执行接入就绪概览">
            {move || access_facts(data).into_iter().map(access_fact_view).collect_view()}
        </dl>
    }
}

#[derive(Clone, Copy)]
struct AccessFact {
    label: &'static str,
    value: &'static str,
    tone: &'static str,
}

fn access_facts(data: OnchainData) -> [AccessFact; 3] {
    data.state.with(|state| {
        let Some(snapshot) = state.value() else {
            return [
                access_fact("RPC", "读取中", "is-neutral"),
                access_fact("公开地址", "读取中", "is-neutral"),
                access_fact("签名器", "读取中", "is-neutral"),
            ];
        };
        let rpc_ready = snapshot.rpc_status.ready
            || snapshot.rpc_status.mode == OnchainRpcMode::ProviderManaged;
        [
            access_fact(
                "RPC",
                if rpc_ready { "可用" } else { "未通过" },
                if rpc_ready {
                    "is-positive"
                } else {
                    "is-danger"
                },
            ),
            access_fact(
                "公开地址",
                if snapshot.execution_readiness.wallet_address_configured {
                    "已配置"
                } else {
                    "待填写"
                },
                if snapshot.execution_readiness.wallet_address_configured {
                    "is-positive"
                } else {
                    "is-warning"
                },
            ),
            access_fact(
                "签名器",
                if snapshot.execution_readiness.chain_submission_ready {
                    "已匹配"
                } else {
                    "待接入"
                },
                if snapshot.execution_readiness.chain_submission_ready {
                    "is-positive"
                } else {
                    "is-warning"
                },
            ),
        ]
    })
}

const fn access_fact(label: &'static str, value: &'static str, tone: &'static str) -> AccessFact {
    AccessFact { label, value, tone }
}

fn access_fact_view(fact: AccessFact) -> impl IntoView {
    view! {
        <div class=fact.tone><dt>{fact.label}</dt><dd>{fact.value}</dd></div>
    }
}

fn wallet_access_state(
    draft: OnchainConfigDraft,
    data: OnchainData,
) -> (&'static str, &'static str) {
    let chain = draft.chain.get();
    let address = draft.wallet_address.get();
    let address = address.trim();
    if address.is_empty() {
        return ("未填写", "is-warning");
    }
    if !wallet_address_shape_valid(&chain, address) {
        return ("格式有误", "is-danger");
    }
    data.state.with(|state| {
        let Some(snapshot) = state.value() else {
            return ("读取中", "is-neutral");
        };
        if !snapshot
            .config
            .wallet_address
            .trim()
            .eq_ignore_ascii_case(address)
            || !snapshot.config.chain.eq_ignore_ascii_case(&chain)
        {
            return ("待应用", "is-warning");
        }
        if snapshot.execution_readiness.chain_submission_ready {
            ("签名匹配", "is-positive")
        } else {
            ("待签名器", "is-warning")
        }
    })
}

fn wallet_access_problem(draft: OnchainConfigDraft, data: OnchainData) -> Option<String> {
    let chain = draft.chain.get();
    let address = draft.wallet_address.get();
    let address = address.trim();
    if address.is_empty() {
        return Some("填写公开地址后，系统才会读取余额并核对当前链签名器。".to_owned());
    }
    if !wallet_address_shape_valid(&chain, address) {
        return Some(if chain.eq_ignore_ascii_case("solana") {
            "Solana 地址应为 32 字节公钥的 base58 文本。".to_owned()
        } else {
            "EVM 地址应为 0x 开头的 40 位十六进制地址。".to_owned()
        });
    }
    data.state.with(|state| {
        let snapshot = state.value()?;
        if !snapshot
            .config
            .wallet_address
            .trim()
            .eq_ignore_ascii_case(address)
            || !snapshot.config.chain.eq_ignore_ascii_case(&chain)
        {
            return Some("地址尚未应用；当前监控与余额读取仍使用已保存配置。".to_owned());
        }
        (!snapshot.execution_readiness.chain_submission_ready)
            .then(|| {
                snapshot
                    .execution_readiness
                    .global_blockers
                    .first()
                    .cloned()
            })
            .flatten()
    })
}

fn wallet_address_placeholder(chain: &str) -> &'static str {
    if chain.eq_ignore_ascii_case("solana") {
        "Solana base58 公开地址"
    } else {
        "0x… EVM 公开地址"
    }
}

fn wallet_address_shape_valid(chain: &str, address: &str) -> bool {
    let address = address.trim();
    if chain.eq_ignore_ascii_case("solana") {
        (32..=44).contains(&address.len())
            && address.bytes().all(|byte| {
                matches!(byte,
                    b'1'..=b'9' | b'A'..=b'H' | b'J'..=b'N' | b'P'..=b'Z'
                        | b'a'..=b'k' | b'm'..=b'z')
            })
    } else {
        address.len() == 42
            && address.starts_with("0x")
            && address[2..].bytes().all(|byte| byte.is_ascii_hexdigit())
    }
}

fn chain_signer_provider(chain: &str) -> &'static str {
    if chain.eq_ignore_ascii_case("solana") {
        "solana_wallet_signer"
    } else {
        "evm_wallet_signer"
    }
}

fn alert_configuration(draft: OnchainConfigDraft, data: OnchainData) -> AnyView {
    view! {
        <section class="onchain-rail-section onchain-alert-section">
            <header class="onchain-rail-section-heading">
                <div><strong>"提醒规则"</strong><span>"费后机会与原始价差分开判断"</span></div>
            </header>
            {spread_alert_control(draft, data)}
        </section>
    }
    .into_any()
}

fn market_budget_fields(draft: OnchainConfigDraft) -> impl IntoView {
    view! {
        <div class="onchain-market-budget" aria-label="监控资金与执行目标">
            <header>
                <strong>"对比与执行规模"</strong>
                <span>"先比较同一资金；构建时读取 100 档盘口"</span>
            </header>
            <div class="workbench-field-pair">
                <label
                    class="workbench-field"
                    title="系统用同一资金换出 Base，再计算双向费后结果。"
                >
                    <span>{move || format!("统一对比资金 ({})", draft.quote_token.get())}</span>
                    <input type="number" min="0" step="any" bind:value=draft.quote_amount />
                </label>
                <label
                    class="workbench-field"
                    title="最优档只预览；构建时按此目标核验完整深度。"
                >
                    <span>"构建目标成交额 (USD)"</span>
                    <input type="number" min="1" step="10" bind:value=draft.min_liquidity />
                </label>
            </div>
        </div>
    }
}

fn advanced_fields(draft: OnchainConfigDraft) -> impl IntoView {
    view! {
        <details class="onchain-advanced-config">
            <summary>"高级身份、成本与时效"</summary>
            <div class="onchain-advanced-grid">
                <label class="workbench-field">
                    <span>"链上路由"</span>
                    <input bind:value=draft.pool_or_route />
                </label>
                <div class="workbench-field-pair">
                    <label class="workbench-field"><span>"CEX 费率 (%)"</span><input type="number" min="0" max="100" step="0.001" bind:value=draft.cex_fee /></label>
                    <label class="workbench-field"><span>"滑点缓冲 (%)"</span><input type="number" min="0" max="100" step="0.001" bind:value=draft.slippage /></label>
                </div>
                <div class="workbench-field-pair">
                    <label class="workbench-field"><span>"Gas (USD)"</span><input type="number" min="0" step="0.001" bind:value=draft.gas_usd /></label>
                    <label class="workbench-field">
                        <span>"链上报价最大时效 (ms)"</span>
                        <input type="number" min="1000" max="60000" step="1000" bind:value=draft.max_age />
                    </label>
                </div>
            </div>
        </details>
    }
}

fn rail_monitor_tools(draft: OnchainConfigDraft, data: OnchainData) -> AnyView {
    view! {
            <div class="onchain-rail-monitor-tools" role="toolbar" aria-label="监控工具">
                <Show when=move || manual_retry_visible(data)>
                    <button
                        class="row-action onchain-rail-tool"
                        type="button"
                        disabled=move || refresh_action_blocked(data)
                        title=move || refresh_action_title(data)
                        aria-label=move || refresh_action_label(data)
                        on:click=move |_| data.refresh.run(())
                    >"↻"</button>
                </Show>
                <Show when=move || monitor_enabled(data)>
                <button
                    class="row-action onchain-rail-tool onchain-batch-add"
                    type="button"
                    disabled=move || batch_action_blocked(draft, data)
                    title=move || batch_action_title(draft, data)
                    aria-label=move || batch_action_label(draft, data)
                    on:click=move |_| {
                        draft.normalize_tokens();
                        data.add_batch.run(draft.patch());
                    }
                >"+"</button>
                <button
                    class="row-action onchain-rail-tool onchain-monitor-stop"
                    type="button"
                    disabled=move || data.saving.get()
                    title="暂停链上 HTTP 询价与 CEX WS 监控；保留当前配置"
                    aria-label="暂停当前监控"
                    on:click=move |_| set_monitor(data, false)
                >"Ⅱ"</button>
                </Show>
            </div>
    }
    .into_any()
}

fn rail_apply_action(draft: OnchainConfigDraft, data: OnchainData) -> AnyView {
    view! {
        <Show when=move || apply_action_visible(draft, data)>
            <div class="workbench-primary-actions onchain-apply-actions">
                <button
                    class=move || if apply_action_blocked(draft, data) {
                        "row-action onchain-apply-action"
                    } else {
                        "workbench-primary onchain-apply-action"
                    }
                    type="button"
                    disabled=move || apply_action_blocked(draft, data)
                    title=move || apply_action_title(draft, data)
                    on:click=move |_| apply_pair(draft, data)
                >
                    {move || apply_action_label(data)}
                </button>
            </div>
        </Show>
    }
    .into_any()
}

fn apply_action_visible(draft: OnchainConfigDraft, data: OnchainData) -> bool {
    !monitor_enabled(data) || !draft_matches_applied(draft, data) || data.saving.get()
}

fn apply_action_blocked(draft: OnchainConfigDraft, data: OnchainData) -> bool {
    data.saving.get()
        || data.state.with(|state| state.value().is_none())
        || apply_problem(draft, data).is_some()
        || (monitor_enabled(data) && draft_matches_applied(draft, data))
}

fn apply_action_title(draft: OnchainConfigDraft, data: OnchainData) -> String {
    if data.saving.get() {
        "等待当前配置操作完成".to_owned()
    } else if monitor_enabled(data) && draft_matches_applied(draft, data) {
        "当前配置已应用，右侧正在实时监控".to_owned()
    } else {
        apply_problem(draft, data).unwrap_or_else(|| {
            if monitor_enabled(data) {
                "应用左侧草稿并读取新的双边比较".to_owned()
            } else {
                "应用左侧草稿并直接开始只读套利监控".to_owned()
            }
        })
    }
}

fn apply_action_label(data: OnchainData) -> &'static str {
    if data.saving.get() {
        "保存中…"
    } else if monitor_enabled(data) {
        "应用变更"
    } else {
        "应用并开始监控"
    }
}

fn refresh_action_blocked(data: OnchainData) -> bool {
    data.saving.get() || provider_retry_delay(data).is_some()
}

fn refresh_action_label(data: OnchainData) -> String {
    if data.saving.get() {
        "重试中…".to_owned()
    } else if let Some(delay) = provider_retry_delay(data) {
        retry_after_label(delay)
    } else {
        "立即重试".to_owned()
    }
}

fn refresh_action_title(data: OnchainData) -> String {
    if data.saving.get() {
        "等待当前配置操作完成".to_owned()
    } else if let Some(delay) = provider_retry_delay(data) {
        format!(
            "Provider 正在退避；系统将在{}，无需连续点击",
            retry_after_label(delay)
        )
    } else if monitor_enabled(data) {
        "只刷新右侧已应用配置，不会应用左侧草稿".to_owned()
    } else {
        "重新读取已保存配置和运行状态，不应用当前草稿".to_owned()
    }
}

fn provider_retry_delay(data: OnchainData) -> Option<i64> {
    data.state.with(|state| {
        state
            .value()
            .and_then(|snapshot| snapshot.provider_retry_after_ms)
            .filter(|delay| *delay > 0)
    })
}

fn manual_retry_visible(data: OnchainData) -> bool {
    data.state.with(|state| {
        state.problem().is_some() || state.value().is_some_and(|snapshot| {
            matches!(
                snapshot.quality,
                OnchainComparisonQuality::Pending
                    | OnchainComparisonQuality::Stale
                    | OnchainComparisonQuality::UpstreamUnavailable
            )
        })
    })
}

fn batch_action_blocked(draft: OnchainConfigDraft, data: OnchainData) -> bool {
    data.saving.get()
        || !monitor_enabled(data)
        || apply_problem(draft, data).is_some()
        || batch_action_state(draft, data) == BatchActionState::Current
}

fn batch_action_label(draft: OnchainConfigDraft, data: OnchainData) -> &'static str {
    if data.saving.get() {
        "处理中…"
    } else {
        match batch_action_state(draft, data) {
            BatchActionState::New => "加入批量监控",
            BatchActionState::Update => "更新批量监控",
            BatchActionState::Current => "已在批量监控",
        }
    }
}

fn batch_action_title(draft: OnchainConfigDraft, data: OnchainData) -> String {
    if data.saving.get() {
        "等待当前配置操作完成".to_owned()
    } else if !monitor_enabled(data) {
        "先启用套利监控，再加入需要持续观察的组合".to_owned()
    } else if batch_action_state(draft, data) == BatchActionState::Current {
        "当前组合与参数已在批量队列中持续监控".to_owned()
    } else if batch_action_state(draft, data) == BatchActionState::Update {
        "用当前资金、成本、时效与提醒参数更新队列中的同一组合".to_owned()
    } else {
        apply_problem(draft, data).unwrap_or_else(|| {
            if draft_matches_applied(draft, data) {
                "将当前运行配置加入只读批量监控".to_owned()
            } else {
                "将左侧草稿直接加入批量队列；右侧当前比较保持不变".to_owned()
            }
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BatchActionState {
    New,
    Update,
    Current,
}

fn batch_action_state(draft: OnchainConfigDraft, data: OnchainData) -> BatchActionState {
    data.state.with(|state| {
        let Some(snapshot) = state.value() else {
            return BatchActionState::New;
        };
        snapshot
            .batch
            .items
            .iter()
            .find(|item| batch_identity_matches(draft, &item.config))
            .map_or(BatchActionState::New, |item| {
                if draft.matches_batch_config(&item.config) {
                    BatchActionState::Current
                } else {
                    BatchActionState::Update
                }
            })
    })
}

fn batch_identity_matches(
    draft: OnchainConfigDraft,
    config: &shared_types::OnchainComparisonConfig,
) -> bool {
    [
        (draft.chain.get(), config.chain.as_str()),
        (draft.provider.get(), config.provider.as_str()),
        (draft.base_mint.get(), config.base_mint.as_str()),
        (draft.quote_mint.get(), config.quote_mint.as_str()),
        (draft.venue.get(), config.cex_venue.as_str()),
        (draft.symbol.get(), config.cex_symbol.as_str()),
    ]
    .into_iter()
    .all(|(draft_value, config_value)| draft_value.trim().eq_ignore_ascii_case(config_value.trim()))
}

fn rail_problems(draft: OnchainConfigDraft, data: OnchainData) -> AnyView {
    view! {
        {move || apply_problem(draft, data).map(|problem| {
            let title = problem.clone();
            view! {
                <details class="onchain-apply-blocker" role="status">
                    <summary title=title>
                        <strong>"暂不可应用"</strong>
                        <span>{problem}</span>
                        <small>"详情"</small>
                    </summary>
                    <p>"右侧继续显示上一次已应用配置；修正草稿后再应用。"</p>
                </details>
            }
        })}
        {move || data.action_problem.get().map(|problem| view! {
            <div class="onchain-config-problem" role="alert"><strong>"操作未完成"</strong><span>{problem}</span></div>
        })}
    }
    .into_any()
}

fn apply_problem(draft: OnchainConfigDraft, data: OnchainData) -> Option<String> {
    market_input_problem(draft)
        .or_else(|| identity_apply_problem(draft, data))
        .or_else(|| cex_pair_apply_problem(draft, data))
        .or_else(|| rpc_apply_problem(draft, data))
        .or_else(|| cross_chain_apply_problem(draft, data))
}

fn cross_chain_apply_problem(draft: OnchainConfigDraft, data: OnchainData) -> Option<String> {
    (draft.cross_chain_enabled.get()
        && !cross_chain_targets(data, &draft.chain.get()).iter()
            .any(|target| target.item_id == draft.cross_chain_peer_item_id.get()))
        .then(|| "跨链目标不可用，请选择另一条链的市场或关闭跨链监控".to_owned())
}

fn market_input_problem(draft: OnchainConfigDraft) -> Option<String> {
    if draft
        .quote_amount_raw()
        .and_then(|value| value.parse::<u128>().ok())
        .filter(|value| *value > 0)
        .is_none()
    {
        return Some("统一对比资金必须是大于 0、且不超过代币精度的正常金额".to_owned());
    }

    let target = draft.min_liquidity.get();
    if target
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value > 0.0)
        .is_none()
    {
        return Some("构建目标成交额必须是大于 0 的美元金额".to_owned());
    }
    None
}

fn apply_pair(draft: OnchainConfigDraft, data: OnchainData) {
    draft.normalize_tokens();
    data.update
        .run(config_patch_for_apply(draft, monitor_enabled(data)));
}

fn config_patch_for_apply(
    draft: OnchainConfigDraft,
    monitor_already_enabled: bool,
) -> OnchainComparisonConfigPatch {
    let mut patch = draft.patch();
    if !monitor_already_enabled {
        patch.enabled = Some(true);
    }
    patch
}

fn set_monitor(data: OnchainData, enabled: bool) {
    data.update.run(OnchainComparisonConfigPatch {
        enabled: Some(enabled),
        ..OnchainComparisonConfigPatch::default()
    });
}

fn monitor_enabled(data: OnchainData) -> bool {
    data.state.with(|state| {
        state
            .value()
            .is_some_and(|snapshot| snapshot.config.enabled)
    })
}

fn draft_matches_applied(draft: OnchainConfigDraft, data: OnchainData) -> bool {
    data.state.with(|state| {
        state
            .value()
            .is_some_and(|snapshot| draft.matches_applied_config(&snapshot.config))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_apply_starts_read_only_monitoring_in_the_same_action() {
        Owner::new().with(|| {
            let draft =
                OnchainConfigDraft::from_config(&shared_types::OnchainComparisonConfig::default());

            assert_eq!(config_patch_for_apply(draft, false).enabled, Some(true));
            assert_eq!(config_patch_for_apply(draft, true).enabled, None);
        });
    }

    #[test]
    fn paused_monitor_keeps_the_saved_config_unchanged() {
        let patch = OnchainComparisonConfigPatch {
            enabled: Some(false),
            ..OnchainComparisonConfigPatch::default()
        };

        assert_eq!(patch.enabled, Some(false));
        assert!(patch.chain.is_none());
        assert!(patch.cex_symbol.is_none());
    }

    #[test]
    fn market_inputs_reject_invalid_comparison_and_depth_amounts() {
        Owner::new().with(|| {
            let draft =
                OnchainConfigDraft::from_config(&shared_types::OnchainComparisonConfig::default());
            assert_eq!(market_input_problem(draft), None);

            draft.quote_amount.set("not-a-number".to_owned());
            assert!(market_input_problem(draft)
                .is_some_and(|problem| problem.contains("正常金额")));

            draft.quote_amount.set("100".to_owned());
            draft.min_liquidity.set("0".to_owned());
            assert!(market_input_problem(draft)
                .is_some_and(|problem| problem.contains("大于 0")));
        });
    }

    #[test]
    fn execution_signer_follows_the_selected_chain_family() {
        assert_eq!(chain_signer_provider("solana"), "solana_wallet_signer");
        assert_eq!(chain_signer_provider("base"), "evm_wallet_signer");
        assert_eq!(chain_signer_provider("arbitrum"), "evm_wallet_signer");
    }

    #[test]
    fn wallet_address_shape_follows_the_selected_chain_family() {
        assert!(wallet_address_shape_valid(
            "solana",
            "11111111111111111111111111111111"
        ));
        assert!(!wallet_address_shape_valid(
            "solana",
            "0OIl-not-base58-wallet-address"
        ));
        assert!(wallet_address_shape_valid(
            "base",
            "0x2222222222222222222222222222222222222222"
        ));
        assert!(!wallet_address_shape_valid(
            "base",
            "0x22222222222222222222222222222222222222zz"
        ));
    }

    #[test]
    fn identity_summary_surfaces_auto_read_precision() {
        Owner::new().with(|| {
            let draft =
                OnchainConfigDraft::from_config(&shared_types::OnchainComparisonConfig::default());
            draft.base_token.set("pups".to_owned());
            draft.base_decimals.set("9".to_owned());
            draft.quote_token.set("usdc".to_owned());
            draft.quote_decimals.set("6".to_owned());

            assert_eq!(market_identity_summary(draft), "PUPS · 9位 / USDC · 6位");

            draft.base_decimals.set(String::new());
            assert_eq!(
                market_identity_summary(draft),
                "PUPS · 精度读取中 / USDC · 6位"
            );
        });
    }
}
