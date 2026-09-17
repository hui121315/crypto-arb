use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

const MAX_PINNED_CLIENTS: usize = 16;
const RPC_CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
const RPC_REQUEST_TIMEOUT: Duration = Duration::from_secs(12);
const RPC_TARGET_CACHE_TTL: Duration = Duration::from_secs(300);

#[derive(Clone)]
struct CachedRpcTarget {
    url: reqwest::Url,
    client: reqwest::Client,
    cached_at: Instant,
}

pub(super) fn endpoint_label(value: &str) -> Option<String> {
    reqwest::Url::parse(value)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
}

pub(super) async fn rpc_target(rpc_url: &str) -> Result<(reqwest::Url, reqwest::Client), String> {
    static TARGETS: OnceLock<dashmap::DashMap<String, CachedRpcTarget>> = OnceLock::new();
    let targets = TARGETS.get_or_init(dashmap::DashMap::new);
    let cache_key = rpc_url.trim().to_owned();
    if let Some(target) = targets.get(&cache_key) {
        if target.cached_at.elapsed() <= RPC_TARGET_CACHE_TTL {
            return Ok((target.url.clone(), target.client.clone()));
        }
    }
    targets.remove(&cache_key);
    let url = webhook::validate_public_https_target(rpc_url)
        .map_err(|_| "custom RPC endpoint failed public HTTPS validation".to_owned())?;
    let host = url
        .host_str()
        .ok_or_else(|| "custom RPC endpoint has no host".to_owned())?;
    let port = url.port_or_known_default().unwrap_or(443);
    let addresses = tokio::net::lookup_host((host, port))
        .await
        .map_err(|_| "custom RPC endpoint DNS resolution failed".to_owned())?
        .collect::<Vec<_>>();
    if addresses.is_empty() {
        return Err("custom RPC endpoint resolved no address".to_owned());
    }
    if !proxy_fake_ip_mapping(host, &addresses) {
        webhook::validate_public_addresses(&addresses)
            .map_err(|_| "custom RPC endpoint did not resolve to a public address".to_owned())?;
    }
    let address = addresses[0];
    let proxy_url = configured_https_proxy().await;
    let client = pinned_rpc_client(host, address, proxy_url.as_deref())?;
    if targets.len() >= MAX_PINNED_CLIENTS {
        targets.clear();
    }
    targets.insert(
        cache_key,
        CachedRpcTarget {
            url: url.clone(),
            client: client.clone(),
            cached_at: Instant::now(),
        },
    );
    Ok((url, client))
}

fn proxy_fake_ip_mapping(host: &str, addresses: &[SocketAddr]) -> bool {
    host.parse::<IpAddr>().is_err()
        && !addresses.is_empty()
        && addresses
            .iter()
            .all(|address| benchmark_proxy_ip(address.ip()))
}

fn benchmark_proxy_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => benchmark_proxy_v4(ip),
        IpAddr::V6(ip) => ip.to_ipv4_mapped().is_some_and(benchmark_proxy_v4),
    }
}

fn benchmark_proxy_v4(ip: Ipv4Addr) -> bool {
    let [first, second, _, _] = ip.octets();
    first == 198 && (18..=19).contains(&second)
}

fn pinned_rpc_client(
    host: &str,
    address: SocketAddr,
    proxy_url: Option<&str>,
) -> Result<reqwest::Client, String> {
    static CLIENTS: OnceLock<dashmap::DashMap<String, reqwest::Client>> = OnceLock::new();
    let clients = CLIENTS.get_or_init(dashmap::DashMap::new);
    let key = format!("{host}|{address}");
    if let Some(client) = clients.get(&key) {
        return Ok(client.clone());
    }
    if clients.len() >= MAX_PINNED_CLIENTS {
        clients.clear();
    }
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .http1_only()
        .connect_timeout(RPC_CONNECT_TIMEOUT)
        .timeout(RPC_REQUEST_TIMEOUT)
        .pool_idle_timeout(Duration::from_secs(30))
        .resolve(host, address);
    if let Some(proxy_url) = proxy_url {
        let proxy = reqwest::Proxy::https(proxy_url)
            .map_err(|_| "system HTTPS proxy could not be configured".to_owned())?;
        builder = builder.proxy(proxy);
    }
    let client = builder
        .build()
        .map_err(|_| "custom RPC client could not be built".to_owned())?;
    clients.insert(key, client.clone());
    Ok(client)
}

async fn configured_https_proxy() -> Option<String> {
    static PROXY: tokio::sync::OnceCell<Option<String>> = tokio::sync::OnceCell::const_new();
    PROXY
        .get_or_init(|| async {
            if let Some(proxy) = environment_https_proxy() {
                return Some(proxy);
            }
            #[cfg(target_os = "macos")]
            {
                tokio::task::spawn_blocking(macos_https_proxy)
                    .await
                    .ok()
                    .flatten()
            }
            #[cfg(not(target_os = "macos"))]
            {
                None
            }
        })
        .await
        .clone()
}

fn environment_https_proxy() -> Option<String> {
    ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"]
        .into_iter()
        .find_map(|key| {
            let value = std::env::var(key).ok()?;
            normalize_proxy_url(&value)
        })
}

fn normalize_proxy_url(value: &str) -> Option<String> {
    let trimmed = value.trim();
    let url = reqwest::Url::parse(trimmed).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return None;
    }
    Some(trimmed.to_owned())
}

#[cfg(target_os = "macos")]
fn macos_https_proxy() -> Option<String> {
    let output = std::process::Command::new("/usr/sbin/scutil")
        .arg("--proxy")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_macos_https_proxy(std::str::from_utf8(&output.stdout).ok()?)
}

fn parse_macos_https_proxy(output: &str) -> Option<String> {
    let mut enabled = false;
    let mut host = None;
    let mut port = None;
    for line in output.lines() {
        let Some((key, value)) = line.trim().split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "HTTPSEnable" => enabled = value == "1",
            "HTTPSProxy" if !value.is_empty() => host = Some(value),
            "HTTPSPort" => port = value.parse::<u16>().ok(),
            _ => {}
        }
    }
    if !enabled {
        return None;
    }
    let mut url = reqwest::Url::parse("http://localhost").ok()?;
    url.set_host(Some(host?)).ok()?;
    url.set_port(Some(port?)).ok()?;
    Some(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_fake_ip_requires_domain_and_entire_benchmark_range() {
        let fake = [SocketAddr::from(([198, 18, 0, 42], 443))];
        let public = [SocketAddr::from(([104, 18, 2, 1], 443))];

        assert!(proxy_fake_ip_mapping("mainnet.base.org", &fake));
        assert!(!proxy_fake_ip_mapping("198.18.0.42", &fake));
        assert!(!proxy_fake_ip_mapping("mainnet.base.org", &public));
    }

    #[test]
    fn parses_enabled_macos_https_proxy() {
        let output = r#"<dictionary> {
  HTTPEnable : 1
  HTTPPort : 1082
  HTTPProxy : 127.0.0.1
  HTTPSEnable : 1
  HTTPSPort : 1082
  HTTPSProxy : 127.0.0.1
}"#;

        assert_eq!(
            parse_macos_https_proxy(output).as_deref(),
            Some("http://127.0.0.1:1082/")
        );
    }

    #[test]
    fn ignores_disabled_macos_https_proxy() {
        let output = r#"<dictionary> {
  HTTPSEnable : 0
  HTTPSPort : 1082
  HTTPSProxy : 127.0.0.1
}"#;

        assert_eq!(parse_macos_https_proxy(output), None);
    }
}
