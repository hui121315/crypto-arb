tokio::task_local! {
    static REQUEST_ID: String;
}

pub const MAX_REQUEST_ID_LEN: usize = 64;

pub async fn scope<F>(request_id: String, future: F) -> F::Output
where
    F: std::future::Future,
{
    REQUEST_ID.scope(request_id, future).await
}

pub fn current() -> Option<String> {
    REQUEST_ID.try_with(String::clone).ok()
}

pub fn new() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

pub fn normalize(value: Option<&str>) -> String {
    match value.and_then(sanitize) {
        Some(value) => value,
        None => new(),
    }
}

pub fn sanitize(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > MAX_REQUEST_ID_LEN {
        return None;
    }
    value
        .bytes()
        .all(is_request_id_byte)
        .then(|| value.to_owned())
}

fn is_request_id_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn current_is_none_outside_scope() {
        assert_eq!(current(), None);
    }

    #[tokio::test]
    async fn current_reads_scoped_request_id() {
        let seen = scope("rid-7".to_owned(), async { current() }).await;
        assert_eq!(seen, Some("rid-7".to_owned()));
    }

    #[test]
    fn sanitize_accepts_short_safe_ascii_request_id() {
        assert_eq!(
            sanitize(" rid-7_trace.2:edge "),
            Some("rid-7_trace.2:edge".to_owned())
        );
    }

    #[test]
    fn sanitize_rejects_empty_unsafe_or_overlong_request_id() {
        assert_eq!(sanitize(""), None);
        assert_eq!(sanitize("with space"), None);
        assert_eq!(sanitize("bad/header"), None);
        assert_eq!(sanitize(&"a".repeat(MAX_REQUEST_ID_LEN + 1)), None);
    }

    #[test]
    fn normalize_generates_uuid_simple_for_invalid_input() {
        let id = normalize(Some("bad header"));
        assert_eq!(id.len(), 32);
        assert!(id.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }
}
