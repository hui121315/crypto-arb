//! 服务端游标分页（`OpportunityListPage`）：控件只消费后端签发的导航游标。
//! 客户端分页见 `client.rs`，`ListPage` 变体见 `list.rs`。

use leptos::prelude::*;
use shared_types::OpportunityListPage;

use super::run_server_cursor;

pub(in crate::panels::modules) fn server_page_controls(
    page: Memo<Option<OpportunityListPage>>,
    loading: Memo<bool>,
    on_page: Callback<Option<String>>,
) -> impl IntoView {
    let first = on_page;
    let previous = on_page;
    let next = on_page;
    let last = on_page;
    view! {
        <div class="table-pager">
            <button
                type="button"
                title="首页"
                disabled=move || loading.get() || !has_previous(page.get().as_ref())
                on:click=move |_| run_server_cursor(None, first)
            >"首页"</button>
            <button
                type="button"
                title="上一页"
                disabled=move || loading.get() || !has_previous(page.get().as_ref())
                on:click=move |_| run_server_cursor(previous_cursor(page.get().as_ref()), previous)
            >"上一页"</button>
            <strong>{move || server_page_summary(page.get().as_ref(), loading.get())}</strong>
            <button
                type="button"
                title="下一页"
                disabled=move || loading.get() || !has_next(page.get().as_ref())
                on:click=move |_| run_server_cursor(next_cursor(page.get().as_ref()), next)
            >"下一页"</button>
            <button
                type="button"
                title="末页"
                disabled=move || loading.get() || !has_last(page.get().as_ref())
                on:click=move |_| {
                    run_server_cursor(page.get().as_ref().and_then(last_cursor), last)
                }
            >"末页"</button>
        </div>
    }
}

fn server_page_summary(page: Option<&OpportunityListPage>, loading: bool) -> String {
    let Some(page) = page else {
        return if loading {
            "加载中".into()
        } else {
            "第 1 / 1 页 · 0 条".into()
        };
    };
    let prefix = if loading { "更新中 · " } else { "" };
    if page.total_rows == 0 {
        return format!("{prefix}第 1 / 1 页 · 0 条");
    }
    let start = page.start_offset.saturating_add(1);
    let end = page
        .start_offset
        .saturating_add(page.returned_count)
        .min(page.total_rows);
    format!(
        "{prefix}第 {} / {} 页 · {}-{} / {}",
        server_page_number(page),
        server_page_count(page),
        start,
        end,
        page.total_rows
    )
}

fn has_previous(page: Option<&OpportunityListPage>) -> bool {
    page.is_some_and(|page| page.start_offset > 0)
}

fn has_next(page: Option<&OpportunityListPage>) -> bool {
    page.is_some_and(|page| page.has_next_page && page.next_cursor.is_some())
}

fn has_last(page: Option<&OpportunityListPage>) -> bool {
    page.and_then(last_cursor).is_some()
}

fn previous_cursor(page: Option<&OpportunityListPage>) -> Option<String> {
    page.and_then(|page| page.previous_cursor.clone())
}

fn next_cursor(page: Option<&OpportunityListPage>) -> Option<String> {
    page.and_then(|page| page.next_cursor.clone())
}

fn last_cursor(page: &OpportunityListPage) -> Option<String> {
    page.last_cursor.clone()
}

fn server_page_number(page: &OpportunityListPage) -> usize {
    page.start_offset / page.page_size.max(1) + 1
}

fn server_page_count(page: &OpportunityListPage) -> usize {
    page.total_rows.div_ceil(page.page_size.max(1)).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_page_summary_uses_backend_window() {
        let page = server_page(25, 50, 25, 96, Some("75"));

        assert_eq!(
            server_page_summary(Some(&page), false),
            "第 3 / 4 页 · 51-75 / 96"
        );
        assert_eq!(previous_cursor(Some(&page)), Some("v1:25:scope".into()));
        assert_eq!(last_cursor(&page), Some("v1:75:scope".into()));
    }

    #[test]
    fn server_page_hides_last_on_last_window() {
        let page = server_page(25, 75, 21, 96, None);

        assert_eq!(
            server_page_summary(Some(&page), false),
            "第 4 / 4 页 · 76-96 / 96"
        );
        assert!(last_cursor(&page).is_none());
    }

    fn server_page(
        page_size: usize,
        start_offset: usize,
        returned_count: usize,
        total_rows: usize,
        next_cursor: Option<&str>,
    ) -> OpportunityListPage {
        OpportunityListPage {
            page_size,
            start_offset,
            returned_count,
            total_rows,
            has_next_page: next_cursor.is_some(),
            next_cursor: next_cursor.map(str::to_owned),
            previous_cursor: (start_offset > 0)
                .then(|| format!("v1:{}:scope", start_offset.saturating_sub(page_size))),
            last_cursor: {
                let last_offset = total_rows.saturating_sub(1) / page_size.max(1) * page_size;
                (last_offset > start_offset).then(|| format!("v1:{last_offset}:scope"))
            },
            sort_key: shared_types::OpportunityListSortKey::Score,
            snapshot_id: "snap".into(),
        }
    }
}
