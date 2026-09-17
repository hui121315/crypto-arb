use crate::panels::workstation::{ModuleId, WorkspaceRuntime};
use crate::state::module_runtime::{ModuleRuntimeState, ModuleRuntimeStatus};
use leptos::prelude::*;

#[derive(Clone, Copy)]
pub(crate) struct ModuleInfo {
    pub id: ModuleId,
    pub title: &'static str,
}

pub(crate) const MODULES: &[ModuleInfo] = &[
    ModuleInfo {
        id: ModuleId::Positions,
        title: "持仓/风控",
    },
    ModuleInfo {
        id: ModuleId::Futures,
        title: "期货套利",
    },
    ModuleInfo {
        id: ModuleId::Opportunities,
        title: "机会扫描",
    },
    ModuleInfo {
        id: ModuleId::GateCrossEx,
        title: "CrossEx",
    },
    ModuleInfo {
        id: ModuleId::Onchain,
        title: "链上套利",
    },
    ModuleInfo {
        id: ModuleId::Stocks,
        title: "股票套利",
    },
    ModuleInfo {
        id: ModuleId::Automation,
        title: "自动化",
    },
    ModuleInfo {
        id: ModuleId::Execution,
        title: "对冲执行",
    },
    ModuleInfo {
        id: ModuleId::Review,
        title: "复盘",
    },
    ModuleInfo {
        id: ModuleId::Settings,
        title: "设置",
    },
];

#[component]
pub(in crate::panels) fn ModuleNav(runtime: WorkspaceRuntime) -> impl IntoView {
    let active_module = runtime.active();
    view! {
        <nav class="module-tabs" aria-label="功能模块">
            {MODULES.iter().copied().map(|module| {
                let state = Memo::new(move |_| runtime.module_runtime_state(module.id));
                view! {
                    <button
                        type="button"
                        class=move || nav_button_class(active_module.get(), module.id)
                        aria-current=move || nav_aria_current(active_module.get(), module.id)
                        aria-label=move || state.with(|state| nav_aria_label(active_module.get(), module, state))
                        title=move || state.with(|state| nav_button_title(active_module.get(), module.id, state))
                        data-module=module.id.slug()
                        data-runtime-state=move || state.with(|state| nav_runtime_slug(active_module.get(), module.id, state))
                        on:click=move |_| {
                            if !nav_is_current(active_module.get_untracked(), module.id) {
                                active_module.set(module.id);
                            }
                        }
                    >
                        <span
                            class="module-tab-status"
                            data-state=move || state.with(|state| nav_runtime_slug(active_module.get(), module.id, state))
                            aria-hidden="true"
                        ></span>
                        <span class="module-tab-copy">
                            <strong>{module.title}</strong>
                            <small>{move || state.with(|state| nav_runtime_label(active_module.get(), module.id, state).unwrap_or_default())}</small>
                        </span>
                        <span class="tab-rail" aria-hidden="true"></span>
                    </button>
                }
            }).collect_view()}
        </nav>
    }
}

fn nav_is_current(active: ModuleId, module: ModuleId) -> bool {
    active == module
}

fn nav_button_class(active: ModuleId, module: ModuleId) -> &'static str {
    if nav_is_current(active, module) {
        "active"
    } else {
        ""
    }
}

fn nav_aria_current(active: ModuleId, module: ModuleId) -> Option<&'static str> {
    nav_is_current(active, module).then_some("page")
}

fn nav_button_title(active: ModuleId, module: ModuleId, state: &ModuleRuntimeState) -> String {
    let action = if nav_is_current(active, module) {
        "当前模块"
    } else {
        "切换模块"
    };
    nav_runtime_label(active, module, state)
        .map(|label| format!("{action} · {label}"))
        .unwrap_or_else(|| action.to_owned())
}

fn nav_aria_label(active: ModuleId, module: ModuleInfo, state: &ModuleRuntimeState) -> String {
    let prefix = if nav_is_current(active, module.id) {
        "当前模块"
    } else {
        "切换到"
    };
    nav_runtime_label(active, module.id, state)
        .map(|label| format!("{prefix}{}，{label}", module.title))
        .unwrap_or_else(|| format!("{prefix}{}", module.title))
}

fn nav_runtime_slug(
    active: ModuleId,
    module: ModuleId,
    state: &ModuleRuntimeState,
) -> &'static str {
    if should_expose_runtime_state(active, module, state) {
        state.slug()
    } else {
        "idle"
    }
}

fn nav_runtime_label(
    active: ModuleId,
    module: ModuleId,
    state: &ModuleRuntimeState,
) -> Option<String> {
    should_expose_runtime_state(active, module, state).then(|| runtime_state_label(state))
}

fn should_expose_runtime_state(
    active: ModuleId,
    module: ModuleId,
    state: &ModuleRuntimeState,
) -> bool {
    nav_is_current(active, module)
        || !matches!(
            state.status,
            ModuleRuntimeStatus::Loading | ModuleRuntimeStatus::Ready
        )
}

fn runtime_state_label(state: &ModuleRuntimeState) -> String {
    state
        .pending_label
        .clone()
        .unwrap_or_else(|| state.label().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nav_marks_only_active_module_as_current() {
        let active = ModuleId::Futures;
        let futures = ModuleInfo {
            id: ModuleId::Futures,
            title: "期货套利",
        };
        let settings = ModuleInfo {
            id: ModuleId::Settings,
            title: "设置",
        };
        let ready = ModuleRuntimeState::ready();

        assert_eq!(nav_button_class(active, futures.id), "active");
        assert_eq!(nav_aria_current(active, futures.id), Some("page"));
        assert_eq!(
            nav_button_title(active, futures.id, &ready),
            "当前模块 · 正常"
        );
        assert_eq!(
            nav_aria_label(active, futures, &ready),
            "当前模块期货套利，正常"
        );

        assert_eq!(nav_button_class(active, settings.id), "");
        assert_eq!(nav_aria_current(active, settings.id), None);
        assert_eq!(nav_button_title(active, settings.id, &ready), "切换模块");
        assert_eq!(nav_aria_label(active, settings, &ready), "切换到设置");
        assert_eq!(nav_runtime_slug(active, settings.id, &ready), "idle");

        let loading = ModuleRuntimeState::from_load_state(
            &crate::state::load_state::LoadState::<u8>::Loading,
        );
        assert_eq!(nav_aria_label(active, settings, &loading), "切换到设置");
        assert_eq!(nav_runtime_slug(active, settings.id, &loading), "idle");

        let stale =
            ModuleRuntimeState::from_load_state(&crate::state::load_state::LoadState::Stale {
                value: 1_u8,
                problem: shared_types::ApiProblem::new("STALE", "old snapshot"),
            });
        assert_eq!(
            nav_aria_label(active, settings, &stale),
            "切换到设置，数据已过期"
        );
        assert_eq!(nav_runtime_slug(active, settings.id, &stale), "stale");
    }
}
