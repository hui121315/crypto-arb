use crate::services::instrument_registry::CandidateTransferStatus;

pub(super) const fn status_allows_deterministic_delivery(status: &CandidateTransferStatus) -> bool {
    matches!(
        status,
        CandidateTransferStatus::NotRequired { .. } | CandidateTransferStatus::Available { .. }
    )
}

pub(super) fn candidate_transfer_payload(status: &CandidateTransferStatus) -> serde_json::Value {
    match status {
        CandidateTransferStatus::NotApplicable => serde_json::json!({
            "state": "not_applicable", "available": null,
            "detail": "当前策略不需要充提路径",
        }),
        CandidateTransferStatus::NotRequired { detail } => serde_json::json!({
            "state": "not_required", "available": true, "detail": detail,
        }),
        CandidateTransferStatus::Warming { detail } => serde_json::json!({
            "state": "warming", "available": null, "detail": detail,
        }),
        CandidateTransferStatus::Available {
            detail,
            base_network,
            quote_network,
            requires_tag,
        } => serde_json::json!({
            "state": "available", "available": true, "detail": detail,
            "baseNetwork": base_network, "quoteNetwork": quote_network,
            "requiresTag": requires_tag,
        }),
        CandidateTransferStatus::Blocked { detail } => serde_json::json!({
            "state": "blocked", "available": false, "detail": detail,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_available_or_unnecessary_transfer_can_be_deterministic() {
        assert!(status_allows_deterministic_delivery(
            &CandidateTransferStatus::NotRequired {
                detail: "同场无需搬币".to_owned(),
            }
        ));
        assert!(status_allows_deterministic_delivery(
            &CandidateTransferStatus::Available {
                detail: "共同网络可用".to_owned(),
                base_network: "ethereum".to_owned(),
                quote_network: "tron".to_owned(),
                requires_tag: false,
            }
        ));
        assert!(!status_allows_deterministic_delivery(
            &CandidateTransferStatus::Warming {
                detail: "正在读取".to_owned(),
            }
        ));
        assert!(!status_allows_deterministic_delivery(
            &CandidateTransferStatus::Blocked {
                detail: "提币暂停".to_owned(),
            }
        ));
        assert!(!status_allows_deterministic_delivery(
            &CandidateTransferStatus::NotApplicable
        ));
    }
}
