//! Order identity policy DTOs.

use crate::fees::FeeProduct;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientOrderIdDerivation {
    #[default]
    Unsupported,
    Identity,
    Normalized,
    StableHash,
    NumericHash,
    Rejected,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientOrderIdPolicy {
    pub venue: String,
    pub venue_family: String,
    pub venue_field: String,
    pub public_client_order_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub venue_client_order_id: Option<String>,
    pub derivation: ClientOrderIdDerivation,
    pub policy_version: String,
    pub official_format: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u16>,
    pub supports_query_by_client_id: bool,
    pub supports_cancel_by_client_id: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constraints: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blockers: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub official_doc_urls: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderIdentityEvidenceKind {
    Metadata,
    UserStream,
    OrderFinality,
    Fee,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderIdentityEvidenceStatus {
    Verified,
    Mismatched,
    #[default]
    Unavailable,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderIdentityEvidence {
    pub kind: OrderIdentityEvidenceKind,
    pub status: OrderIdentityEvidenceStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixture_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parser_test: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl OrderIdentityEvidence {
    pub fn is_verified(&self) -> bool {
        self.status == OrderIdentityEvidenceStatus::Verified
            && self.evidence_id.as_deref().is_some_and(non_empty)
            && self.source.as_deref().is_some_and(non_empty)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExchangeOrderIdFinalitySource {
    PrivateUserStream,
    RestOrderQuery,
    PrivateUserStreamWithRestFallback,
    #[default]
    Unavailable,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderIdentityPlan {
    pub evidence_required: bool,
    pub canonical_symbol: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_symbol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settle_asset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote_asset: Option<String>,
    pub product: FeeProduct,
    pub client_order_id_policy: ClientOrderIdPolicy,
    pub exchange_order_id_finality_source: ExchangeOrderIdFinalitySource,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<OrderIdentityEvidence>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blockers: Vec<String>,
}

impl OrderIdentityPlan {
    pub fn from_compile_contract(
        venue: &str,
        symbol: &str,
        product: FeeProduct,
        client_order_id_policy: &ClientOrderIdPolicy,
    ) -> Self {
        let fields = identity_fields(&client_order_id_policy.constraints);
        let declared_canonical_symbol = field(&fields, "identity.canonical_symbol");
        let evidence_required =
            (is_binance_family(venue) && product == FeeProduct::Perp) || !fields.is_empty();
        let canonical_symbol = declared_canonical_symbol.unwrap_or(symbol).to_owned();
        let native_symbol = optional_field(&fields, "identity.native_symbol");
        let settle_asset = optional_field(&fields, "identity.settle_asset");
        let quote_asset = optional_field(&fields, "identity.quote_asset");
        let finality_source =
            parse_finality_source(field(&fields, "identity.exchange_order_id_finality_source"));
        let evidence = evidence_rows(&fields);
        let mut plan = Self {
            evidence_required,
            canonical_symbol,
            native_symbol,
            settle_asset,
            quote_asset,
            product,
            client_order_id_policy: client_order_id_policy.clone(),
            exchange_order_id_finality_source: finality_source,
            evidence,
            blockers: Vec::new(),
        };
        plan.blockers = identity_blockers(
            &plan,
            symbol,
            declared_canonical_symbol,
            field(&fields, "identity.product"),
        );
        plan
    }

    pub fn is_execution_ready(&self) -> bool {
        !self.evidence_required || self.blockers.is_empty()
    }

    pub fn evidence_for(&self, kind: OrderIdentityEvidenceKind) -> Option<&OrderIdentityEvidence> {
        self.evidence.iter().find(|row| row.kind == kind)
    }
}

fn identity_fields(constraints: &[String]) -> BTreeMap<&str, &str> {
    constraints
        .iter()
        .filter_map(|constraint| constraint.split_once('='))
        .filter(|(key, value)| key.starts_with("identity.") && non_empty(value))
        .collect()
}

fn field<'a>(fields: &'a BTreeMap<&str, &str>, key: &str) -> Option<&'a str> {
    fields.get(key).copied().filter(|value| non_empty(value))
}

fn optional_field(fields: &BTreeMap<&str, &str>, key: &str) -> Option<String> {
    field(fields, key).map(str::to_owned)
}

fn evidence_rows(fields: &BTreeMap<&str, &str>) -> Vec<OrderIdentityEvidence> {
    [
        (OrderIdentityEvidenceKind::Metadata, "metadata"),
        (OrderIdentityEvidenceKind::UserStream, "user_stream"),
        (OrderIdentityEvidenceKind::OrderFinality, "order_finality"),
        (OrderIdentityEvidenceKind::Fee, "fee"),
    ]
    .into_iter()
    .map(|(kind, key)| evidence_row(fields, kind, key))
    .collect()
}

fn evidence_row(
    fields: &BTreeMap<&str, &str>,
    kind: OrderIdentityEvidenceKind,
    key: &str,
) -> OrderIdentityEvidence {
    let prefix = format!("identity.evidence.{key}");
    OrderIdentityEvidence {
        kind,
        status: parse_evidence_status(field(fields, &format!("{prefix}.status"))),
        evidence_id: optional_field(fields, &format!("{prefix}.evidence_id")),
        source: optional_field(fields, &format!("{prefix}.source")),
        fixture_id: optional_field(fields, &format!("{prefix}.fixture_id")),
        parser_test: optional_field(fields, &format!("{prefix}.parser_test")),
        detail: optional_field(fields, &format!("{prefix}.detail")),
    }
}

fn parse_evidence_status(value: Option<&str>) -> OrderIdentityEvidenceStatus {
    match value {
        Some("verified") => OrderIdentityEvidenceStatus::Verified,
        Some("mismatched") => OrderIdentityEvidenceStatus::Mismatched,
        _ => OrderIdentityEvidenceStatus::Unavailable,
    }
}

fn parse_finality_source(value: Option<&str>) -> ExchangeOrderIdFinalitySource {
    match value {
        Some("private_user_stream") => ExchangeOrderIdFinalitySource::PrivateUserStream,
        Some("rest_order_query") => ExchangeOrderIdFinalitySource::RestOrderQuery,
        Some("private_user_stream_with_rest_fallback") => {
            ExchangeOrderIdFinalitySource::PrivateUserStreamWithRestFallback
        }
        _ => ExchangeOrderIdFinalitySource::Unavailable,
    }
}

fn identity_blockers(
    plan: &OrderIdentityPlan,
    compile_symbol: &str,
    declared_canonical_symbol: Option<&str>,
    declared_product: Option<&str>,
) -> Vec<String> {
    if !plan.evidence_required {
        return Vec::new();
    }
    let mut blockers = Vec::new();
    required_match(
        &mut blockers,
        "ORDER_IDENTITY_CANONICAL_SYMBOL",
        declared_canonical_symbol,
        compile_symbol,
    );
    required_value(
        &mut blockers,
        "ORDER_IDENTITY_NATIVE_SYMBOL_MISSING",
        plan.native_symbol.as_deref(),
    );
    required_value(
        &mut blockers,
        "ORDER_IDENTITY_SETTLE_ASSET_MISSING",
        plan.settle_asset.as_deref(),
    );
    required_value(
        &mut blockers,
        "ORDER_IDENTITY_QUOTE_ASSET_MISSING",
        plan.quote_asset.as_deref(),
    );
    let expected_product = product_name(plan.product);
    if declared_product != Some(expected_product) {
        blockers.push("ORDER_IDENTITY_PRODUCT_MISMATCH".to_owned());
    }
    if !client_id_policy_ready(&plan.client_order_id_policy) {
        blockers.push("ORDER_IDENTITY_CLIENT_ID_POLICY_UNVERIFIED".to_owned());
    }
    if plan.exchange_order_id_finality_source == ExchangeOrderIdFinalitySource::Unavailable {
        blockers.push("ORDER_IDENTITY_FINALITY_SOURCE_UNAVAILABLE".to_owned());
    }
    for kind in [
        OrderIdentityEvidenceKind::Metadata,
        OrderIdentityEvidenceKind::UserStream,
        OrderIdentityEvidenceKind::OrderFinality,
        OrderIdentityEvidenceKind::Fee,
    ] {
        if !plan
            .evidence_for(kind)
            .is_some_and(OrderIdentityEvidence::is_verified)
        {
            blockers.push(format!(
                "ORDER_IDENTITY_{}_EVIDENCE_UNAVAILABLE",
                evidence_code(kind)
            ));
        }
    }
    blockers
}

fn is_binance_family(venue: &str) -> bool {
    venue
        .split([':', '-', '_'])
        .next()
        .is_some_and(|family| family.eq_ignore_ascii_case("binance"))
}

fn required_match(blockers: &mut Vec<String>, code: &str, actual: Option<&str>, expected: &str) {
    match actual {
        None => blockers.push(format!("{code}_MISSING")),
        Some(actual) if actual != expected => blockers.push(format!("{code}_MISMATCH")),
        Some(_) => {}
    }
}

fn required_value(blockers: &mut Vec<String>, code: &str, value: Option<&str>) {
    if !value.is_some_and(non_empty) {
        blockers.push(code.to_owned());
    }
}

fn client_id_policy_ready(policy: &ClientOrderIdPolicy) -> bool {
    non_empty(&policy.public_client_order_id)
        && policy
            .venue_client_order_id
            .as_deref()
            .is_some_and(non_empty)
        && !matches!(
            policy.derivation,
            ClientOrderIdDerivation::Unsupported | ClientOrderIdDerivation::Rejected
        )
        && policy.blockers.is_empty()
}

fn product_name(product: FeeProduct) -> &'static str {
    match product {
        FeeProduct::Spot => "spot",
        FeeProduct::Perp => "perp",
        FeeProduct::Margin => "margin",
        FeeProduct::Unknown => "unknown",
    }
}

fn evidence_code(kind: OrderIdentityEvidenceKind) -> &'static str {
    match kind {
        OrderIdentityEvidenceKind::Metadata => "METADATA",
        OrderIdentityEvidenceKind::UserStream => "USER_STREAM",
        OrderIdentityEvidenceKind::OrderFinality => "ORDER_FINALITY",
        OrderIdentityEvidenceKind::Fee => "FEE",
        OrderIdentityEvidenceKind::Unknown => "UNKNOWN",
    }
}

fn non_empty(value: &str) -> bool {
    !value.trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verified_usdc_native_identity_is_execution_ready() {
        let policy = policy(identity_constraints("BTCUSDC", "verified"));
        let plan = OrderIdentityPlan::from_compile_contract(
            "binance",
            "BTCUSDC",
            FeeProduct::Perp,
            &policy,
        );

        assert!(plan.is_execution_ready());
        assert_eq!(plan.native_symbol.as_deref(), Some("BTCUSDC"));
        assert_eq!(plan.settle_asset.as_deref(), Some("USDC"));
        assert_eq!(plan.quote_asset.as_deref(), Some("USDC"));
        assert_eq!(plan.evidence.len(), 4);
    }

    #[test]
    fn missing_and_mismatched_evidence_fail_closed() {
        let policy = policy(identity_constraints("BTCUSDT", "unavailable"));
        let plan = OrderIdentityPlan::from_compile_contract(
            "binance",
            "BTCUSDC",
            FeeProduct::Perp,
            &policy,
        );

        assert!(!plan.is_execution_ready());
        assert!(plan
            .blockers
            .contains(&"ORDER_IDENTITY_CANONICAL_SYMBOL_MISMATCH".to_owned()));
        assert!(plan
            .blockers
            .contains(&"ORDER_IDENTITY_USER_STREAM_EVIDENCE_UNAVAILABLE".to_owned()));
        assert!(plan
            .blockers
            .contains(&"ORDER_IDENTITY_FEE_EVIDENCE_UNAVAILABLE".to_owned()));
    }

    #[test]
    fn legacy_non_binance_plan_does_not_require_new_identity_evidence() {
        let policy = policy(Vec::new());
        let plan = OrderIdentityPlan::from_compile_contract(
            "kucoin",
            "BTCUSDTM",
            FeeProduct::Perp,
            &policy,
        );

        assert!(!plan.evidence_required);
        assert_eq!(plan.canonical_symbol, "BTCUSDTM");
        assert!(plan.is_execution_ready());
        assert!(plan.blockers.is_empty());
    }

    #[test]
    fn binance_spot_plan_is_not_blocked_by_usdm_identity_contract() {
        let policy = policy(Vec::new());
        let plan = OrderIdentityPlan::from_compile_contract(
            "binance",
            "BTCUSDT",
            FeeProduct::Spot,
            &policy,
        );

        assert!(!plan.evidence_required);
        assert!(plan.is_execution_ready());
        assert!(plan.blockers.is_empty());
    }

    fn policy(constraints: Vec<String>) -> ClientOrderIdPolicy {
        ClientOrderIdPolicy {
            venue: "binance".into(),
            venue_family: "binance".into(),
            venue_field: "newClientOrderId".into(),
            public_client_order_id: "public-order-1".into(),
            venue_client_order_id: Some("venue-order-1".into()),
            derivation: ClientOrderIdDerivation::Identity,
            policy_version: "binance-usdm-v1".into(),
            official_format: "1..=36 ASCII".into(),
            max_length: Some(36),
            supports_query_by_client_id: true,
            supports_cancel_by_client_id: true,
            constraints,
            blockers: Vec::new(),
            official_doc_urls: vec!["https://developers.binance.com/".into()],
        }
    }

    fn identity_constraints(canonical_symbol: &str, evidence_status: &str) -> Vec<String> {
        let mut values = vec![
            format!("identity.canonical_symbol={canonical_symbol}"),
            "identity.native_symbol=BTCUSDC".into(),
            "identity.settle_asset=USDC".into(),
            "identity.quote_asset=USDC".into(),
            "identity.product=perp".into(),
            "identity.exchange_order_id_finality_source=private_user_stream_with_rest_fallback"
                .into(),
        ];
        for kind in ["metadata", "user_stream", "order_finality", "fee"] {
            values.push(format!("identity.evidence.{kind}.status={evidence_status}"));
            values.push(format!(
                "identity.evidence.{kind}.evidence_id=binance-{kind}-1"
            ));
            values.push(format!("identity.evidence.{kind}.source=e2e-capture"));
        }
        values
    }
}
