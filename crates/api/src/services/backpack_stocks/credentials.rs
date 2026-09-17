use super::super::venue_credentials;
use base64::{engine::general_purpose::STANDARD, Engine};
use std::collections::BTreeMap;

pub(crate) const PROVIDER: &str = "backpack_stocks";
pub(crate) const API_KEY: &str = "BACKPACK_STOCK_API_KEY";
pub(crate) const SECRET_KEY: &str = "BACKPACK_STOCK_SECRET_KEY";
pub(super) const WINDOW: u64 = 5000;

// Deliberately neither Debug nor Serialize: credentials never enter a snapshot or journal.
pub(super) struct Credentials {
    pub(super) public: String,
    seed: [u8; 32],
}

impl Credentials {
    pub(super) fn parse(public: &str, secret: &str) -> Result<Self, String> {
        let public_raw = STANDARD
            .decode(public.trim())
            .map_err(|_| "Backpack API Key 须为 Base64 公钥")?;
        let seed: [u8; 32] = STANDARD
            .decode(secret.trim())
            .map_err(|_| "Backpack Secret Key 须为 Base64 seed")?
            .try_into()
            .map_err(|_| "Backpack Secret Key 必须是 32 字节 seed")?;
        if public_raw.as_slice()
            != common::signing::ed25519_public_key(&seed).map_err(|_| "Backpack 密钥无效")?
        {
            return Err("Backpack 公钥与 Secret Key 不匹配；请同时填写同一组 API 凭证".into());
        }
        Ok(Self {
            public: STANDARD.encode(public_raw),
            seed,
        })
    }
    pub(super) fn load() -> Result<Self, String> {
        let key = venue_credentials::secret(API_KEY).ok_or("请先配置 Backpack API Key")?;
        let secret = venue_credentials::secret(SECRET_KEY).ok_or("请先配置 Backpack Secret Key")?;
        Self::parse(&key, &secret)
    }
    pub(super) fn fingerprint(&self) -> String {
        common::signing::hmac_sha256_hex(b"backpack-stock-account-v1", self.public.as_bytes())
    }
    pub(super) fn signature(
        &self,
        instruction: &str,
        params: &BTreeMap<String, String>,
        now: i64,
    ) -> Result<String, String> {
        let fields = params
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("&");
        let message = format!(
            "instruction={instruction}{}&timestamp={now}&window={WINDOW}",
            if fields.is_empty() {
                String::new()
            } else {
                format!("&{fields}")
            }
        );
        common::signing::ed25519_sign_bytes_base64(&self.seed, message.as_bytes())
            .map_err(|_| "Backpack 请求签名失败".into())
    }
    pub(super) fn subscribe(&self, now: i64) -> Result<String, String> {
        self.subscribe_stream("account.rfqUpdate", now)
    }
    pub(super) fn subscribe_stream(&self, stream: &str, now: i64) -> Result<String, String> {
        Ok(serde_json::json!({"method":"SUBSCRIBE","params":[stream],"signature":[self.public,self.signature("subscribe",&BTreeMap::new(),now)?,now.to_string(),WINDOW.to_string()]}).to_string())
    }
}

pub(crate) fn validate_updates(updates: &[(String, String)]) -> Result<(), String> {
    let get = |key: &str| {
        updates
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
            .ok_or("Backpack 公钥和 Secret Key 需同时保存")
    };
    Credentials::parse(get(API_KEY)?, get(SECRET_KEY)?)?;
    Ok(())
}
