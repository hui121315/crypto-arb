use crate::api::base::{store_api_auth_token, store_api_base};
use crate::api::rest::{ApiClient, ApiError};
use crate::state::action_state::ActionState;
use crate::state::context::use_global;
use futures::future::{select, Either};
use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use leptos::task::spawn_local;
use std::future::Future;

#[derive(Clone, Copy)]
pub(in crate::panels::modules::settings) struct ApiBaseValidateAction {
    pub state: RwSignal<ActionState>,
    pub submit: Callback<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(in crate::panels::modules::settings) struct ApiBaseValidationResult {
    pub(in crate::panels::modules::settings) health: shared_types::SystemHealth,
    pub(in crate::panels::modules::settings) ws_ticket_checked: bool,
}

pub(in crate::panels::modules::settings) fn save_api_base(
    api_base: RwSignal<String>,
    base_url: &str,
) {
    let normalized = store_api_base(base_url);
    apply_api_base(api_base, normalized);
}

pub(in crate::panels::modules::settings) fn save_api_auth_token(
    api_auth_token: RwSignal<String>,
    token: &str,
) -> bool {
    let normalized = store_api_auth_token(token);
    apply_api_auth_token(api_auth_token, normalized)
}

pub(in crate::panels::modules::settings) fn api_auth_configured(
    api_auth_token: RwSignal<String>,
) -> bool {
    !api_auth_token.get_untracked().trim().is_empty()
}

pub(in crate::panels::modules::settings) fn current_api_base() -> String {
    use_global().client.base_url()
}

fn apply_api_base(api_base: RwSignal<String>, normalized: String) {
    api_base.set(normalized);
}

fn apply_api_auth_token(api_auth_token: RwSignal<String>, normalized: String) -> bool {
    let configured = !normalized.is_empty();
    api_auth_token.set(normalized);
    configured
}

pub(in crate::panels::modules::settings) fn use_api_base_validate_action() -> ApiBaseValidateAction
{
    let state = RwSignal::new(ActionState::Idle);
    let api_auth_token = use_context::<crate::state::AppContext>().map(|ctx| ctx.api_auth_token);
    let submit = Callback::new(move |candidate: String| {
        if state.get_untracked().is_pending() {
            return;
        }
        let token = api_auth_token
            .map(|signal| signal.get_untracked())
            .unwrap_or_default();
        let token_configured = !token.trim().is_empty();
        let client = ApiClient::with_base_and_auth(&candidate, &token);
        let target = client.base_url();
        state.set(ActionState::pending(format!(
            "正在探测 API Base 连通性：{target}"
        )));
        spawn_local(async move {
            match validate_api_base_task(client, token_configured).await {
                Ok(result) => state.set(ActionState::succeeded(api_base_validate_success_message(
                    &target, &result,
                ))),
                Err(error) => state.set(ActionState::failed(
                    format!("API Base 验证失败：{target}"),
                    error.problem,
                )),
            }
        });
    });
    ApiBaseValidateAction { state, submit }
}

async fn validate_api_base_task(
    client: ApiClient,
    token_configured: bool,
) -> Result<ApiBaseValidationResult, ApiError> {
    let health =
        validation_request_with_timeout(client.system_health(), "/api/system/health").await?;
    require_api_version(&health)?;
    if token_configured {
        let ticket =
            validation_request_with_timeout(client.ws_ticket(), "/api/auth/ws-ticket").await?;
        if ticket.ticket.trim().is_empty() {
            return Err(ApiError::client(
                "WS_TICKET_EMPTY",
                "验证失败：/api/auth/ws-ticket 返回空 ticket",
            ));
        }
    }
    Ok(ApiBaseValidationResult {
        health,
        ws_ticket_checked: token_configured,
    })
}

pub(in crate::panels::modules::settings) fn require_api_version(
    health: &shared_types::SystemHealth,
) -> Result<(), ApiError> {
    if health.api_version.trim().is_empty() {
        return Err(ApiError::client(
            "API_VERSION_MISSING",
            "验证失败：/api/system/health 未返回 apiVersion，无法确认 CROSSLINE API 版本",
        ));
    }
    Ok(())
}

async fn validation_request_with_timeout<T>(
    request: impl Future<Output = Result<T, ApiError>>,
    path: &'static str,
) -> Result<T, ApiError> {
    let timeout = TimeoutFuture::new(10_000);
    futures::pin_mut!(request, timeout);
    match select(request, timeout).await {
        Either::Left((result, _)) => result,
        Either::Right((_, _)) => Err(ApiError::client(
            "TIMEOUT",
            format!("验证超时：目标 API Base 未在 10 秒内响应 {path}"),
        )),
    }
}

pub(in crate::panels::modules::settings) fn api_base_validate_success_message(
    target: &str,
    result: &ApiBaseValidationResult,
) -> String {
    let health = &result.health;
    let auth_probe = if result.ws_ticket_checked {
        "/api/auth/ws-ticket 探测通过"
    } else {
        "未提供 token，本次不探测 /api/auth/ws-ticket"
    };
    format!(
        "API Base 可达：{target}（/api/system/health version {}，API {}/{}，WS 频道 {}，断开 {}；{auth_probe}）",
        health.api_version,
        health.api.healthy,
        health.api.total,
        health.ws.channels,
        health.ws.disconnected.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_auth_signal_changes_without_context_lookup() {
        let owner = Owner::new();
        owner.with(|| {
            let token = RwSignal::new(String::new());

            assert!(apply_api_auth_token(token, "secret".to_owned()));
            assert_eq!(token.get_untracked(), "secret");
            assert!(!apply_api_auth_token(token, String::new()));
            assert_eq!(token.get_untracked(), "");
        });
    }

    #[test]
    fn runtime_base_signal_changes_without_context_lookup() {
        let owner = Owner::new();
        owner.with(|| {
            let api_base = RwSignal::new("http://old:8000".to_owned());

            apply_api_base(api_base, "http://new:8000".to_owned());

            assert_eq!(api_base.get_untracked(), "http://new:8000");
        });
    }
}
