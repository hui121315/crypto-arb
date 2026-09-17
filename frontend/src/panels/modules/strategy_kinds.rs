use shared_types::{StrategyKind, StrategyKindInfo};

use super::strategy_scope::{p0_strategy_label, P0_STRATEGY_KINDS};

pub(crate) fn p0_strategy_chip_options(
    kinds: &[StrategyKindInfo],
) -> Vec<(Option<StrategyKind>, &'static str)> {
    let mut chips = P0_STRATEGY_KINDS
        .into_iter()
        .filter(|kind| kind_enabled(kinds, *kind))
        .filter_map(|kind| p0_strategy_label(kind).map(|label| (Some(kind), label)))
        .collect::<Vec<_>>();
    if !chips.is_empty() {
        chips.insert(0, (None, "全部"));
    }
    chips
}

pub(crate) fn strategy_option_target_index(key: &str, current: usize, len: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    match key {
        "ArrowRight" | "ArrowDown" => Some((current + 1) % len),
        "ArrowLeft" | "ArrowUp" => Some(current.checked_sub(1).unwrap_or(len - 1)),
        "Home" => Some(0),
        "End" => Some(len - 1),
        _ => None,
    }
}

fn kind_enabled(kinds: &[StrategyKindInfo], kind: StrategyKind) -> bool {
    kinds
        .iter()
        .any(|item| item.kind == kind && item.is_main_p0_executable())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p0_strategy_chip_options_require_backend_enabled_rows() {
        let kinds = vec![
            StrategyKindInfo::from_kind(StrategyKind::PerpCross, true),
            strategy_kind_info(StrategyKind::SpotPerp, false),
            StrategyKindInfo::from_kind(StrategyKind::SpotCross, true),
        ];

        let chips = p0_strategy_chip_options(&kinds);

        assert_eq!(
            chips,
            vec![
                (None, "全部"),
                (Some(StrategyKind::PerpCross), "永续跨所"),
                (Some(StrategyKind::SpotCross), "现货跨所"),
            ]
        );
    }

    #[test]
    fn empty_strategy_kind_snapshot_shows_no_local_default_chips() {
        assert!(p0_strategy_chip_options(&[]).is_empty());
    }

    fn strategy_kind_info(kind: StrategyKind, frontend_enabled: bool) -> StrategyKindInfo {
        StrategyKindInfo {
            frontend_enabled,
            exposure: if frontend_enabled {
                shared_types::StrategyExposure::MainP0
            } else {
                shared_types::StrategyExposure::Diagnostic
            },
            engine_present: frontend_enabled,
            data_contract_ready: frontend_enabled,
            execution_supported: frontend_enabled,
            ..StrategyKindInfo::from_kind(kind, true)
        }
    }
}
