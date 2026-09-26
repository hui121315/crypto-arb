use super::load_state::LoadState;
use crate::panels::shared::confirmation::ConfirmedAt;
use gloo_timers::callback::Interval;
use leptos::prelude::*;
use shared_types::ApiProblem;

#[derive(Clone, Copy)]
pub(crate) struct ReadFreshness {
    confirmed: RwSignal<ConfirmedAt>,
    pub(crate) expired: RwSignal<bool>,
}

impl ReadFreshness {
    pub(crate) fn new<T: Send + Sync + 'static>(
        state: RwSignal<LoadState<T>>,
        problem: ApiProblem,
    ) -> Self {
        let freshness = Self {
            confirmed: RwSignal::new(ConfirmedAt::now()),
            expired: RwSignal::new(false),
        };
        let timer = StoredValue::new_local(None::<Interval>);
        Effect::new(move |_| {
            let problem = problem.clone();
            timer.set_value(Some(Interval::new(1_000, move || {
                if freshness
                    .confirmed
                    .try_get_untracked()
                    .is_some_and(ConfirmedAt::expired)
                {
                    if freshness.expired.try_get_untracked() == Some(false) {
                        freshness.expired.try_set(true);
                    }
                    if state.try_with_untracked(|state| {
                        matches!(state, LoadState::Ready(_) | LoadState::Loading)
                    }) == Some(true)
                    {
                        state.try_update(|state| {
                            state.apply_result(Err(problem.clone()));
                        });
                    }
                }
            })));
        });
        on_cleanup(move || {
            timer.update_value(|timer| {
                timer.take();
            })
        });
        freshness
    }

    pub(crate) fn reset(self) {
        self.confirmed.set(ConfirmedAt::now());
        if self.expired.get_untracked() {
            self.expired.set(false);
        }
    }

    pub(crate) fn confirm(self, started: ConfirmedAt) {
        self.confirmed.set(started);
        if self.expired.get_untracked() {
            self.expired.set(false);
        }
    }
}
