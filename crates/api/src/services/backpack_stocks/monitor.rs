use super::*;
use crate::services::onchain_comparison::stock_quotes;
use futures::future::BoxFuture;
use std::sync::Weak;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JobKind {
    Context,
    Quote,
}
type JobResult = Result<Option<StockMarketSnapshot>, String>;
const QUANTITY_RECHECK_MS: i64 = 30_000;

#[derive(PartialEq)]
struct QuantityContext {
    kind: StockRouteKind,
    symbol: Option<String>,
    session: Option<StockSession>,
    market: Option<StockOrderBookMarket>,
}

fn quantity_context(snapshot: &StockMarketSnapshot) -> Option<QuantityContext> {
    let route = snapshot.trading_route.as_ref()?;
    Some(QuantityContext {
        kind: route.kind,
        symbol: route.symbol.clone(),
        session: route.session.clone(),
        market: snapshot.security.as_ref().and_then(|s| {
            s.order_books
                .iter()
                .find(|m| Some(&m.symbol) == route.symbol.as_ref())
                .cloned()
        }),
    })
}

impl BackpackStocks {
    pub(crate) fn set_monitor(
        self: &Arc<Self>,
        request: StockMonitorRequest,
        hub: realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        self.configure_monitor(request)?;
        self.ensure_started(hub.clone());
        self.publish(&hub);
        Ok(self.snapshot())
    }

    fn configure_monitor(&self, request: StockMonitorRequest) -> Result<(), String> {
        let mut snapshot = self.snapshot.write();
        if snapshot
            .security
            .as_ref()
            .is_none_or(|s| s.asset != request.quote.asset)
        {
            return Err("股票已改变，请重新设置监控".into());
        }
        if request.enabled {
            comparison::budget_raw(&request.quote)?;
            comparison::issuer(&snapshot)?;
            if request.alerts.enabled {
                request.alerts.threshold()?;
            }
        }
        if snapshot.monitor.enabled == request.enabled
            && (!request.enabled || snapshot.monitor.request.as_ref() == Some(&request.quote))
            && (!request.enabled || snapshot.monitor.alerts == request.alerts)
        {
            return Ok(());
        }
        self.generation.fetch_add(1, Ordering::SeqCst);
        if request.enabled && snapshot.monitor.request.as_ref() != Some(&request.quote) {
            snapshot.comparison = None;
        }
        let parameters = if request.enabled {
            Some(request.quote)
        } else {
            snapshot.monitor.request.clone()
        };
        let alerts = if request.enabled {
            request.alerts
        } else {
            snapshot.monitor.alerts.clone()
        };
        snapshot.monitor = StockMonitorStatus {
            enabled: request.enabled,
            alerts,
            request: parameters,
            phase: if request.enabled {
                StockMonitorPhase::Refreshing
            } else {
                StockMonitorPhase::Disabled
            },
            next_attempt_at_ms: request.enabled.then(common::time::now_ms),
            ..Default::default()
        };
        snapshot.observed_at_ms =
            common::time::now_ms().max(snapshot.observed_at_ms.saturating_add(1));
        Ok(())
    }

    fn change_monitor(
        &self,
        generation: u64,
        hub: &realtime::WsHub,
        change: impl FnOnce(&mut StockMonitorStatus),
    ) {
        let mut snapshot = self.snapshot.write();
        if self.generation.load(Ordering::SeqCst) != generation {
            return;
        }
        let previous = snapshot.monitor.clone();
        change(&mut snapshot.monitor);
        if previous == snapshot.monitor {
            return;
        }
        snapshot.observed_at_ms =
            common::time::now_ms().max(snapshot.observed_at_ms.saturating_add(1));
        drop(snapshot);
        self.publish(hub);
    }
}

async fn execute(
    service: Arc<BackpackStocks>,
    hub: realtime::WsHub,
    kind: JobKind,
    generation: u64,
) -> JobResult {
    match kind {
        JobKind::Context => {
            service.refresh_context(generation).await?;
            service.publish(&hub);
            Ok(None)
        }
        JobKind::Quote => {
            let request = service
                .snapshot
                .read()
                .monitor
                .request
                .clone()
                .ok_or("监控参数缺失")?;
            service.ensure_generation(generation, &request.asset)?;
            service.compare(request, hub).await.map(Some)
        }
    }
}

pub(super) async fn run(service: Weak<BackpackStocks>, hub: realtime::WsHub) {
    run_with(service, hub, |s, h, k, g| Box::pin(execute(s, h, k, g))).await;
}

