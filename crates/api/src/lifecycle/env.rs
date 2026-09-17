pub(super) fn var(key: &str) -> Option<String> {
    crate::services::venue_credentials::secret(key)
}

pub(super) fn pair(k1: &str, k2: &str) -> Option<(String, String)> {
    Some((var(k1)?, var(k2)?))
}

pub(super) fn triple(k1: &str, k2: &str, k3: &str) -> Option<(String, String, String)> {
    Some((var(k1)?, var(k2)?, var(k3)?))
}
