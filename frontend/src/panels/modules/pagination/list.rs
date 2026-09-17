//! 服务端游标分页（`ListPage` 变体，按 `limit` 与 `has_more` 口径）：控件 + 页码/游标推导。
//! 客户端分页见 `client.rs`，`OpportunityListPage` 变体见 `server.rs`。

use leptos::prelude::*;
use shared_types::ListPage;

use super::run_server_cursor;

pub(in crate::panels::modules) fn list_page_controls(
    page: Memo<Option<ListPage>>,
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
                disabled=move || loading.get() || !list_has_previous(page.get().as_ref())
                on:click=move |_| run_server_cursor(None, first)
            >"首页"</button>
            <button
                type="button"
                title="上一页"
                disabled=move || loading.get() || !list_has_previous(page.get().as_ref())
                on:click=move |_| {
                    run_server_cursor(list_previous_cursor(page.get().as_ref()), previous)
                }
            >"上一页"</button>
            <strong>{move || list_page_summary(page.get().as_ref(), loading.get())}</strong>
            <button
                type="button"
                title="下一页"
                disabled=move || loading.get() || !list_has_next(page.get().as_ref())
                on:click=move |_| run_server_cursor(list_next_cursor(page.get().as_ref()), next)
            >"下一页"</button>
            <button
                type="button"
                title="末页"
                disabled=move || loading.get() || !list_has_last(page.get().as_ref())
                on:click=move |_| {
                    run_server_cursor(page.get().as_ref().and_then(list_last_cursor), last)
                }
            >"末页"</button>
        </div>
    }
}

fn list_page_summary(page: Option<&ListPage>, loading: bool) -> String {
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
        list_page_number(page),
        list_page_count(page),
        start,
        end,
        page.total_rows
    )
}

fn list_has_previous(page: Option<&ListPage>) -> bool {
    page.is_some_and(|page| page.start_offset > 0)
}

fn list_has_next(page: Option<&ListPage>) -> bool {
    page.is_some_and(|page| page.has_more && page.next_cursor.is_some())
}

fn list_has_last(page: Option<&ListPage>) -> bool {
    page.and_then(list_last_cursor).is_some()
}

fn list_previous_cursor(page: Option<&ListPage>) -> Option<String> {
    let page = page?;
    if page.start_offset == 0 {
        return None;
    }
    if let Some(cursor) = page.previous_cursor.as_ref() {
        return Some(cursor.clone());
    }
    Some(
        page.start_offset
            .saturating_sub(page.limit.max(1))
            .to_string(),
    )
}

fn list_next_cursor(page: Option<&ListPage>) -> Option<String> {
    page.and_then(|page| page.next_cursor.clone())
}

fn list_last_cursor(page: &ListPage) -> Option<String> {
    if let Some(cursor) = page.last_cursor.as_ref() {
        return Some(cursor.clone());
    }
    if page.total_rows == 0 {
        return None;
    }
    let limit = page.limit.max(1);
    let last_offset = page.total_rows.saturating_sub(1) / limit * limit;
    (last_offset > page.start_offset).then(|| last_offset.to_string())
}

fn list_page_number(page: &ListPage) -> usize {
    page.start_offset / page.limit.max(1) + 1
}

fn list_page_count(page: &ListPage) -> usize {
    page.total_rows.div_ceil(page.limit.max(1)).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_page_summary_uses_backend_window() {
        let page = list_page(50, 50, 50, 125, Some("100"));

        assert_eq!(
            list_page_summary(Some(&page), false),
            "第 2 / 3 页 · 51-100 / 125"
        );
        assert_eq!(
            list_previous_cursor(Some(&page)),
            Some("rv1:0:review-snapshot".into())
        );
        assert_eq!(list_next_cursor(Some(&page)), Some("100".into()));
        assert_eq!(
            list_last_cursor(&page),
            Some("rv1:100:review-snapshot".into())
        );
    }

    #[test]
    fn list_page_hides_last_on_last_window() {
        let page = list_page(50, 100, 25, 125, None);

        assert_eq!(
            list_page_summary(Some(&page), false),
            "第 3 / 3 页 · 101-125 / 125"
        );
        assert!(list_last_cursor(&page).is_none());
    }

    fn list_page(
        limit: usize,
        start_offset: usize,
        returned_count: usize,
        total_rows: usize,
        next_cursor: Option<&str>,
    ) -> ListPage {
        ListPage {
            limit,
            max_limit: 100,
            start_offset,
            returned_count,
            total_rows,
            has_more: next_cursor.is_some(),
            previous_cursor: (start_offset > 0).then(|| "rv1:0:review-snapshot".to_owned()),
            next_cursor: next_cursor.map(str::to_owned),
            last_cursor: next_cursor
                .is_some()
                .then(|| "rv1:100:review-snapshot".to_owned()),
            snapshot_id: Some("review-snapshot".to_owned()),
        }
    }
}
