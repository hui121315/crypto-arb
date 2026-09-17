use shared_types::IndexCompositionStatus;
use shared_types::{IndexCompositionQuality, IndexCompositionSnapshot};

pub(super) fn status_label(status: IndexCompositionStatus) -> &'static str {
    match status {
        IndexCompositionStatus::Verified => "已验证",
        IndexCompositionStatus::HiddenPrice => "隐藏价格",
        IndexCompositionStatus::Mismatch => "成分不一致",
        IndexCompositionStatus::Unverified => "未验证",
        IndexCompositionStatus::Unsupported => "不支持",
        IndexCompositionStatus::Stale => "已过期",
        IndexCompositionStatus::Error => "错误",
    }
}

pub(super) fn quality_label(quality: IndexCompositionQuality) -> &'static str {
    match quality {
        IndexCompositionQuality::Verified => "已验证",
        IndexCompositionQuality::Unverified => "未验证",
        IndexCompositionQuality::Unsupported => "不支持",
        IndexCompositionQuality::Stale => "已过期",
        IndexCompositionQuality::Error => "错误",
    }
}

pub(super) fn payload_evidence_label(snapshot: &IndexCompositionSnapshot) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(url) = snapshot.source_url.as_deref() {
        parts.push(format!("官方来源 {url}"));
    }
    if let Some(hash) = snapshot.payload_sha256.as_deref() {
        parts.push(format!("sha256 {}", &hash[..hash.len().min(12)]));
    }
    if let Some(version) = snapshot.schema_version.as_deref() {
        parts.push(format!("schema {version}"));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

pub(super) fn snapshot_health(
    snapshot: &IndexCompositionSnapshot,
    total: usize,
    shown: usize,
) -> String {
    let mut parts = vec![snapshot.source.clone()];
    if let Some(freshness_ms) = snapshot.freshness_ms {
        parts.push(duration_label(freshness_ms));
    } else {
        parts.push("新鲜度未知".into());
    }
    if snapshot.received_at_ms > 0 {
        parts.push(format!(
            "取得于 {}",
            checked_at_label(snapshot.received_at_ms)
        ));
    }
    if total > shown {
        parts.push(format!("成分 {total} 项 · 首屏 {shown} 项"));
    } else if total > 0 {
        parts.push(format!("成分 {total} 项"));
    }
    if let Some(retry_after_ms) = snapshot.retry_after_ms {
        parts.push(format!("{}后重试", duration_label(retry_after_ms)));
    }
    parts.join(" · ")
}

#[cfg(target_arch = "wasm32")]
fn checked_at_label(ms: i64) -> String {
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ms as f64));
    String::from(date.to_iso_string())
}

#[cfg(not(target_arch = "wasm32"))]
fn checked_at_label(ms: i64) -> String {
    format!("{ms}ms")
}

fn duration_label(ms: i64) -> String {
    let ms = ms.max(0);
    if ms < 1_000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1_000.0)
    } else {
        format!("{:.1}m", ms as f64 / 60_000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_health_uses_chinese_unknown_freshness() {
        let snapshot = IndexCompositionSnapshot {
            venue: "hyperliquid:xyz".into(),
            symbol: "MU".into(),
            index_id: "MU".into(),
            components: Vec::new(),
            quality: IndexCompositionQuality::Unverified,
            source: "local-cache".into(),
            received_at_ms: 1_000,
            freshness_ms: None,
            error: None,
            retry_after_ms: Some(2_000),
            source_url: None,
            payload_sha256: None,
            schema_version: None,
        };

        assert_eq!(
            snapshot_health(&snapshot, 12, 8),
            "local-cache · 新鲜度未知 · 取得于 1000ms · 成分 12 项 · 首屏 8 项 · 2.0s后重试"
        );
    }

    #[test]
    fn duration_label_clamps_negative_values() {
        assert_eq!(duration_label(-1), "0ms");
        assert_eq!(duration_label(1_500), "1.5s");
        assert_eq!(duration_label(120_000), "2.0m");
    }
}
