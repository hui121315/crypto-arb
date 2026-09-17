use super::*;

#[test]
fn secret_storage_health_labels_are_fail_closed() {
    assert_eq!(
        secret_storage_health_label(SecretStorageHealth::Ready),
        "可用"
    );
    assert_eq!(
        secret_storage_health_label(SecretStorageHealth::Degraded),
        "降级"
    );
    assert_eq!(
        secret_storage_health_label(SecretStorageHealth::Unavailable),
        "不可用"
    );
    assert_eq!(
        secret_storage_health_label(SecretStorageHealth::Unknown),
        "状态未知"
    );
}

#[test]
fn only_ready_secret_storage_is_green() {
    assert_eq!(
        secret_storage_health_class(SecretStorageHealth::Ready),
        "status-pill ready"
    );
    assert_eq!(
        secret_storage_health_class(SecretStorageHealth::Degraded),
        "status-pill pending"
    );
    assert_eq!(
        secret_storage_health_class(SecretStorageHealth::Unknown),
        "status-pill pending"
    );
    assert_eq!(
        secret_storage_health_class(SecretStorageHealth::Unavailable),
        "status-pill blocked"
    );
}
