use super::*;

pub(super) fn intent_for_role(preview: &HedgePreviewResponse, role: HedgeLegRole) -> &OrderIntent {
    match role {
        HedgeLegRole::Long => &preview.long_leg,
        HedgeLegRole::Short => &preview.short_leg,
    }
}

pub(super) fn quote_for_role(
    ticket: &shared_types::HedgeTicket,
    role: HedgeLegRole,
) -> &shared_types::HedgeLegQuote {
    match role {
        HedgeLegRole::Long => &ticket.long_leg,
        HedgeLegRole::Short => &ticket.short_leg,
    }
}

pub(super) fn run_leg_mut(run: &mut ExecutionRun, role: HedgeLegRole) -> &mut ExecutionRunLeg {
    match role {
        HedgeLegRole::Long => &mut run.long_leg,
        HedgeLegRole::Short => &mut run.short_leg,
    }
}

pub(super) const fn recovery_action_for_role(role: HedgeLegRole) -> RecoveryAction {
    match role {
        HedgeLegRole::Long => RecoveryAction::UnwindLongLeg,
        HedgeLegRole::Short => RecoveryAction::UnwindShortLeg,
    }
}

pub(super) fn records_by_role(
    first_role: HedgeLegRole,
    first: Option<OrderRecord>,
    second: Option<OrderRecord>,
) -> (Option<OrderRecord>, Option<OrderRecord>) {
    match first_role {
        HedgeLegRole::Long => (first, second),
        HedgeLegRole::Short => (second, first),
    }
}
