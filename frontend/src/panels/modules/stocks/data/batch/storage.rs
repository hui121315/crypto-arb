use super::StockBatchRequest;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct Draft {
    pub version: u8,
    pub request: StockBatchRequest,
    pub applied: Option<StockBatchRequest>,
    #[serde(default)]
    pub revision: String,
}

fn valid(request: &StockBatchRequest) -> bool {
    // Partial numeric input is a draft, not a valid submitted budget.
    request.budget_usdc.len() <= 128 && request.assets.len() <= super::STOCK_BATCH_LIMIT
        && request.assets.iter().all(|s| !s.is_empty() && s.len() <= 40
            && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'.'))
}

pub(super) fn load(key: &str) -> Result<Option<Draft>, String> {
    let Some(raw) = read(key)? else { return Ok(None); };
    let draft: Draft = serde_json::from_str(&raw).map_err(|_| "批量草稿损坏，原记录已保留。")?;
    if draft.version != 1 || draft.revision.len() > 128 || !valid(&draft.request) || draft.applied.as_ref().is_some_and(|r| !valid(r)) {
        return Err("批量草稿格式不受支持，原记录已保留。".into());
    }
    Ok(Some(draft))
}

pub(super) fn save(key: &str, draft: &Draft) -> Result<(), String> {
    if !valid(&draft.request) { return Err("批量草稿过长，尚未保存。".into()); }
    let raw = serde_json::to_string(draft).map_err(|_| "无法保存批量草稿。")?;
    if read(key)?.as_deref() != Some(&raw) { write(key, Some(&raw))?; }
    if read(key)?.as_deref() != Some(&raw) { return Err("批量草稿未完整保存。".into()); }
    Ok(())
}

pub(super) fn remove(key: &str) -> Result<(), String> {
    write(key, None)?;
    if read(key)?.is_some() { return Err("本地草稿未清除，请恢复会话存储后重试。".into()); }
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn session() -> Result<web_sys::Storage, String> {
    web_sys::window().ok_or("浏览器不可用。")?.session_storage()
        .map_err(|_| "会话存储被阻止，批量草稿未保存。")?
        .ok_or_else(|| "会话存储不可用，批量草稿未保存。".into())
}

fn read(key: &str) -> Result<Option<String>, String> {
    #[cfg(target_arch = "wasm32")]
    return session()?.get_item(key).map_err(|_| "无法读取批量草稿。".into());
    #[cfg(not(target_arch = "wasm32"))]
    { let _ = key; Ok(None) }
}

fn write(key: &str, value: Option<&str>) -> Result<(), String> {
    #[cfg(target_arch = "wasm32")]
    return match value {
        Some(value) => session()?.set_item(key, value),
        None => session()?.remove_item(key),
    }.map_err(|_| "无法写入会话存储，批量草稿未保存。".into());
    #[cfg(not(target_arch = "wasm32"))]
    { let _ = (key, value); Err("草稿恢复需要浏览器会话存储。".into()) }
}
