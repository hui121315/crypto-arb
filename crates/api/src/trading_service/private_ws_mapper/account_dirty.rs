use super::*;

pub(super) fn dirty_account(
    venue: &str,
    scope: PrivateAccountScope,
    reason: &'static str,
) -> Vec<PrivateWsEvent> {
    vec![PrivateWsEvent::AccountDirty(PrivateAccountDirty::new(
        venue, scope, reason,
    ))]
}
