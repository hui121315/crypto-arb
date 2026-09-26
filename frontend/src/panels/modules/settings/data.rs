//! Settings 模块数据层。
//!
//! 为满足模块尺寸硬规则拆成聚焦子模块（生产代码各 ≤300 行）：
//! - [`actions`]：凭证保存 / 风控保存 / Kill Switch 的提交类 hooks 与幂等重放键、
//!   API Base/Token 本地写入助手。
//! - [`resources`]：各只读资源 hooks（adapters / credentials / diagnostics / action-runs…）
//!   与请求版本闸、`LoadState` 落态助手（错误保留上次快照）。
//! - [`format`]：保存成功 / 凭证校验证据的文案派生（私有读探针 fail-closed 标注）。

mod actions;
mod connection;
mod credential_maintenance;
mod format;
mod market_subscriptions;
mod resources;
mod spot_debug;
mod watchlist_alerts;
mod webhook;

#[cfg(test)]
mod tests;

pub(in crate::panels::modules::settings) use actions::*;
pub(in crate::panels::modules::settings) use credential_maintenance::*;
pub(in crate::panels::modules::settings) use crate::panels::shared::operation_journal::{
    OperationJournal as SettingsJournal, settings_recovery_panel, validate_setting_response,
};
pub(in crate::panels::modules::settings) use market_subscriptions::*;
pub(in crate::panels::modules::settings) use resources::*;
pub(in crate::panels::modules::settings) use spot_debug::*;
pub(in crate::panels::modules::settings) use watchlist_alerts::*;
pub(in crate::panels::modules::settings) use webhook::*;
