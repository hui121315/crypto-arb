use super::*;

#[derive(Debug)]
pub(super) struct KucoinPrivateSession {
    pub(super) ws_url: String,
    pub(super) ping_interval_ms: u64,
}

pub(super) async fn fetch_private_session(
    base_url: &str,
    path: &str,
    client_name: &'static str,
    api_key: &str,
    api_secret: &str,
    passphrase: &str,
) -> ExchangeResult<KucoinPrivateSession> {
    let timestamp = common::time::now_ms().to_string();
    let signature =
        exchange::signing::kucoin::sign(api_secret.as_bytes(), &timestamp, "POST", path, "");
    let encrypted_passphrase =
        exchange::signing::kucoin::encrypt_passphrase(api_secret.as_bytes(), passphrase);
    let http = HttpClient::new(client_name)?;
    let url = format!("{base_url}{path}");
    let response = http
        .execute_with_retry(|| {
            http.request(Method::POST, &url)
                .header("KC-API-KEY", api_key)
                .header("KC-API-SIGN", &signature)
                .header("KC-API-TIMESTAMP", &timestamp)
                .header("KC-API-PASSPHRASE", &encrypted_passphrase)
                .header("KC-API-KEY-VERSION", exchange::signing::kucoin::KEY_VERSION)
        })
        .await?;
    let body: KucoinBulletResponse = response
        .json()
        .await
        .map_err(|error| ExchangeError::Parse(format!("kucoin bullet-private json: {error}")))?;
    body.into_session()
}

#[derive(Debug, Deserialize)]
pub(super) struct KucoinBulletResponse {
    code: String,
    data: KucoinBulletData,
}

impl KucoinBulletResponse {
    pub(super) fn into_session(self) -> ExchangeResult<KucoinPrivateSession> {
        if self.code != "200000" {
            return Err(ExchangeError::Api {
                exchange: "kucoin".to_owned(),
                code: self.code,
                message: "bullet-private".to_owned(),
            });
        }
        let server = self
            .data
            .instance_servers
            .into_iter()
            .find(|server| server.protocol == "websocket")
            .ok_or_else(|| ExchangeError::Parse("kucoin private ws missing server".into()))?;
        Ok(KucoinPrivateSession {
            ws_url: format!("{}?token={}", server.endpoint, self.data.token),
            ping_interval_ms: server.ping_interval.unwrap_or(DEFAULT_KUCOIN_PING_MS),
        })
    }
}

#[derive(Debug, Deserialize)]
struct KucoinBulletData {
    token: String,
    #[serde(default, rename = "instanceServers")]
    instance_servers: Vec<KucoinBulletServer>,
}

#[derive(Debug, Deserialize)]
struct KucoinBulletServer {
    endpoint: String,
    protocol: String,
    #[serde(default, rename = "pingInterval")]
    ping_interval: Option<u64>,
}
