//! 分页运行态与控件：客户端分页（client.rs）、服务端游标分页（server.rs）、
//! `ListPage` 游标分页（list.rs）三套，共享滚动到顶 + 游标回调。

#[path = "pagination/client.rs"]
mod client;
#[path = "pagination/list.rs"]
mod list;
#[path = "pagination/server.rs"]
mod server;

use leptos::prelude::*;
use wasm_bindgen::JsCast;

pub(in crate::panels::modules) use client::page_controls;
pub(crate) use client::{use_table_runtime, TableRuntime, TableRuntimeHandle};
pub(in crate::panels::modules) use list::list_page_controls;
pub(in crate::panels::modules) use server::server_page_controls;

fn run_server_cursor(cursor: Option<String>, on_page: Callback<Option<String>>) {
    on_page.run(cursor);
    scroll_module_to_top();
}

fn scroll_module_to_top() {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let Ok(Some(content)) = document.query_selector(".mod-content") else {
        return;
    };
    if let Some(element) = content.dyn_ref::<web_sys::HtmlElement>() {
        element.set_scroll_top(0);
    }
}
