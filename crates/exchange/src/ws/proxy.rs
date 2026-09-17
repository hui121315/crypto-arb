use std::sync::LazyLock;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tracing::debug;

const MAX_CONNECT_RESPONSE_BYTES: usize = 16 * 1024;
const MACOS_SYSTEM_PROXY_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, PartialEq, Eq)]
struct HttpProxy {
    host: String,
    port: u16,
    fail_closed: bool,
}

pub(super) struct ProxyTunnel {
    stream: TcpStream,
    fail_closed: bool,
}

impl ProxyTunnel {
    pub(super) fn fail_closed(&self) -> bool {
        self.fail_closed
    }

    pub(super) fn into_stream(self) -> TcpStream {
        self.stream
    }
}

pub(super) async fn connect_tunnel(target_url: &str) -> Result<Option<ProxyTunnel>, String> {
    static PROXY: LazyLock<Option<HttpProxy>> = LazyLock::new(configured_https_proxy);
    let Some(proxy) = PROXY.as_ref() else {
        return Ok(None);
    };
    let target = url::Url::parse(target_url)
        .map_err(|_| "websocket proxy target URL is invalid".to_owned())?;
    let host = target
        .host_str()
        .ok_or_else(|| "websocket proxy target has no host".to_owned())?;
    if !matches!(host, "ws.kraken.com" | "futures.kraken.com") {
        return Ok(None);
    }
    let port = target
        .port_or_known_default()
        .ok_or_else(|| "websocket proxy target has no port".to_owned())?;
    let connect = connect_http_tunnel(proxy, host, port);
    if proxy.fail_closed {
        return connect.await.map(Some);
    }
    match tokio::time::timeout(MACOS_SYSTEM_PROXY_CONNECT_TIMEOUT, connect).await {
        Ok(Ok(tunnel)) => Ok(Some(tunnel)),
        Ok(Err(error)) => {
            debug!(%error, "macOS system proxy tunnel failed; using system network route");
            Ok(None)
        }
        Err(_) => {
            debug!(
                timeout_ms = MACOS_SYSTEM_PROXY_CONNECT_TIMEOUT.as_millis() as u64,
                "macOS system proxy tunnel timed out; using system network route"
            );
            Ok(None)
        }
    }
}

async fn connect_http_tunnel(
    proxy: &HttpProxy,
    host: &str,
    port: u16,
) -> Result<ProxyTunnel, String> {
    let mut stream = TcpStream::connect((proxy.host.as_str(), proxy.port))
        .await
        .map_err(|error| format!("websocket proxy connection failed: {error}"))?;
    stream
        .set_nodelay(true)
        .map_err(|error| format!("websocket proxy TCP setup failed: {error}"))?;
    let authority = format!("{host}:{port}");
    stream
        .write_all(format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n\r\n").as_bytes())
        .await
        .map_err(|error| format!("websocket proxy CONNECT send failed: {error}"))?;
    read_connect_response(&mut stream).await?;
    Ok(ProxyTunnel {
        stream,
        fail_closed: proxy.fail_closed,
    })
}

async fn read_connect_response(stream: &mut TcpStream) -> Result<(), String> {
    let mut response = Vec::with_capacity(512);
    let mut chunk = [0_u8; 512];
    while !response.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream
            .read(&mut chunk)
            .await
            .map_err(|error| format!("websocket proxy CONNECT read failed: {error}"))?;
        if read == 0 {
            return Err("websocket proxy closed before CONNECT completed".to_owned());
        }
        response.extend_from_slice(&chunk[..read]);
        if response.len() > MAX_CONNECT_RESPONSE_BYTES {
            return Err("websocket proxy CONNECT response is too large".to_owned());
        }
    }
    let status = response
        .split(|byte| *byte == b'\n')
        .next()
        .and_then(|line| std::str::from_utf8(line).ok())
        .map(str::trim)
        .unwrap_or_default();
    if status.starts_with("HTTP/1.1 200") || status.starts_with("HTTP/1.0 200") {
        return Ok(());
    }
    Err(format!(
        "websocket proxy CONNECT rejected: {}",
        if status.is_empty() {
            "invalid response"
        } else {
            status
        }
    ))
}

fn configured_https_proxy() -> Option<HttpProxy> {
    environment_https_proxy().or_else(macos_https_proxy)
}

fn environment_https_proxy() -> Option<HttpProxy> {
    ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"]
        .into_iter()
        .find_map(|key| {
            std::env::var(key)
                .ok()
                .and_then(|value| parse_proxy_url(&value))
        })
}

fn parse_proxy_url(value: &str) -> Option<HttpProxy> {
    let url = url::Url::parse(value.trim()).ok()?;
    if url.scheme() != "http" || !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    Some(HttpProxy {
        host: url.host_str()?.to_owned(),
        port: url.port_or_known_default()?,
        fail_closed: true,
    })
}

#[cfg(target_os = "macos")]
fn macos_https_proxy() -> Option<HttpProxy> {
    let output = std::process::Command::new("/usr/sbin/scutil")
        .arg("--proxy")
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| parse_macos_https_proxy(std::str::from_utf8(&output.stdout).ok()?))?
}

#[cfg(not(target_os = "macos"))]
fn macos_https_proxy() -> Option<HttpProxy> {
    None
}

#[cfg(target_os = "macos")]
fn parse_macos_https_proxy(output: &str) -> Option<HttpProxy> {
    let mut enabled = false;
    let mut host = None;
    let mut port = None;
    for line in output.lines() {
        let Some((key, value)) = line.trim().split_once(':') else {
            continue;
        };
        match key.trim() {
            "HTTPSEnable" => enabled = value.trim() == "1",
            "HTTPSProxy" if !value.trim().is_empty() => host = Some(value.trim().to_owned()),
            "HTTPSPort" => port = value.trim().parse::<u16>().ok(),
            _ => {}
        }
    }
    if !enabled {
        return None;
    }
    Some(HttpProxy {
        host: host?,
        port: port?,
        fail_closed: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_http_connect_proxy_without_credentials() {
        assert_eq!(
            parse_proxy_url("http://127.0.0.1:1082"),
            Some(HttpProxy {
                host: "127.0.0.1".to_owned(),
                port: 1082,
                fail_closed: true,
            })
        );
        assert_eq!(parse_proxy_url("socks5://127.0.0.1:1080"), None);
        assert_eq!(parse_proxy_url("http://user:secret@127.0.0.1:1082"), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn parses_enabled_macos_https_proxy() {
        let output = r#"<dictionary> {
  HTTPSEnable : 1
  HTTPSPort : 1082
  HTTPSProxy : 127.0.0.1
}"#;
        assert_eq!(
            parse_macos_https_proxy(output),
            Some(HttpProxy {
                host: "127.0.0.1".to_owned(),
                port: 1082,
                fail_closed: false,
            })
        );
    }
}
