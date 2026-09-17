//! 交易所 WebSocket 能力矩阵 DTO。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExchangeWsVenuesResponse {
    pub venues: Vec<ExchangeWsVenue>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExchangeWsOperationsResponse {
    pub venues: Vec<ExchangeWsOperationVenue>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExchangeWsOperationVenue {
    pub venue: String,
    pub label: String,
    pub operations: Vec<ExchangeWsOperationRegistryRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExchangeWsOperationRegistryRow {
    pub label: String,
    #[serde(default)]
    pub evidence_scope: ExchangeWsEvidenceScope,
    pub supported: bool,
    pub status: ExchangeWsSupportStatus,
    #[serde(default)]
    pub release_status: ExchangeWsReleaseStatus,
    #[serde(default)]
    pub requires_authenticated_runtime_evidence: bool,
    #[serde(default)]
    pub authenticated_runtime_evidence: bool,
    pub operation: Option<String>,
    pub product: String,
    pub note: String,
    pub checked_at: String,
    pub doc_version: String,
    pub doc_url: String,
    pub parser_test: Option<String>,
    pub subscription_test: Option<String>,
    #[serde(default)]
    pub fixture_id: Option<String>,
    #[serde(default)]
    pub fixture_hash: Option<String>,
    pub auth_kind: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExchangeWsEvidenceScope {
    #[default]
    Unknown,
    PrivateAccountStream,
    PrivatePositionStream,
    PrivateFillStream,
    PrivateOrderStream,
    AckOnly,
    OrderStatusRead,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExchangeWsVenue {
    pub venue: String,
    pub label: String,
    pub public_endpoint: String,
    pub private_endpoint: Option<String>,
    pub trade_endpoint: Option<String>,
    pub account_stream: ExchangeWsOperation,
    pub position_stream: ExchangeWsOperation,
    pub fill_stream: ExchangeWsOperation,
    pub order_stream: ExchangeWsOperation,
    pub place_order: ExchangeWsOperation,
    pub cancel_order: ExchangeWsOperation,
    pub close_position: ExchangeWsOperation,
    pub order_status: ExchangeWsOperation,
    pub auth_fields: Vec<String>,
    pub docs: Vec<ExchangeWsDoc>,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExchangeWsOperation {
    pub supported: bool,
    pub status: ExchangeWsSupportStatus,
    pub operation: Option<String>,
    pub product: String,
    pub note: String,
    pub evidence: Option<ExchangeWsOperationEvidence>,
}

impl ExchangeWsOperation {
    /// 该 WS 操作是否可进入实盘下单写路径（live writer）。
    ///
    /// 契约：只有 `supported`、带 `ProductionReady` operation evidence、不缺要求的认证
    /// 运行态证据，且 status 为
    /// `Ready`（公开 schema 已核验）或 `RequiresPermission`（schema 已知、仅需账户交易权限/私有登录）
    /// 才允许被实盘写单分发；
    /// `SchemaPending`（官方公告支持但公开 schema 待核准）与 `RestOnly`（无 WS 写路径）
    /// 即使 `supported = true` 也只能在能力矩阵中展示，绝不能进入 live writer——
    /// 否则会用未核验的消息结构直写生产下单路径。
    pub fn is_live_submittable(&self) -> bool {
        self.supported
            && self.evidence.as_ref().is_some_and(|evidence| {
                evidence.release_status == ExchangeWsReleaseStatus::ProductionReady
                    && (!evidence.requires_authenticated_runtime_evidence
                        || evidence.authenticated_runtime_evidence)
            })
            && matches!(
                self.status,
                ExchangeWsSupportStatus::Ready | ExchangeWsSupportStatus::RequiresPermission
            )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExchangeWsSupportStatus {
    Ready,
    RequiresPermission,
    SchemaPending,
    RestOnly,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExchangeWsReleaseStatus {
    #[default]
    Unknown,
    ProductionReady,
    BetaUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExchangeWsOperationEvidence {
    #[serde(default)]
    pub release_status: ExchangeWsReleaseStatus,
    #[serde(default)]
    pub requires_authenticated_runtime_evidence: bool,
    #[serde(default)]
    pub authenticated_runtime_evidence: bool,
    pub checked_at: String,
    pub doc_version: String,
    pub doc_url: String,
    pub parser_test: Option<String>,
    pub subscription_test: Option<String>,
    #[serde(default)]
    pub fixture_id: Option<String>,
    #[serde(default)]
    pub fixture_hash: Option<String>,
    pub auth_kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExchangeWsDoc {
    pub label: String,
    pub url: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(supported: bool, status: ExchangeWsSupportStatus) -> ExchangeWsOperation {
        ExchangeWsOperation {
            supported,
            status,
            operation: None,
            product: "USDT swap".to_owned(),
            note: String::new(),
            evidence: None,
        }
    }

    fn evidenced_op(supported: bool, status: ExchangeWsSupportStatus) -> ExchangeWsOperation {
        ExchangeWsOperation {
            evidence: Some(ExchangeWsOperationEvidence {
                release_status: ExchangeWsReleaseStatus::ProductionReady,
                requires_authenticated_runtime_evidence: false,
                authenticated_runtime_evidence: false,
                checked_at: "2026-07-02".to_owned(),
                doc_version: "test-doc".to_owned(),
                doc_url: "https://example.test/ws".to_owned(),
                parser_test: Some("parser_test".to_owned()),
                subscription_test: Some("request_test".to_owned()),
                fixture_id: None,
                fixture_hash: None,
                auth_kind: "login".to_owned(),
            }),
            ..op(supported, status)
        }
    }

    #[test]
    fn ready_supported_op_with_evidence_is_live_submittable() {
        assert!(evidenced_op(true, ExchangeWsSupportStatus::Ready).is_live_submittable());
    }

    #[test]
    fn requires_permission_op_with_evidence_is_live_submittable() {
        assert!(
            evidenced_op(true, ExchangeWsSupportStatus::RequiresPermission).is_live_submittable()
        );
    }

    #[test]
    fn ready_supported_op_without_evidence_is_not_live_submittable() {
        assert!(!op(true, ExchangeWsSupportStatus::Ready).is_live_submittable());
    }

    #[test]
    fn schema_pending_op_is_not_live_submittable_even_when_supported() {
        assert!(!evidenced_op(true, ExchangeWsSupportStatus::SchemaPending).is_live_submittable());
    }

    #[test]
    fn rest_only_op_is_not_live_submittable() {
        assert!(!evidenced_op(true, ExchangeWsSupportStatus::RestOnly).is_live_submittable());
    }

    #[test]
    fn unsupported_ready_op_is_not_live_submittable() {
        assert!(!evidenced_op(false, ExchangeWsSupportStatus::Ready).is_live_submittable());
    }

    #[test]
    fn beta_release_evidence_never_becomes_live_submittable_from_static_ack() {
        let mut operation = evidenced_op(true, ExchangeWsSupportStatus::Ready);
        operation
            .evidence
            .as_mut()
            .expect("evidence")
            .release_status = ExchangeWsReleaseStatus::BetaUnavailable;

        assert!(!operation.is_live_submittable());
    }

    #[test]
    fn authenticated_runtime_requirement_blocks_schema_only_write_evidence() {
        let mut operation = evidenced_op(true, ExchangeWsSupportStatus::Ready);
        operation
            .evidence
            .as_mut()
            .expect("evidence")
            .requires_authenticated_runtime_evidence = true;

        assert!(!operation.is_live_submittable());

        operation
            .evidence
            .as_mut()
            .expect("evidence")
            .authenticated_runtime_evidence = true;
        assert!(operation.is_live_submittable());
    }

    #[test]
    fn operation_registry_row_defaults_unknown_evidence_scope_for_legacy_payload() {
        let row = serde_json::json!({
            "label": "place_order",
            "supported": true,
            "status": "ready",
            "operation": "order.place",
            "product": "USD-M",
            "note": "legacy fixture",
            "checkedAt": "2026-07-02",
            "docVersion": "test-doc",
            "docUrl": "https://example.test/ws",
            "parserTest": "parser_test",
            "subscriptionTest": "request_test",
            "authKind": "login"
        });

        let row: ExchangeWsOperationRegistryRow =
            serde_json::from_value(row).expect("legacy row still deserializes");
        assert_eq!(ExchangeWsEvidenceScope::Unknown, row.evidence_scope);
        assert_eq!(ExchangeWsReleaseStatus::Unknown, row.release_status);
        assert!(!row.requires_authenticated_runtime_evidence);
        assert!(!row.authenticated_runtime_evidence);
        assert!(row.fixture_id.is_none());
        assert!(row.fixture_hash.is_none());
    }

    #[test]
    fn operation_registry_row_serializes_typed_evidence_scope() {
        let row = ExchangeWsOperationRegistryRow {
            label: "place_order".to_owned(),
            evidence_scope: ExchangeWsEvidenceScope::AckOnly,
            supported: true,
            status: ExchangeWsSupportStatus::Ready,
            release_status: ExchangeWsReleaseStatus::ProductionReady,
            requires_authenticated_runtime_evidence: false,
            authenticated_runtime_evidence: false,
            operation: Some("order.place".to_owned()),
            product: "USD-M".to_owned(),
            note: "ACK only".to_owned(),
            checked_at: "2026-07-02".to_owned(),
            doc_version: "test-doc".to_owned(),
            doc_url: "https://example.test/ws".to_owned(),
            parser_test: Some("parser_test".to_owned()),
            subscription_test: Some("request_test".to_owned()),
            fixture_id: Some(
                "crates/exchange/fixtures/binance/ws_order_place_success.json".to_owned(),
            ),
            fixture_hash: Some(
                "sha256:ebf0c6b4d768c737094e49dd337346b8084da9f5e394931307157e8c20b58fa6"
                    .to_owned(),
            ),
            auth_kind: "login".to_owned(),
        };

        let value = serde_json::to_value(row).expect("row serializes");
        assert_eq!("ack_only", value["evidenceScope"]);
        assert_eq!("production_ready", value["releaseStatus"]);
        assert_eq!(
            "crates/exchange/fixtures/binance/ws_order_place_success.json",
            value["fixtureId"]
        );
        assert_eq!(
            "sha256:ebf0c6b4d768c737094e49dd337346b8084da9f5e394931307157e8c20b58fa6",
            value["fixtureHash"]
        );
    }
}
