use leptos::prelude::*;
use shared_types::OnchainRpcMode;

use super::super::data::OnchainData;
use super::super::draft::OnchainConfigDraft;

pub(super) fn rpc_control(draft: OnchainConfigDraft, data: OnchainData) -> impl IntoView {
    view! {
        <section class="onchain-rpc-control" aria-label="链上 RPC 节点数据依据">
            <div class="onchain-rpc-heading">
                <div><strong>"RPC 节点"</strong><span>"身份、精度与余额读取"</span></div>
                <a href=move || rpc_docs_url(&draft.chain.get()) target="_blank" rel="noreferrer">
                    "官方说明"
                </a>
            </div>
            <div class="onchain-rpc-mode-row">
                <label class="workbench-field">
                    <span>"节点模式"</span>
                    <select
                        prop:value=move || rpc_mode_value(draft.rpc_mode.get())
                        on:change=move |event| {
                            draft.rpc_mode.set(parse_rpc_mode(&event_target_value(&event)));
                        }
                    >
                        <option value="provider_managed">"公共 RPC"</option>
                        <option value="custom">"自定义 RPC"</option>
                    </select>
                </label>
                {move || rpc_status(data)}
            </div>
            {move || rpc_editor(draft, data)}
        </section>
    }
}

pub(super) fn rpc_apply_problem(draft: OnchainConfigDraft, data: OnchainData) -> Option<String> {
    (draft.rpc_mode.get() == OnchainRpcMode::Custom
        && draft.custom_rpc_url.get().trim().is_empty()
        && !rpc_configured(data))
    .then(|| "自定义 RPC 模式需要 HTTPS RPC URL".to_owned())
}

fn rpc_editor(draft: OnchainConfigDraft, data: OnchainData) -> AnyView {
    if draft.rpc_mode.get() != OnchainRpcMode::Custom {
        return ().into_any();
    }
    let placeholder = if rpc_configured(data) {
        "已保存；留空保持当前端点"
    } else {
        "https://your-provider.example/v2/key"
    };
    view! {
        <label class="workbench-field onchain-rpc-secret">
            <span>"RPC URL"</span>
            <input
                type="password"
                autocomplete="off"
                spellcheck="false"
                placeholder=placeholder
                bind:value=draft.custom_rpc_url
            />
            <small>"仅保存在后端内存；界面和审计只显示域名。"</small>
        </label>
    }
    .into_any()
}

fn rpc_status(data: OnchainData) -> AnyView {
    let Some(snapshot) = data.state.with(|state| state.value().cloned()) else {
        return view! {
            <div class="onchain-rpc-status is-neutral">
                <strong>"读取中"</strong><span>"等待节点状态"</span>
            </div>
        }
        .into_any();
    };
    let status = snapshot.rpc_status;
    if status.mode == OnchainRpcMode::ProviderManaged {
        return view! {
            <div class="onchain-rpc-status is-neutral">
                <strong>"公共节点"</strong><span>"按需读取身份与余额"</span>
            </div>
        }
        .into_any();
    }
    let ready = status.ready;
    let tone = if ready { "is-positive" } else { "is-danger" };
    let detail = if ready && status.expected_chain_id.is_none() {
        format!(
            "{} · Solana mainnet · Slot {} · {}ms",
            status.endpoint_label.as_deref().unwrap_or("已脱敏端点"),
            status
                .block_number
                .map_or_else(|| "未知".to_owned(), |slot| slot.to_string()),
            status
                .latency_ms
                .map_or_else(|| "未知".to_owned(), |latency| latency.to_string()),
        )
    } else if ready {
        format!(
            "{} · Chain {} · Block #{} · {}ms",
            status.endpoint_label.as_deref().unwrap_or("已脱敏端点"),
            status
                .observed_chain_id
                .map_or_else(|| "未知".to_owned(), |id| id.to_string()),
            status
                .block_number
                .map_or_else(|| "未知".to_owned(), |block| block.to_string()),
            status
                .latency_ms
                .map_or_else(|| "未知".to_owned(), |latency| latency.to_string()),
        )
    } else {
        status
            .problem
            .unwrap_or_else(|| "RPC 尚未通过核对".to_owned())
    };
    let state_label = if ready { "已连接" } else { "未通过" };
    let detail_title = detail.clone();
    view! {
        <div class=format!("onchain-rpc-status {tone}") title=detail_title>
            <strong>{state_label}</strong><span>{detail}</span>
        </div>
    }
    .into_any()
}

fn rpc_configured(data: OnchainData) -> bool {
    data.state.with(|state| {
        state
            .value()
            .is_some_and(|snapshot| snapshot.rpc_status.configured)
    })
}

fn is_solana_chain(chain: &str) -> bool {
    chain.eq_ignore_ascii_case("solana")
}

fn rpc_docs_url(chain: &str) -> &'static str {
    if is_solana_chain(chain) {
        "https://solana.com/docs/rpc/http/getgenesishash"
    } else {
        "https://ethereum.org/developers/docs/apis/json-rpc/"
    }
}

const fn rpc_mode_value(mode: OnchainRpcMode) -> &'static str {
    match mode {
        OnchainRpcMode::ProviderManaged => "provider_managed",
        OnchainRpcMode::Custom => "custom",
    }
}

fn parse_rpc_mode(value: &str) -> OnchainRpcMode {
    if value == "custom" {
        OnchainRpcMode::Custom
    } else {
        OnchainRpcMode::ProviderManaged
    }
}
