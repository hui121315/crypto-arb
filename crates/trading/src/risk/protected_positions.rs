use super::evidence::RiskEvidenceContext;
use shared_types::{
    normalized_venue_name, OrderIntent, ProtectedPositionFingerprint, RiskBlockEvidence,
    RiskBlockReason,
};

pub(super) fn normalize(fingerprints: &mut [ProtectedPositionFingerprint]) {
    for fingerprint in fingerprints.iter_mut() {
        fingerprint.venue = normalized_venue_name(&fingerprint.venue);
        fingerprint.canonical_symbol = normalized_symbol(&fingerprint.canonical_symbol);
        fingerprint.native_symbol = normalized_symbol(&fingerprint.native_symbol);
        fingerprint.side = fingerprint.side.trim().to_ascii_lowercase();
        fingerprint.opening_identity = fingerprint.opening_identity.trim().to_owned();
        fingerprint.source = fingerprint.source.trim().to_owned();
        fingerprint.position_mode = fingerprint
            .position_mode
            .take()
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty());
    }
    fingerprints.sort_by(|left, right| {
        (
            &left.venue,
            &left.native_symbol,
            &left.side,
            &left.opening_identity,
        )
            .cmp(&(
                &right.venue,
                &right.native_symbol,
                &right.side,
                &right.opening_identity,
            ))
    });
}

pub(super) fn record_block(
    fingerprints: &[ProtectedPositionFingerprint],
    intent: &OrderIntent,
    reasons: &mut Vec<RiskBlockReason>,
    evidence: &mut Vec<RiskBlockEvidence>,
    context: &RiskEvidenceContext<'_>,
) {
    let Some(fingerprint) = matching_fingerprint(fingerprints, intent) else {
        return;
    };
    context.push(
        reasons,
        evidence,
        RiskBlockReason::ProtectedPosition,
        (
            "protected_position_fingerprint",
            Some(serde_json::json!({
                "venue": fingerprint.venue,
                "canonicalSymbol": fingerprint.canonical_symbol,
                "nativeSymbol": fingerprint.native_symbol,
                "side": fingerprint.side,
                "quantity": fingerprint.quantity,
                "entryPrice": fingerprint.entry_price,
                "positionMode": fingerprint.position_mode,
                "openingIdentity": fingerprint.opening_identity,
                "source": fingerprint.source,
                "capturedAtMs": fingerprint.captured_at_ms,
            })),
            Some(serde_json::Value::String(
                "no order may target this protected position".to_owned(),
            )),
        ),
    );
}

fn matching_fingerprint<'a>(
    fingerprints: &'a [ProtectedPositionFingerprint],
    intent: &OrderIntent,
) -> Option<&'a ProtectedPositionFingerprint> {
    let venue = normalized_venue_name(&intent.exchange);
    let symbol = normalized_symbol(&intent.symbol);
    fingerprints.iter().find(|fingerprint| {
        fingerprint.venue == venue
            && (fingerprint.canonical_symbol == symbol || fingerprint.native_symbol == symbol)
    })
}

fn normalized_symbol(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}
