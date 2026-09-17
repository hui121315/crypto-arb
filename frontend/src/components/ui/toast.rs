//! Toast 通知容器 & 单条 toast。
//!
//! - 全局右下角浮动列表，队列由状态层去重并限制数量。
//! - 每条 5s 自动消失（`gloo_timers::callback::Timeout`）。
//! - 支持手动点击 ✕ 立即关闭。
//! - 入场动画：`animate-fade-in`（Tailwind 默认有 `animate-pulse`，没有 fade-in；
//!   这里用最小化样式，依赖 `transition` 过渡即可）。

use crate::state::{dismiss_toast_from, use_toasts, Toast, ToastLevel};
use gloo_timers::callback::Timeout;
use leptos::prelude::*;

/// 容器：渲染所有 active toasts，挂载在 App 顶层。
#[component]
pub fn ToastContainer() -> impl IntoView {
    let toasts = use_toasts();

    view! {
        <div class="toast-stack" role="status" aria-live="polite">
            <For
                each=move || toasts.get()
                key=|t| t.id
                children=move |toast| view! { <ToastItem toast/> }
            />
        </div>
    }
}

#[component]
fn ToastItem(toast: Toast) -> impl IntoView {
    let id = toast.id;
    let toasts = use_toasts();
    let timeout = StoredValue::new_local(Some(Timeout::new(5_000, move || {
        dismiss_toast_from(toasts, id);
    })));
    on_cleanup(move || {
        timeout.update_value(|slot| {
            if let Some(timeout) = slot.take() {
                timeout.cancel();
            }
        });
    });

    let (icon, level_class) = toast_visual_tokens(toast.level);
    let msg = toast.message;

    view! {
        <div class=format!("toast-item {level_class}")>
            <span class="toast-icon">{icon}</span>
            <span class="toast-message">
                {msg}
            </span>
            <button
                class="toast-dismiss"
                aria-label="Dismiss"
                on:click=move |_| dismiss_toast_from(toasts, id)
            >"✕"</button>
        </div>
    }
}

fn toast_visual_tokens(level: ToastLevel) -> (&'static str, &'static str) {
    match level {
        ToastLevel::Success => ("✓", "success"),
        ToastLevel::Error => ("✕", "error"),
        ToastLevel::Warning => ("⚠", "warning"),
        ToastLevel::Info => ("ℹ", "info"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toast_visual_tokens_keep_levels_distinct() {
        assert_eq!(toast_visual_tokens(ToastLevel::Success).0, "✓");
        assert_eq!(toast_visual_tokens(ToastLevel::Error).0, "✕");
        assert_eq!(toast_visual_tokens(ToastLevel::Warning).0, "⚠");
        assert_eq!(toast_visual_tokens(ToastLevel::Info).0, "ℹ");
    }

    #[test]
    fn captured_toast_signal_dismisses_without_context_lookup() {
        let owner = Owner::new();
        let toasts = owner.with(|| {
            RwSignal::new(vec![Toast {
                id: 7,
                level: ToastLevel::Warning,
                message: "warning".into(),
            }])
        });

        dismiss_toast_from(toasts, 7);

        assert!(toasts.get_untracked().is_empty());
    }
}
