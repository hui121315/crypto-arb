use crate::state::load_state::LoadState;
use crate::state::module_runtime::{ModuleRuntimeState, ModuleRuntimeStatus};
use leptos::prelude::*;
use shared_types::{
    OnchainProviderCredentialStatus, OnchainProviderCredentialsResponse, SecretStorageStatus,
};

use super::data::ProviderCredentialsData;

pub(super) fn runtime_health(data: ProviderCredentialsData, provider: &str) -> ModuleRuntimeState {
    let load = data.state.get();
    let operation = if data.busy.get() || data.pending.with(Option::is_some) {
        ModuleRuntimeState {
            status: ModuleRuntimeStatus::Pending,
            problem: None,
            pending_label: Some(if data.busy.get() { "凭证操作进行中" } else { "原凭证操作待核对" }.into()),
        }
    } else {
        ModuleRuntimeState::from_problem(data.storage_problem.get().or_else(|| {
            data.target.get().filter(|target| target == provider).and_then(|_| data.problem.get())
        }).map(|message| shared_types::ApiProblem::new("PROVIDER_CREDENTIAL_OPERATION_FAILED", message)
            .with_source("frontend.provider_credentials")))
    };
    let configured = if is_configurable_provider(provider) && matches!(load, LoadState::Ready(_)) {
        match current_status_for(&load, provider) {
            Some(row) if row.ready => ModuleRuntimeState::ready(),
            Some(_) => ModuleRuntimeState::setup_required(),
            None => ModuleRuntimeState::from_problem(Some(shared_types::ApiProblem::new(
                "PROVIDER_STATUS_MISSING", "后端响应缺少当前 报价服务 的凭证状态",
            ))),
        }
    } else { ModuleRuntimeState::ready() };
    ModuleRuntimeState::combine([ModuleRuntimeState::from_load_state(&load), configured, operation])
}

pub(super) fn current_status_for(
    state: &LoadState<OnchainProviderCredentialsResponse>,
    provider: &str,
) -> Option<OnchainProviderCredentialStatus> {
    state
        .value()
        .and_then(|response| {
            response
                .providers
                .iter()
                .find(|row| row.provider == provider)
        })
        .cloned()
}

fn readiness_label(status: Option<OnchainProviderCredentialStatus>) -> String {
    match status {
        Some(status) if status.ready => "配置完整".to_owned(),
        Some(status) => format!("{}/{} 已配置", status.configured_count, status.field_count),
        None => "状态缺失".to_owned(),
    }
}

fn readiness_label_for_state(
    state: &LoadState<OnchainProviderCredentialsResponse>,
    provider: &str,
) -> String {
    match state {
        LoadState::Loading => "读取中".to_owned(),
        LoadState::Error(_) => "读取失败".to_owned(),
        LoadState::Stale { .. } => format!(
            "{} · 陈旧",
            readiness_label(current_status_for(state, provider))
        ),
        LoadState::Ready(_) => readiness_label(current_status_for(state, provider)),
    }
}

pub(super) fn selected_readiness_label(
    data: ProviderCredentialsData,
    selected_provider: RwSignal<String>,
) -> String {
    let provider = selected_provider.get();
    if !is_configurable_provider(&provider) {
        return "无需配置".to_owned();
    }
    if data.reading.get() {
        return "核对中".to_owned();
    }
    readiness_label_for_state(&data.state.get(), &provider)
}

fn readiness_class(status: Option<OnchainProviderCredentialStatus>) -> &'static str {
    match status {
        Some(status) if status.ready => "provider-credentials-readiness is-ready",
        Some(_) => "provider-credentials-readiness is-missing",
        None => "provider-credentials-readiness",
    }
}

fn readiness_class_for_state(
    state: &LoadState<OnchainProviderCredentialsResponse>,
    provider: &str,
) -> &'static str {
    match state {
        LoadState::Loading => "provider-credentials-readiness",
        LoadState::Error(_) => "provider-credentials-readiness is-error",
        LoadState::Stale { .. } => "provider-credentials-readiness is-missing",
        LoadState::Ready(_) => readiness_class(current_status_for(state, provider)),
    }
}

pub(super) fn selected_readiness_class(
    data: ProviderCredentialsData,
    selected_provider: RwSignal<String>,
) -> &'static str {
    let provider = selected_provider.get();
    if !is_configurable_provider(&provider) {
        return "provider-credentials-readiness is-ready";
    }
    if data.reading.get() {
        return "provider-credentials-readiness";
    }
    readiness_class_for_state(&data.state.get(), &provider)
}

