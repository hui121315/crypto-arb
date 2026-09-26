use super::PendingOperation;
use crate::api::base::{normalize_api_auth_token, normalize_api_base};
use sha2::{Digest, Sha256};

pub(super) fn key(domain: &str, base: &str, token: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(normalize_api_base(base));
    hash.update([0]);
    hash.update(normalize_api_auth_token(token));
    format!(
        "crossline.settings.pending.v1:{domain}:{:x}",
        hash.finalize()
    )
}

pub(super) fn load(key: &str, domain: &str) -> Result<Option<PendingOperation>, String> {
    let Some(raw) = read(key)? else {
        return Ok(None);
    };
    let attempt: PendingOperation = serde_json::from_str(&raw)
        .map_err(|_| "恢复记录损坏，已暂停修改；不能认定上次操作未执行。".to_owned())?;
    if !attempt.valid(domain) {
        return Err("恢复记录身份不完整，已暂停修改。".into());
    }
    Ok(Some(attempt))
}

pub(super) fn persist(
    key: &str,
    domain: &str,
    attempt: &PendingOperation,
    new: bool,
) -> Result<(), String> {
    if load(key, domain)?
        .as_ref()
        .is_some_and(|old| new || !old.same_request(attempt))
    {
        return Err("已有待核对操作，未覆盖原恢复记录。".into());
    }
    let raw = serde_json::to_string(attempt).map_err(|_| "无法生成恢复记录。".to_owned())?;
    write(key, Some(&raw))?;
    if load(key, domain)?.as_ref() != Some(attempt) {
        return Err("无法保留完整恢复记录，请求未发送。".into());
    }
    Ok(())
}

pub(super) fn resolve(key: &str, domain: &str, attempt: &PendingOperation) -> Result<(), String> {
    if load(key, domain)?
        .as_ref()
        .is_some_and(|old| !old.same_request(attempt))
    {
        return Err("恢复记录已变化，未清除其他操作。".into());
    }
    write(key, None)?;
    if read(key)?.is_some() {
        return Err("处理结果已确认，但恢复记录未清除；请重试核对。".into());
    }
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn session() -> Result<web_sys::Storage, String> {
    web_sys::window()
        .ok_or("浏览器不可用，已暂停修改。")?
        .session_storage()
        .map_err(|_| "浏览器会话存储被阻止，已暂停修改。")?
        .ok_or_else(|| "浏览器会话存储不可用，已暂停修改。".into())
}

fn read(key: &str) -> Result<Option<String>, String> {
    #[cfg(target_arch = "wasm32")]
    return session()?
        .get_item(key)
        .map_err(|_| "无法读取恢复记录，已暂停修改。".into());
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = key;
        Err("恢复需要浏览器会话存储。".into())
    }
}

fn write(key: &str, value: Option<&str>) -> Result<(), String> {
    #[cfg(target_arch = "wasm32")]
    {
        let storage = session()?;
        match value {
            Some(value) => storage.set_item(key, value),
            None => storage.remove_item(key),
        }
        .map_err(|_| "无法更新恢复记录，已暂停修改；请恢复浏览器存储后重试。".into())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (key, value);
        Err("恢复需要浏览器会话存储。".into())
    }
}
