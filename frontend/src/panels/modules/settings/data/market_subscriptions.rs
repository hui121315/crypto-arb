use crate::api::rest::with_mutation_timeout;
use crate::state::action_state::{action_state_from_action_run, ActionState};
use crate::state::load_state::LoadState;
use crate::state::read_scope::{ReadScope, ScopedRead};
use leptos::{prelude::*, task::spawn_local};
use shared_types::{ActionRunKind, MarketSubscriptionPatch, MarketSubscriptionsResponse};

use super::resources::{settings_read, SettingsResource};
use super::{validate_setting_response, SettingsJournal};

#[derive(Clone, Copy)]
pub(in crate::panels::modules::settings) struct MarketSubscriptionData {
    pub state: SettingsResource<MarketSubscriptionsResponse>,
    pub action: RwSignal<ActionState>,
    pub refreshing: RwSignal<bool>,
    pub refresh: Callback<()>,
    pub submit: Callback<MarketSubscriptionPatch>,
    pub journal: SettingsJournal,
    pub recheck: Callback<()>,
    read_epoch: RwSignal<u64>,
    active: RwSignal<bool>,
    read: ScopedRead,
}

pub(in crate::panels::modules::settings) fn create_market_subscriptions() -> MarketSubscriptionData
{
    let journal = SettingsJournal::new("market");
    let state = RwSignal::new(LoadState::<MarketSubscriptionsResponse>::Loading);
    let action = RwSignal::new(ActionState::Idle);
    let refreshing = RwSignal::new(false);
    let revision = RwSignal::new(0_u64);
    let read_epoch = RwSignal::new(0_u64);
    let active = RwSignal::new(false);
    let scope = ReadScope::new(|| {});
    let read = scope.request();
    let refresh = Callback::new(move |()| {
        if !active.get_untracked() || refreshing.get_untracked() || journal.busy.get_untracked() {
            return;
        }
        refreshing.set(true);
        let anchor = revision.get_untracked();
        let epoch = read_epoch.get_untracked();
        let operation = journal.epoch.get_untracked();
        read.run(|client| async move {
            settings_read(client.market_subscriptions()).await.map_err(|e| e.problem)
        }, move |result| {
            if read_epoch.try_get_untracked() != Some(epoch) {
                return;
            }
            refreshing.set(false);
            if journal.current(operation) && anchor == revision.get_untracked() {
                state.update(|state| state.apply_result(result));
            }
        });
    });
    let recheck = journal.recheck(Callback::new(move |run| {
        action.set(action_state_from_action_run(&run));
        revision.update(|v| *v = v.wrapping_add(1));
        state.set(LoadState::Loading);
        // The read runs after the journal releases its in-flight lock.
        spawn_local(async move {
            refresh.run(());
        });
    }));
    Effect::new(move |_| {
        journal.connection.track();
        scope.track();
        read.cancel();
        action.set(journal.restored_state(&[ActionRunKind::MarketSubscriptionsUpdate]));
        revision.update(|v| *v = v.wrapping_add(1));
        state.set(LoadState::Loading);
        refreshing.set(false);
        refresh.run(());
    });
    let submit = Callback::new(move |patch: MarketSubscriptionPatch| {
        if !matches!(state.get_untracked(), LoadState::Ready(_)) {
            return;
        }
        let Some(attempt) = journal.begin(
            ActionRunKind::MarketSubscriptionsUpdate,
            patch.venue.trim().to_ascii_lowercase(),
        ) else {
            return;
        };
        let epoch = journal.epoch.get_untracked();
        read.cancel();
        refreshing.set(false);
        let evidence = attempt.context.evidence();
        action.set(ActionState::pending("正在更新行情订阅").with_evidence(evidence.clone()));
        revision.update(|v| *v = v.wrapping_add(1));
        let client = journal.client();
        spawn_local(async move {
            let result = with_mutation_timeout(
                "更新行情订阅",
                client.update_market_subscription_with_context(&patch, &attempt.context),
            )
            .await
            .and_then(|response| {
                validate_setting_response(&attempt, &response)?;
                Ok(response)
            });
            if !journal.current(epoch) {
                return;
            }
            revision.update(|v| *v = v.wrapping_add(1));
            match result {
                Ok(response) => {
                    state.set(LoadState::Ready(response));
                    let message = if journal.resolve(&attempt) {
                        ActionState::succeeded("行情订阅已保存；连接状态以运行数据依据为准")
                    } else {
                        ActionState::accepted("处理结果已返回，恢复记录待清理")
                    };
                    action.set(message.with_evidence(evidence));
                    journal.busy.set(false);
                }
                Err(error) => {
                    journal.failed(&attempt, &error);
                    action.set(
                        ActionState::failed("行情订阅更新未确认", error.problem)
                            .with_evidence(evidence),
                    );
                    refresh.run(());
                }
            }
        });
    });
    MarketSubscriptionData {
        state,
        action,
        refreshing,
        refresh,
        submit,
        journal,
        recheck,
        read_epoch,
        active,
        read,
    }
}

pub(in crate::panels::modules::settings) fn use_market_subscriptions(
    data: MarketSubscriptionData,
) -> MarketSubscriptionData {
    let epoch = data.read_epoch.get_untracked().wrapping_add(1);
    data.read_epoch.set(epoch);
    data.active.set(true);
    data.refreshing.set(false);
    on_cleanup(move || {
        if data.read_epoch.try_get_untracked() == Some(epoch) {
            data.read.cancel();
            data.read_epoch.set(epoch.wrapping_add(1));
            data.active.set(false);
            data.refreshing.set(false);
        }
    });
    data.refresh.run(());
    data
}
