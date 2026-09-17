//! 时间戳工具。

use chrono::{DateTime, Utc};

/// 当前 Unix 毫秒时间戳。
pub fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// 当前 Unix 秒时间戳。
pub fn now_secs() -> i64 {
    Utc::now().timestamp()
}

pub fn now_utc() -> DateTime<Utc> {
    Utc::now()
}
