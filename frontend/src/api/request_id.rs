pub(crate) fn next_request_id() -> String {
    format!("web-{}", request_id_entropy())
}

#[cfg(target_arch = "wasm32")]
fn request_id_entropy() -> String {
    let now = js_sys::Date::now().to_bits();
    let random = js_sys::Math::random().to_bits();
    format!("{now:016x}{random:016x}")
}

#[cfg(not(target_arch = "wasm32"))]
fn request_id_entropy() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(1);
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{sequence:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_request_id_is_frontend_scoped_and_unique() {
        let first = next_request_id();
        let second = next_request_id();

        assert!(first.starts_with("web-"));
        assert!(first.len() > 8);
        assert_ne!(first, second);
    }
}
