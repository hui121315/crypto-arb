use crate::api::base::{normalize_api_auth_token, normalize_api_base};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::recovery::CredentialAttempt;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecoveryRecord {
    version: u8,
    attempt: CredentialAttempt,
}

pub(super) fn scope(base: &str, token: &str) -> String {
    // Neither the backend URL nor its authorization token belongs in the record.
    let mut hash = Sha256::new();
    hash.update(normalize_api_base(base));
    hash.update([0]);
    hash.update(normalize_api_auth_token(token));
    format!("crossline.provider.pending.v1:{:x}", hash.finalize())
}

fn same_attempt(left: &CredentialAttempt, right: &CredentialAttempt) -> bool {
    left.provider == right.provider
        && left.operation == right.operation
        && left.context == right.context
        && left
            .run_id
            .as_ref()
            .is_none_or(|id| right.run_id.as_ref() == Some(id))
}

pub(super) fn load(key: &str) -> Result<Option<CredentialAttempt>, String> {
    let Some(raw) = read(key)? else {
        return Ok(None);
    };
    let record: RecoveryRecord = serde_json::from_str(&raw)
        .map_err(|_| "浏览器恢复记录格式不完整，不能认定上次操作未执行。".to_owned())?;
    let attempt = record.attempt;
    let provider_valid = matches!(
        attempt.provider.as_str(),
        "jupiter_swap_v2_keyed"
            | "zeroex_swap_v2"
            | "okx_dex_v6"
            | "lifi"
            | "solana_wallet_signer"
            | "evm_wallet_signer"
            | "backpack_stocks"
    );
    let valid_id = |id: &str| {
        !id.is_empty()
            && id.len() <= 256
            && id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b':' | b'.'))
    };
    let expected_key = format!(
        "onchain-provider-credentials-{}-{}:{}",
        attempt.operation.kind().as_str(),
        attempt.provider,
        attempt.context.request_id()
    );
    if record.version != 1
        || !provider_valid
        || !valid_id(attempt.context.request_id())
        || attempt.context.idempotency_key() != Some(expected_key.as_str())
        || attempt.run_id.as_deref().is_some_and(|id| !valid_id(id))
    {
        return Err("浏览器恢复记录身份不完整，不能重新提交凭证。".into());
    }
    Ok(Some(attempt))
}

pub(super) fn persist(key: &str, attempt: &CredentialAttempt, new: bool) -> Result<(), String> {
    if let Some(previous) = load(key)? {
        if new || !same_attempt(&previous, attempt) {
            return Err("存在另一个待核对操作，未覆盖其恢复记录。".into());
        }
    }
    let raw = serde_json::to_string(&RecoveryRecord {
        version: 1,
        attempt: attempt.clone(),
    })
    .map_err(|_| "无法生成凭证恢复记录。".to_owned())?;
    write(key, Some(&raw))?;
    if load(key)?.as_ref() != Some(attempt) {
        return Err("浏览器未保留完整恢复记录，凭证请求未发送。".into());
    }
    Ok(())
}

pub(super) fn resolve(key: &str, attempt: &CredentialAttempt) -> Result<(), String> {
    if load(key)?
        .as_ref()
        .is_some_and(|stored| !same_attempt(stored, attempt))
    {
        return Err("恢复记录已变化，未清除其他操作的记录。".into());
    }
    write(key, None)?;
    if read(key)?.is_some() {
        return Err("处理结果已确认，但浏览器尚未清除恢复记录；请重试核对。".into());
    }
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn session() -> Result<web_sys::Storage, String> {
    web_sys::window()
        .ok_or("浏览器不可用，已暂停凭证修改。")?
        .session_storage()
        .map_err(|_| "浏览器会话存储被阻止，已暂停凭证修改。")?
        .ok_or_else(|| "浏览器会话存储不可用，已暂停凭证修改。".into())
}

fn read(key: &str) -> Result<Option<String>, String> {
    #[cfg(target_arch = "wasm32")]
    return session()?
        .get_item(key)
        .map_err(|_| "无法读取恢复记录，已暂停凭证修改。".into());
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = key;
        Err("凭证恢复需要浏览器会话存储。".into())
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
        .map_err(|_| "无法更新浏览器恢复记录，已暂停凭证修改；请恢复存储后重试。".into())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (key, value);
        Err("凭证恢复需要浏览器会话存储。".into())
    }
}
