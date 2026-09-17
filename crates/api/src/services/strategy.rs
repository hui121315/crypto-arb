use shared_types::{
    is_p0_executable_strategy, StrategyKind, StrategyKindInfo, P0_EXECUTABLE_STRATEGY_KINDS,
    PHASE2_STRATEGY_KINDS,
};

pub(crate) fn list_kinds() -> Vec<StrategyKindInfo> {
    PHASE2_STRATEGY_KINDS
        .into_iter()
        .map(|kind| StrategyKindInfo::from_kind(kind, implemented(kind)))
        .collect()
}

pub(crate) fn list_main_kinds() -> Vec<StrategyKindInfo> {
    P0_EXECUTABLE_STRATEGY_KINDS
        .into_iter()
        .map(|kind| StrategyKindInfo::from_kind(kind, true))
        .collect()
}

const fn implemented(kind: StrategyKind) -> bool {
    is_p0_executable_strategy(kind)
}

#[cfg(test)]
mod tests {
    use super::list_kinds;
    use shared_types::{StrategyKindInfo, LIVE_EXECUTABLE_STRATEGY_KINDS};

    #[test]
    fn lists_phase2_strategy_contracts() {
        let kinds = list_kinds();
        assert_eq!(kinds.len(), 9);
        assert!(kinds
            .iter()
            .filter(|item| item.frontend_enabled)
            .all(|item| shared_types::is_p0_executable_strategy(item.kind)));
        assert!(kinds
            .iter()
            .filter(|item| item.implemented)
            .all(|item| shared_types::is_p0_executable_strategy(item.kind)));
        assert!(kinds
            .iter()
            .filter(|item| item.execution_supported)
            .all(|item| item.is_main_p0_executable()));
        assert!(kinds
            .iter()
            .any(|item| !item.implemented
                && item.kind == shared_types::StrategyKind::OptionsPerpBasis));
        assert!(kinds
            .iter()
            .any(|item| !item.frontend_enabled
                && item.kind == shared_types::StrategyKind::FundingCarry));
    }

    #[test]
    fn lists_main_strategy_contracts_only() {
        let kinds = super::list_main_kinds();
        assert_eq!(
            kinds.len(),
            shared_types::P0_EXECUTABLE_STRATEGY_KINDS.len()
        );
        assert!(kinds.iter().all(|item| item.exposure.is_main_p0()));
        assert!(kinds.iter().all(|item| item.frontend_enabled));
        assert!(kinds.iter().all(|item| item.implemented));
        assert!(kinds.iter().all(|item| item.engine_present));
        assert!(kinds.iter().all(|item| item.data_contract_ready));
        assert!(kinds.iter().all(|item| item.execution_supported));
        assert!(kinds.iter().all(StrategyKindInfo::is_main_p0_executable));
        assert_eq!(
            kinds
                .iter()
                .filter(|item| item.live_execution_supported)
                .map(|item| item.kind)
                .collect::<Vec<_>>(),
            LIVE_EXECUTABLE_STRATEGY_KINDS
        );
        assert!(kinds
            .iter()
            .filter(|item| item.live_execution_supported)
            .all(StrategyKindInfo::is_main_p0_live_executable));
    }
}