async fn run_with<F>(service: Weak<BackpackStocks>, hub: realtime::WsHub, mut start: F)
where
    F: FnMut(Arc<BackpackStocks>, realtime::WsHub, JobKind, u64) -> BoxFuture<'static, JobResult>,
{
    let mut job: Option<BoxFuture<'static, JobResult>> = None;
    let mut job_kind = JobKind::Context;
    let mut job_generation = 0;
    let mut started_at = 0;
    let mut context_retry_at = 0;
    let mut quantity_wait: Option<(u64, Option<QuantityContext>)> = None;
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _=tick.tick()=>{},
            result=async {job.as_mut().unwrap().await},if job.is_some()=>{
                job=None;
                let Some(s)=service.upgrade() else {break};
                if s.generation.load(Ordering::SeqCst)!=job_generation {continue;}
                let now=common::time::now_ms();
                match (job_kind,result) {
                    (JobKind::Context,Ok(_))=>{
                        context_retry_at=0;
                        s.change_monitor(job_generation,&hub,|m|{if m.enabled {m.phase=StockMonitorPhase::Watching;m.next_attempt_at_ms=Some(now);m.problem=None;}});
                    },
                    (JobKind::Context,Err(error))=>{
                        context_retry_at=now+30_000;
                        *s.context_error.write()=Some(error.clone());
                        let mut snapshot=s.snapshot.write();
                        snapshot.trading_route=Some(calendar::unknown(format!("交易上下文更新失败：{error}"),None));
                        snapshot.observed_at_ms=now.max(snapshot.observed_at_ms.saturating_add(1));
                        drop(snapshot);s.publish(&hub);
                    },
                    (JobKind::Quote,Ok(snapshot))=>{
                        let comparison=snapshot.as_ref().and_then(|s|s.comparison.as_ref());
                        let problem=comparison.and_then(|c|c.sell_problem.clone());
                        let quantity_limited=comparison.is_some_and(|c|c.sell.is_none() && c.quantity_limit.is_some());
                        quantity_wait=quantity_limited.then(||(job_generation,snapshot.as_ref().and_then(quantity_context)));
                        let requested_at=comparison.map_or(started_at,|c|c.buy.requested_at_ms);
                        s.change_monitor(job_generation,&hub,|m|{
                            let interval=stock_quotes::interval_ms(m.request.as_ref().is_some_and(|r|r.keyed));
                            if quantity_limited {
                                m.phase=StockMonitorPhase::QuantityLimited;m.consecutive_failures=0;
                                m.problem=problem;m.next_attempt_at_ms=Some(now+QUANTITY_RECHECK_MS);
                                return;
                            }
                            if let Some(problem)=problem {
                                m.phase=StockMonitorPhase::Backoff;m.consecutive_failures=m.consecutive_failures.saturating_add(1);
                                m.problem=Some(problem);m.next_attempt_at_ms=Some(now+backoff_ms(m.consecutive_failures));
                                return;
                            }
                            m.phase=StockMonitorPhase::Watching;m.completed_quotes=m.completed_quotes.saturating_add(1);
                            m.consecutive_failures=0;m.problem=None;m.last_success_at_ms=Some(now);
                            m.next_attempt_at_ms=Some((requested_at+interval).max(now+250));
                        });
                    },
                    (JobKind::Quote,Err(error))=>s.change_monitor(job_generation,&hub,|m|{
                        m.phase=StockMonitorPhase::Backoff;m.consecutive_failures=m.consecutive_failures.saturating_add(1);
                        m.problem=Some(error);m.next_attempt_at_ms=Some(now+backoff_ms(m.consecutive_failures));
                    }),
                }
            }
        }
        let Some(s) = service.upgrade() else { break };
        let generation = s.generation.load(Ordering::SeqCst);
        if job.is_some() && job_generation != generation {
            job = None;
            context_retry_at = 0;
        }
        let now = common::time::now_ms();
        if s.update_route(now) {
            s.publish(&hub);
        }
        let snapshot = s.snapshot();
        if quantity_wait
            .as_ref()
            .is_some_and(|(g, c)| *g != generation || *c != quantity_context(&snapshot))
        {
            quantity_wait = None;
            s.change_monitor(generation, &hub, |m| {
                if m.enabled && m.phase == StockMonitorPhase::QuantityLimited {
                    m.phase = StockMonitorPhase::Refreshing;
                    m.next_attempt_at_ms = Some(now);
                    m.problem = None;
                }
            });
        }
        let viewers =
            hub.subscriber_count(realtime::channels::STOCKS) > 0 || s.background_monitoring();
        if !viewers || snapshot.security.is_none() {
            // Dropping the owned future cancels HTTP work, rather than detaching another task.
            job = None;
            context_retry_at = 0;
            s.change_monitor(generation, &hub, |m| {
                if m.enabled {
                    m.phase = StockMonitorPhase::WaitingForViewers;
                    m.next_attempt_at_ms = None;
                }
            });
            continue;
        }
        if job.is_some() {
            continue;
        }
        if !s.context_current(now) {
            if now >= context_retry_at {
                job_kind = JobKind::Context;
                job_generation = generation;
                started_at = now;
                job = Some(start(s.clone(), hub.clone(), job_kind, generation));
                context_retry_at = now + 30_000;
            }
            s.change_monitor(generation, &hub, |m| {
                if m.enabled {
                    m.phase = StockMonitorPhase::Backoff;
                    m.next_attempt_at_ms = Some(context_retry_at);
                    m.problem = Some("等待官方日历与合约目录更新".into());
                }
            });
            continue;
        }
        let monitor = s.snapshot.read().monitor.clone();
        if monitor.enabled && monitor.next_attempt_at_ms.is_none_or(|t| t <= now) {
            job_kind = JobKind::Quote;
            job_generation = generation;
            started_at = now;
            s.change_monitor(generation, &hub, |m| {
                m.phase = StockMonitorPhase::Refreshing
            });
            job = Some(start(s.clone(), hub.clone(), job_kind, generation));
        }
    }
}

fn backoff_ms(failures: u32) -> i64 {
    (5_000_i64.saturating_mul(1_i64 << failures.saturating_sub(1).min(4))).min(60_000)
}

#[cfg(test)]
mod tests;
