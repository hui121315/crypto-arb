//! 余额面板纯派生入口：资产分组与证据语义分别维护。

#[path = "derive/evidence.rs"]
mod evidence;
#[path = "derive/groups.rs"]
mod groups;

pub(super) use evidence::*;
pub(super) use groups::*;
