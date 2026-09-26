//! Toast 通知容器 & 单条 toast。
//!
//! - 全局右下角浮动列表，队列由状态层去重并限制数量。
//! - 空闲 5s 后关闭；鼠标、键盘阅读或展开详情时暂停计时。
//! - 长内容可滚动，关闭按钮始终保留在内容外。

use crate::state::{dismiss_toast_from, use_toasts, Toast, ToastLevel};
use gloo_timers::callback::Timeout;
use leptos::prelude::*;

/// 容器：渲染所有 active toasts，挂载在 App 顶层。
#[component]
pub fn ToastContainer() -> impl IntoView {
    let toasts = use_toasts();

    view! {
        <div class="toast-stack" role="status" aria-live="polite" aria-label="通知">
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
    let hovering = RwSignal::new(false);
    let focused = RwSignal::new(false);
    let expanded = RwSignal::new(false);
    let timeout = StoredValue::new_local(None::<Timeout>);
    Effect::new(move |_| {
        let reading = hovering.get() || focused.get() || expanded.get();
        timeout.update_value(|slot| {
            if let Some(timer) = slot.take() {
                timer.cancel();
            }
            if !reading {
                *slot = Some(Timeout::new(5_000, move || dismiss_toast_from(toasts, id)));
            }
        });
    });
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
        <div class=format!("toast-item {level_class}")
            on:mouseenter=move |_| hovering.set(true)
            on:mouseleave=move |_| hovering.set(false)
            on:focusin=move |_| focused.set(true)
            on:focusout=move |_| focused.set(false)
        >
            <span class="toast-icon" aria-hidden="true">{icon}</span>
            <div class="toast-content" tabindex="0" aria-label="通知内容">
                <p class="toast-message">{msg}</p>
                {toast.details.map(|details| view! {
                    <details class="toast-details" on:toggle=move |event| {
                        expanded.set(event_target::<web_sys::Element>(&event).has_attribute("open"));
                    }>
                        <summary>"技术详情"</summary>
                        <pre>{details}</pre>
                    </details>
                })}
            </div>
            <button
                class="toast-dismiss"
                aria-label="关闭通知"
                title="关闭通知"
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
                details: None,
            }])
        });

        dismiss_toast_from(toasts, 7);

        assert!(toasts.get_untracked().is_empty());
    }
}
