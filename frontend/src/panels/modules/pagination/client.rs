//! 客户端分页运行态：把整集行裁成当前页窗口 + DOM 预算，并持久化页码。
//! 服务端游标分页见 `server.rs` / `list.rs`，共享滚动/游标助手见 `pagination.rs`。

use leptos::prelude::*;

use super::scroll_module_to_top;
use crate::state::module_runtime::{store_page, stored_page};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PageSlice {
    pub page: usize,
    pub page_count: usize,
    pub start: usize,
    pub end: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct TableRenderBudget {
    pub rendered_rows: usize,
    pub max_dom_rows: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TableRuntime<T> {
    pub window: PageSlice,
    pub rows: Vec<T>,
    pub budget: TableRenderBudget,
}

#[derive(Clone, Copy)]
pub(crate) struct TableRuntimeHandle<T>
where
    T: Clone + PartialEq + Send + Sync + 'static,
{
    pub current_page: RwSignal<usize>,
    pub total: Memo<usize>,
    pub runtime: Memo<TableRuntime<T>>,
}

pub(crate) fn page_slice(total: usize, page: usize, page_size: usize) -> PageSlice {
    let page_size = page_size.max(1);
    let page_count = total.div_ceil(page_size).max(1);
    let page = page.clamp(1, page_count);
    let start = if total == 0 {
        0
    } else {
        (page - 1).saturating_mul(page_size)
    };
    PageSlice {
        page,
        page_count,
        start,
        end: start.saturating_add(page_size).min(total),
        total,
    }
}

pub(crate) fn table_runtime<T>(rows: Vec<T>, page: usize, page_size: usize) -> TableRuntime<T> {
    let window = page_slice(rows.len(), page, page_size);
    let rendered_rows = window.end.saturating_sub(window.start);
    let visible_rows = rows
        .into_iter()
        .skip(window.start)
        .take(rendered_rows)
        .collect();
    TableRuntime {
        window,
        rows: visible_rows,
        budget: TableRenderBudget {
            rendered_rows,
            max_dom_rows: page_size.max(1),
        },
    }
}

pub(crate) fn reset_page_on_key(current_page: RwSignal<usize>, reset_key: Memo<String>) {
    let last_key = RwSignal::new(String::new());
    Effect::new(move |_| {
        let key = reset_key.get();
        let changed = last_key.with_untracked(|last| !last.is_empty() && last != &key);
        last_key.set(key);
        if changed {
            current_page.set(1);
        }
    });
}

pub(crate) fn use_table_runtime<T>(
    storage_key: &'static str,
    dataset_key: Memo<String>,
    rows: Memo<Vec<T>>,
    page_size: usize,
) -> TableRuntimeHandle<T>
where
    T: Clone + PartialEq + Send + Sync + 'static,
{
    let current_page = RwSignal::new(stored_page(storage_key).unwrap_or(1));
    reset_page_on_key(current_page, dataset_key);

    let total = Memo::new(move |_| rows.with(Vec::len));
    let runtime = Memo::new(move |_| table_runtime(rows.get(), current_page.get(), page_size));

    Effect::new(move |_| {
        let window = page_slice(total.get(), current_page.get(), page_size);
        if current_page.get_untracked() != window.page {
            current_page.set(window.page);
        }
    });

    Effect::new(move |_| {
        store_page(storage_key, current_page.get());
    });

    TableRuntimeHandle {
        current_page,
        total,
        runtime,
    }
}

pub(in crate::panels::modules) fn page_controls(
    total: Memo<usize>,
    current_page: RwSignal<usize>,
    page_size: usize,
) -> impl IntoView {
    Effect::new(move |_| {
        let window = page_slice(total.get(), current_page.get(), page_size);
        if current_page.get_untracked() != window.page {
            current_page.set(window.page);
        }
    });

    view! {
        <div class="table-pager">
            <button
                type="button"
                title="首页"
                disabled=move || page_slice(total.get(), current_page.get(), page_size).page <= 1
                on:click=move |_| set_page(current_page, 1)
            >"首页"</button>
            <button
                type="button"
                title="上一页"
                disabled=move || page_slice(total.get(), current_page.get(), page_size).page <= 1
                on:click=move |_| {
                    let page = page_slice(total.get(), current_page.get_untracked(), page_size).page;
                    set_page(current_page, page.saturating_sub(1).max(1));
                }
            >"上一页"</button>
            <strong>{move || page_summary(page_slice(total.get(), current_page.get(), page_size))}</strong>
            <button
                type="button"
                title="下一页"
                disabled=move || {
                    let window = page_slice(total.get(), current_page.get(), page_size);
                    window.page >= window.page_count
                }
                on:click=move |_| {
                    let window = page_slice(total.get(), current_page.get_untracked(), page_size);
                    set_page(current_page, (window.page + 1).min(window.page_count));
                }
            >"下一页"</button>
            <button
                type="button"
                title="末页"
                disabled=move || {
                    let window = page_slice(total.get(), current_page.get(), page_size);
                    window.page >= window.page_count
                }
                on:click=move |_| {
                    let page_count = page_slice(total.get(), current_page.get_untracked(), page_size).page_count;
                    set_page(current_page, page_count);
                }
            >"末页"</button>
        </div>
    }
}

fn set_page(current_page: RwSignal<usize>, page: usize) {
    current_page.set(page);
    scroll_module_to_top();
}

fn page_summary(window: PageSlice) -> String {
    if window.total == 0 {
        return "第 1 / 1 页 · 0 条".into();
    }
    format!(
        "第 {} / {} 页 · {}-{} / {}",
        window.page,
        window.page_count,
        window.start + 1,
        window.end,
        window.total
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_slice_clamps_page_and_bounds_rows() {
        let window = page_slice(95, 9, 25);
        assert_eq!(window.page, 4);
        assert_eq!(window.page_count, 4);
        assert_eq!(window.start, 75);
        assert_eq!(window.end, 95);
    }

    #[test]
    fn page_slice_handles_empty_rows() {
        assert_eq!(
            page_slice(0, 99, 25),
            PageSlice {
                page: 1,
                page_count: 1,
                start: 0,
                end: 0,
                total: 0,
            }
        );
    }

    #[test]
    fn table_runtime_keeps_rendered_rows_inside_page_budget() {
        let runtime = table_runtime((0..175).collect::<Vec<_>>(), 2, 50);

        assert_eq!(runtime.window.page, 2);
        assert_eq!(runtime.window.total, 175);
        assert_eq!(runtime.rows.len(), 50);
        assert_eq!(runtime.rows.first(), Some(&50));
        assert_eq!(runtime.rows.last(), Some(&99));
        assert_eq!(
            runtime.budget,
            TableRenderBudget {
                rendered_rows: 50,
                max_dom_rows: 50,
            }
        );
    }
}
