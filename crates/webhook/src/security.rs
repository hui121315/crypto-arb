use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::net::{IpAddr, SocketAddr};
use url::{Host, Url};

pub fn validate_public_https_target(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw).map_err(|error| format!("invalid webhook URL: {error}"))?;
    if url.scheme() != "https" {
        return Err("webhook URL must use HTTPS".to_owned());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("webhook URL cannot contain credentials".to_owned());
    }
    if url.fragment().is_some() {
        return Err("webhook URL cannot contain a fragment".to_owned());
    }
    let host = url
        .host_str()
        .ok_or_else(|| "webhook host is required".to_owned())?;
    if host.eq_ignore_ascii_case("localhost") || host.ends_with(".localhost") {
        return Err("localhost webhook targets are forbidden".to_owned());
    }
    match url.host() {
        Some(Host::Ipv4(ip)) => validate_public_ip(IpAddr::V4(ip))?,
        Some(Host::Ipv6(ip)) => validate_public_ip(IpAddr::V6(ip))?,
        _ => {}
    }
    Ok(url)
}

pub async fn resolve_public_target(url: &Url) -> Result<Vec<SocketAddr>, String> {
    resolve_target(url, false).await
}

pub async fn resolve_bark_target(url: &Url) -> Result<Vec<SocketAddr>, String> {
    resolve_target(url, true).await
}

async fn resolve_target(
    url: &Url,
    allow_official_bark_fake_ip: bool,
) -> Result<Vec<SocketAddr>, String> {
    let host = url
        .host_str()
        .ok_or_else(|| "webhook host is required".to_owned())?;
    let port = url.port_or_known_default().unwrap_or(443);
    let addresses = tokio::net::lookup_host((host, port))
        .await
        .map_err(|error| format!("webhook DNS resolution failed: {error}"))?
        .collect::<Vec<_>>();
    if addresses.is_empty() {
        return Err("webhook DNS resolution returned no addresses".to_owned());
    }
    for address in &addresses {
        validate_resolved_ip(host, address.ip(), allow_official_bark_fake_ip)?;
    }
    Ok(addresses)
}

pub fn validate_public_addresses(addresses: &[SocketAddr]) -> Result<(), String> {
    for address in addresses {
        validate_public_ip(address.ip())?;
    }
    Ok(())
}

pub fn signature(secret: &[u8], timestamp_ms: i64, body: &[u8]) -> Result<String, String> {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret)
        .map_err(|error| format!("webhook signing key rejected: {error}"))?;
    mac.update(timestamp_ms.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    Ok(format!("v1={}", hex::encode(mac.finalize().into_bytes())))
}

fn validate_public_ip(ip: IpAddr) -> Result<(), String> {
    let forbidden = match ip {
        IpAddr::V4(ip) => forbidden_v4(ip),
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || (ip.segments()[0] & 0xfe00) == 0xfc00
                || (ip.segments()[0] & 0xffc0) == 0xfe80
                || ip.segments()[0..2] == [0x2001, 0x0db8]
                || ip.to_ipv4_mapped().is_some_and(forbidden_v4)
        }
    };
    if forbidden {
        Err(format!("webhook target resolves to forbidden address {ip}"))
    } else {
        Ok(())
    }
}

fn validate_resolved_ip(
    host: &str,
    ip: IpAddr,
    allow_official_bark_fake_ip: bool,
) -> Result<(), String> {
    if allow_official_bark_fake_ip
        && host.eq_ignore_ascii_case("api.day.app")
        && matches!(ip, IpAddr::V4(value) if is_benchmark_v4(value))
    {
        return Ok(());
    }
    validate_public_ip(ip)
}

fn is_benchmark_v4(ip: std::net::Ipv4Addr) -> bool {
    let [a, b, _, _] = ip.octets();
    a == 198 && (b == 18 || b == 19)
}

fn forbidden_v4(ip: std::net::Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_unspecified()
        || a == 0
        || a >= 224
        || (a == 100 && (64..=127).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 0 && c == 2)
        || (a == 198 && (b == 18 || b == 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_private_and_non_https_targets() {
        assert!(validate_public_https_target("http://example.com/hook").is_err());
        assert!(validate_public_https_target("https://127.0.0.1/hook").is_err());
        assert!(validate_public_https_target("https://[::1]/hook").is_err());
        assert!(validate_public_https_target("https://100.64.0.1/hook").is_err());
        assert!(validate_public_https_target("https://192.0.2.1/hook").is_err());
        assert!(validate_public_https_target("https://[2001:db8::1]/hook").is_err());
        assert!(validate_public_https_target("https://example.com/hook").is_ok());
    }

    #[test]
    fn signature_is_stable_and_versioned() {
        let first = signature(b"secret", 123, br#"{"ok":true}"#);
        assert_eq!(first, signature(b"secret", 123, br#"{"ok":true}"#));
        assert!(first.is_ok_and(|value| value.starts_with("v1=")));
    }

    #[test]
    fn bark_fake_ip_exception_is_exact_and_keeps_private_ranges_blocked() {
        let fake_ip = IpAddr::V4(std::net::Ipv4Addr::new(198, 18, 0, 158));
        let loopback = IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);

        assert!(validate_resolved_ip("api.day.app", fake_ip, true).is_ok());
        assert!(validate_resolved_ip("example.com", fake_ip, true).is_err());
        assert!(validate_resolved_ip("api.day.app", loopback, true).is_err());
        assert!(validate_resolved_ip("api.day.app", fake_ip, false).is_err());
    }
}