pub(super) fn is_configurable_provider(provider: &str) -> bool {
    matches!(
        provider,
        "jupiter_swap_v2_keyed"
            | "zeroex_swap_v2"
            | "okx_dex_v6"
            | "lifi"
            | "solana_wallet_signer"
            | "evm_wallet_signer"
            | "backpack_stocks"
    )
}

fn status_line_from_status(status: Option<OnchainProviderCredentialStatus>) -> String {
    match status {
        Some(status) if status.ready => format!(
            "已保存 {} 项，无缺失必填字段；权限与实际可用性仍需对应运行请求验证。",
            status.configured_count
        ),
        Some(status) => format!(
            "还需配置：{}",
            status
                .fields
                .iter()
                .filter(|field| !field.configured)
                .map(|field| field.label.as_str())
                .collect::<Vec<_>>()
                .join("、")
        ),
        None => "后端响应缺少当前 报价服务 的凭证状态。".to_owned(),
    }
}

pub(super) fn status_line(
    state: &LoadState<OnchainProviderCredentialsResponse>,
    provider: &str,
) -> String {
    match state {
        LoadState::Loading => "正在读取后端凭证状态…".to_owned(),
        LoadState::Error(_) => "凭证状态读取失败，请确认 API 服务后重新读取。".to_owned(),
        LoadState::Stale { .. } => format!(
            "{} 当前显示上次成功读取的状态。",
            status_line_from_status(current_status_for(state, provider))
        ),
        LoadState::Ready(_) => status_line_from_status(current_status_for(state, provider)),
    }
}

pub(super) fn storage_note(data: ProviderCredentialsData) -> AnyView {
    match data.state.get() {
        LoadState::Ready(response)
        | LoadState::Stale {
            value: response, ..
        } => storage_view(response.secret_storage),
        LoadState::Loading => {
            view! { <div class="provider-credentials-storage">"安全存储状态读取中…"</div> }
                .into_any()
        }
        LoadState::Error(_) => view! {
            <div class="provider-credentials-storage is-error">
                <strong>"安全存储状态不可用"</strong>
                <span>"后端未返回凭证存储能力；页面不会回填或缓存密钥明文。"</span>
            </div>
        }
        .into_any(),
    }
}

pub(super) fn load_problem(data: ProviderCredentialsData) -> Option<AnyView> {
    let state = data.state.get();
    let problem = state.problem()?.clone();
    let prefix = if matches!(state, LoadState::Stale { .. }) {
        "凭证状态刷新失败"
    } else {
        "凭证状态读取失败"
    };
    Some(
        view! {
            <div class="provider-credentials-feedback is-danger has-action" role="alert">
                <span>{format!("{prefix}：{}", problem.message)}</span>
                <button type="button" disabled=move || data.busy.get() || data.reading.get() on:click=move |_| data.reload.run(())>
                    "重新读取"
                </button>
            </div>
        }
        .into_any(),
    )
}

fn storage_view(storage: SecretStorageStatus) -> AnyView {
    view! {
        <div class="provider-credentials-storage">
            <strong>{storage.label}</strong>
            <span>{storage.message}</span>
            {storage.warning.map(|warning| view! { <small>{warning}</small> })}
        </div>
    }
    .into_any()
}

#[cfg(test)]
mod tests {
    use shared_types::ApiProblem;

    use super::*;

    #[test]
    fn failed_load_is_never_presented_as_loading() {
        let state = LoadState::Error(ApiProblem::new("HTTP_404", "route not found"));

        assert_eq!(
            readiness_label_for_state(&state, "zeroex_swap_v2"),
            "读取失败"
        );
        assert_eq!(
            status_line(&state, "zeroex_swap_v2"),
            "凭证状态读取失败，请确认 API 服务后重新读取。"
        );
        assert_eq!(
            readiness_class_for_state(&state, "zeroex_swap_v2"),
            "provider-credentials-readiness is-error"
        );
    }

    #[test]
    fn only_the_keyed_jupiter_route_requires_credentials() {
        assert!(is_configurable_provider("jupiter_swap_v2_keyed"));
        assert!(is_configurable_provider("solana_wallet_signer"));
        assert!(!is_configurable_provider("jupiter_swap_v2"));
    }
}
