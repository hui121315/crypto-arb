/// Canonical source for scanner, list and `HedgeTicket` profitability evidence.
pub const PROFITABILITY_EVIDENCE_SOURCE: &str = "fee_schedule_registry+funding_history";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profitability_evidence_source_names_both_authoritative_registries() {
        assert_eq!(
            PROFITABILITY_EVIDENCE_SOURCE,
            "fee_schedule_registry+funding_history"
        );
    }
}
