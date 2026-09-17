const REQUIRED_FEE_EVIDENCE_LEGS: usize = 2;

pub(crate) fn fee_evidence_label(count: usize, complete: bool) -> String {
    let count = count.min(REQUIRED_FEE_EVIDENCE_LEGS);
    if complete && count == REQUIRED_FEE_EVIDENCE_LEGS {
        "费率证据 2/2".into()
    } else {
        format!("费率证据 {count}/2 未完整")
    }
}

pub(crate) const fn one_cycle_verdict(covers_round_trip_cost: bool) -> &'static str {
    if covers_round_trip_cost {
        "覆盖成本"
    } else {
        "阻断执行"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_cost_copy_fails_closed_and_keeps_one_cycle_verdicts_stable() {
        assert_eq!(fee_evidence_label(2, true), "费率证据 2/2");
        assert_eq!(fee_evidence_label(1, false), "费率证据 1/2 未完整");
        assert_eq!(fee_evidence_label(0, true), "费率证据 0/2 未完整");
        assert_eq!(one_cycle_verdict(true), "覆盖成本");
        assert_eq!(one_cycle_verdict(false), "阻断执行");
    }
}
