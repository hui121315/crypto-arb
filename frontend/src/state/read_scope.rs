use super::AppContext;
use crate::api::{
    base::{normalize_api_auth_token, normalize_api_base},
    rest::ApiClient,
};
use futures::future::{select, AbortHandle, Abortable, Either};
use gloo_timers::{callback::Interval, future::TimeoutFuture};
use leptos::prelude::*;
use leptos::task::spawn_local;
use std::{future::Future, rc::Rc, time::Duration};

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ReadConnection {
    base: String,
    token: String,
    generation: u64,
}

impl ReadConnection {
    fn capture(app: AppContext, generation: u64) -> Self {
        Self {
            base: normalize_api_base(&app.api_base.get()),
            token: normalize_api_auth_token(&app.api_auth_token.get()),
            generation,
        }
    }

    pub(crate) fn client(&self) -> ApiClient {
        ApiClient::with_base_and_auth(&self.base, &self.token)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ReadScope {
    app: AppContext,
    connection: RwSignal<ReadConnection>,
}

impl ReadScope {
    pub(crate) fn new(reset: impl Fn() + 'static) -> Self {
        let app = expect_context::<AppContext>();
        let connection = RwSignal::new(untrack(|| ReadConnection::capture(app, 0)));
        Effect::new(move |_| {
            let previous = connection.get_untracked();
            let mut next = ReadConnection::capture(app, previous.generation);
            if next != previous {
                next.generation = next.generation.wrapping_add(1);
                untrack(&reset);
                connection.set(next);
            }
        });
        Self { app, connection }
    }

    pub(crate) fn track(self) {
        self.connection.track();
    }

    pub(crate) fn capture(self) -> ReadConnection {
        self.connection.get_untracked()
    }

    pub(crate) fn request(self) -> ScopedRead {
        let read = ScopedRead {
            scope: self,
            active: StoredValue::new(None),
            revision: RwSignal::new(0),
        };
        on_cleanup(move || read.cancel());
        read
    }

    pub(crate) fn accepts(self, source: &ReadConnection) -> bool {
        // Identity alone is insufficient when a request survives A -> B -> A.
        self.connection
            .try_with_untracked(|current| current == source)
            == Some(true)
            && self
                .app
                .api_base
                .try_get_untracked()
                .zip(self.app.api_auth_token.try_get_untracked())
                .is_some_and(|(base, token)| {
                    normalize_api_base(&base) == source.base
                        && normalize_api_auth_token(&token) == source.token
                })
    }

    pub(crate) fn poll<T: 'static, Fut: Future<Output = T> + 'static>(
        self,
        period: Duration,
        enabled: impl Fn() -> bool + 'static,
        fetch: impl Fn(ApiClient) -> Fut + 'static,
        apply: impl Fn(T) + 'static,
    ) {
        let tick = RwSignal::new(0_u64);
        let active = StoredValue::new_local(None::<(ReadConnection, AbortHandle)>);
        let timer = StoredValue::new_local(None::<Interval>);
        Effect::new(move |_| {
            timer.set_value(Some(Interval::new(
                period.as_millis().clamp(250, u32::MAX as u128) as u32,
                move || {
                    tick.try_update(|value| *value = value.wrapping_add(1));
                },
            )));
        });
        let apply = Rc::new(apply);
        Effect::new(move |_| {
            self.track();
            tick.get();
            let allowed = enabled();
            untrack(|| {
                let connection = self.capture();
                active.update_value(|slot| {
                    if slot
                        .as_ref()
                        .is_some_and(|(source, _)| source != &connection)
                    {
                        if let Some((_, abort)) = slot.take() {
                            abort.abort();
                        }
                    }
                });
                // One read per source; a hung old source must not block a new one.
                if !allowed || active.with_value(Option::is_some) {
                    return;
                }
                let future = fetch(connection.client().cancelable_reads());
                let (abort, registration) = AbortHandle::new_pair();
                active.set_value(Some((connection.clone(), abort)));
                let apply = apply.clone();
                spawn_local(async move {
                    let Ok(result) = Abortable::new(future, registration).await else {
                        return;
                    };
                    if !self.accepts(&connection) {
                        return;
                    }
                    active.update_value(|slot| {
                        slot.take();
                    });
                    apply(result);
                });
            });
        });
        on_cleanup(move || {
            timer.update_value(|timer| {
                timer.take();
            });
            active.update_value(|slot| {
                if let Some((_, abort)) = slot.take() {
                    abort.abort();
                }
            });
        });
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ScopedRead {
    scope: ReadScope,
    active: StoredValue<Option<AbortHandle>>,
    revision: RwSignal<u64>,
}

impl ScopedRead {
    pub(crate) fn cancel(self) {
        self.active.update_value(|active| {
            if let Some(abort) = active.take() {
                abort.abort();
            }
        });
        self.revision
            .update(|revision| *revision = revision.wrapping_add(1));
    }

    pub(crate) fn run<T: 'static, Fut: Future<Output = T> + 'static>(
        self,
        fetch: impl FnOnce(ApiClient) -> Fut,
        apply: impl FnOnce(T) + 'static,
    ) {
        self.cancel();
        let revision = self.revision.get_untracked();
        let source = self.scope.capture();
        let future = fetch(source.client().cancelable_reads());
        let (abort, registration) = AbortHandle::new_pair();
        self.active.set_value(Some(abort));
        spawn_local(async move {
            let Ok(result) = Abortable::new(future, registration).await else {
                return;
            };
            if self.revision.try_get_untracked() != Some(revision) || !self.scope.accepts(&source) {
                return;
            }
            self.active.update_value(|active| {
                active.take();
            });
            apply(result);
        });
    }
}

pub(crate) async fn bounded_read<T>(
    request: impl Future<Output = Result<T, crate::api::rest::ApiError>>,
) -> Result<T, shared_types::ApiProblem> {
    let timeout = TimeoutFuture::new(15_000);
    futures::pin_mut!(request, timeout);
    match select(request, timeout).await {
        Either::Left((result, _)) => result.map_err(|error| error.problem),
        Either::Right(_) => Err(shared_types::ApiProblem::new(
            "SHARED_READ_TIMEOUT",
            "后台状态读取超过 15 秒未返回",
        )
        .with_source("shared_state")),
    }
}
