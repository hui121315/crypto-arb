use leptos::prelude::*;

#[derive(Clone, Copy)]
pub(in crate::panels::modules::positions) struct SnapshotRequestGate {
    pub(in crate::panels::modules::positions) version: RwSignal<u64>,
    pub(in crate::panels::modules::positions) token: u64,
}

pub(super) fn next_snapshot_request_gate(version: RwSignal<u64>) -> SnapshotRequestGate {
    let token = version.get_untracked().wrapping_add(1);
    version.set(token);
    SnapshotRequestGate { version, token }
}

pub(super) fn invalidate_snapshot_requests(version: RwSignal<u64>) {
    version.update(|latest| *latest = latest.wrapping_add(1));
}

impl SnapshotRequestGate {
    pub(super) fn is_latest(self) -> bool {
        self.version
            .try_get_untracked()
            .is_some_and(|latest| snapshot_response_is_latest(latest, self.token))
    }
}

pub(in crate::panels::modules::positions) fn snapshot_response_is_latest(
    latest: u64,
    token: u64,
) -> bool {
    latest == token
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disposed_route_rejects_late_snapshot_response_without_panicking() {
        let owner = Owner::new();
        let gate = owner.with(|| next_snapshot_request_gate(RwSignal::new(0)));

        owner.cleanup();

        assert!(!gate.is_latest());
    }

    #[test]
    fn ws_snapshot_invalidates_an_older_rest_response() {
        Owner::new().with(|| {
            let version = RwSignal::new(0);
            let rest = next_snapshot_request_gate(version);

            invalidate_snapshot_requests(version);

            assert!(!rest.is_latest());
        });
    }
}
