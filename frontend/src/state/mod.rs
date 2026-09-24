//! 应用全局状态。

pub mod action_state;
pub mod arbitrage_stream;
pub mod context;
pub mod load_state;
pub mod module_runtime;
pub mod polling;
pub(crate) mod resource_polling;
pub(crate) mod section;
pub(crate) mod strategy_kinds;
pub(crate) mod table_runtime;
pub(crate) mod trading_status;
pub mod watchlist_alerts;

use crate::api::base::{stored_api_auth_token, stored_or_default_api_base};
use crate::i18n::Lang;
use gloo_storage::Storage;
use leptos::prelude::*;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy)]
pub struct AppContext {
    pub api_base: RwSignal<String>,
    pub api_auth_token: RwSignal<String>,
    pub lang: RwSignal<Lang>,
    /// 跨视图跳转请求：funding 矩阵单元格点击后写入 (symbol, exchange?)，arbitrage
    /// 视图 mount 时消费它（找到匹配机会后弹详情面板，再清空）。`None` = 无 pending 跳转。
    pub jump_symbol: RwSignal<Option<JumpRequest>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JumpRequest {
    pub symbol: String,
    pub prefer_exchange: Option<String>,
}

pub fn provide_app_context() -> AppContext {
    let api_base = RwSignal::new(stored_or_default_api_base());
    let api_auth_token = RwSignal::new(stored_api_auth_token());
    let lang = RwSignal::new(load_lang().unwrap_or(Lang::Zh));
    let jump_symbol = RwSignal::new(None);
    let ctx = AppContext {
        api_base,
        api_auth_token,
        lang,
        jump_symbol,
    };
    provide_context(ctx);
    // 单独 provide lang signal，方便子组件用 `use_lang()` 直接拿
    provide_context(lang);
    ctx
}

// LocalStorage 持久化（V1 简化版）

fn load_lang() -> Option<Lang> {
    let raw: String = gloo_storage::LocalStorage::get("lang").ok()?;
    match raw.as_str() {
        "zh" => Some(Lang::Zh),
        "en" => Some(Lang::En),
        _ => None,
    }
}

/// 监听 lang 变化并持久化。
pub fn persist_lang_on_change(ctx: AppContext) {
    Effect::new(move |_| {
        let l = ctx.lang.get();
        let v = match l {
            Lang::Zh => "zh",
            Lang::En => "en",
        };
        let _ = gloo_storage::LocalStorage::set("lang", v);
    });
}

// =====================================================================
// Toast / 通知系统
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastLevel {
    Success,
    Error,
    Info,
    Warning,
}

#[derive(Debug, Clone)]
pub struct Toast {
    pub id: u64,
    pub level: ToastLevel,
    pub message: String,
}

/// 全局 Toast 列表 signal（context 中独立提供）。
pub type Toasts = RwSignal<Vec<Toast>>;

static NEXT_TOAST_ID: AtomicU64 = AtomicU64::new(1);
const MAX_ACTIVE_TOASTS: usize = 3;

fn next_toast_id() -> u64 {
    NEXT_TOAST_ID.fetch_add(1, Ordering::Relaxed)
}

/// 在 `App` 初始化时调用，注入 `Toasts` context。
pub fn provide_toasts() -> Toasts {
    let list: Toasts = RwSignal::new(Vec::new());
    provide_context(list);
    list
}

/// 从 context 取 Toasts signal。
pub fn use_toasts() -> Toasts {
    expect_context::<Toasts>()
}

/// 推送一条 toast；默认 5s 自动移除（由 `<ToastContainer/>` 内的定时器负责）。
pub fn push_toast(level: ToastLevel, message: impl Into<String>) {
    let toasts = use_toasts();
    push_toast_to(toasts, level, message);
}

pub(crate) fn push_toast_to(toasts: Toasts, level: ToastLevel, message: impl Into<String>) {
    let message = message.into();
    let entry = Toast {
        id: next_toast_id(),
        level,
        message: message.clone(),
    };
    toasts.update(|active| {
        if active
            .iter()
            .any(|toast| toast.level == level && toast.message == message)
        {
            return;
        }
        active.push(entry);
        let overflow = active.len().saturating_sub(MAX_ACTIVE_TOASTS);
        if overflow > 0 {
            active.drain(..overflow);
        }
    });
}

/// 按 id 移除一条 toast。
pub fn dismiss_toast(id: u64) {
    let toasts = use_toasts();
    dismiss_toast_from(toasts, id);
}

pub(crate) fn dismiss_toast_from(toasts: Toasts, id: u64) {
    toasts.update(|v| v.retain(|t| t.id != id));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toast_queue_deduplicates_and_stays_bounded() {
        let owner = Owner::new();
        let toasts = owner.with(|| RwSignal::new(Vec::new()));

        push_toast_to(toasts, ToastLevel::Warning, "same");
        push_toast_to(toasts, ToastLevel::Warning, "same");
        push_toast_to(toasts, ToastLevel::Error, "two");
        push_toast_to(toasts, ToastLevel::Info, "three");
        push_toast_to(toasts, ToastLevel::Success, "four");

        let active = toasts.get_untracked();
        assert_eq!(active.len(), MAX_ACTIVE_TOASTS);
        assert_eq!(active[0].message, "two");
        assert_eq!(active[2].message, "four");
    }
}
