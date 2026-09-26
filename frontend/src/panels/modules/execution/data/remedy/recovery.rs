use crate::api::rest::{with_mutation_timeout, ApiClient, MutationRequestContext};
use super::super::connection::ExecutionConnection;
use leptos::{prelude::*, task::spawn_local};
use serde::{Deserialize, Serialize};
use shared_types::{ActionEvidence, ActionState, ApiProblem, ExecutionRun, OrderRecord};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CancelRequest {
    pub order_id: String,
    pub exchange: String,
    pub symbol: String,
    pub context: MutationRequestContext,
    pub sent: bool,
    #[serde(default)]
    pub account_rejected: bool,
}

impl CancelRequest {
    pub(super) fn matches(&self, order: &OrderRecord) -> bool {
        self.order_id == order.intent.id
            && self.exchange == order.intent.exchange
            && self.symbol == order.intent.symbol
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PendingCancel {
    version: u8,
    run_id: String,
    ticket_id: String,
    opportunity_id: String,
    pub requests: Vec<CancelRequest>,
}

impl PendingCancel {
    fn recorded_order<'a>(&self, request: &CancelRequest, rows: &'a [OrderRecord]) -> Option<&'a OrderRecord> {
        if let Some(row) = rows.iter().find(|row| request.matches(row)) {
            return Some(row);
        }
        // Old batches may contain several aliases for one order. Require the same
        // batch's exact internal-ID request before using an alias, never venue alone.
        let mut matches = rows.iter().filter(|row| {
            request.exchange == row.intent.exchange && request.symbol == row.intent.symbol
                && self.requests.iter().any(|original| original.sent && !original.account_rejected && original.matches(row))
                && row.identity_snapshot().matches_order_id(&request.order_id)
        });
        let row = matches.next()?;
        matches.next().is_none().then_some(row)
    }

    fn evidence(&self) -> ActionEvidence {
        let mut evidence = ActionEvidence::default();
        evidence.run_id = Some(self.run_id.clone());
        evidence.ticket_id = Some(self.ticket_id.clone());
        for request in self.requests.iter().filter(|r| r.sent) {
            evidence.merge(request.context.evidence());
            evidence.order_ids.push(request.order_id.clone());
            evidence.venues.push(request.exchange.clone());
            evidence.symbols.push(request.symbol.clone());
        }
        evidence
    }
}

/// One durable batch; reload only reads sent orders and never resumes queued writes.
#[derive(Clone, Copy)]
pub(in crate::panels::modules::execution) struct CancelRecovery {
    pending: RwSignal<Option<PendingCancel>>,
    pub busy: RwSignal<bool>,
    pub state: RwSignal<ActionState>,
    pub problem: RwSignal<Option<String>>,
    scope: StoredValue<String>,
    connection: ExecutionConnection,
}

impl CancelRecovery {
    pub(in crate::panels::modules::execution) fn new() -> Self {
        let connection = expect_context::<ExecutionConnection>();
        let recovery = Self {
            pending: RwSignal::new(None),
            busy: RwSignal::new(false),
            state: RwSignal::new(ActionState::Idle),
            problem: RwSignal::new(None),
            scope: StoredValue::new(connection.key("crossline.execution.pendingCancel.v1")),
            connection,
        };
        recovery.restore();
        Effect::new(move |_| {
            if !connection.available() {
                recovery.problem.set(Some(
                    "连接已改变，请刷新页面；原撤单记录仍保留在原连接下。".into(),
                ));
            }
        });
        recovery
    }

    pub(super) fn current(self) -> bool {
        self.connection.current()
    }

    pub(super) fn client(self) -> ApiClient {
        self.connection.client()
    }

    pub(in crate::panels::modules::execution) fn blocked(self) -> bool {
        !self.connection.available() || self.busy.get() || self.pending.with(Option::is_some) || self.problem.with(Option::is_some)
    }

    fn restore(self) {
        match storage::load(&self.scope.get_value()) {
            Ok(pending) => {
                if let Some(batch) = &pending {
                    self.state.set(
                        ActionState::accepted("原撤单结果待核对").with_evidence(batch.evidence()),
                    );
                }
                self.pending.set(pending);
                self.problem.set(None);
            }
            Err(message) => self.problem.set(Some(message)),
        }
    }

    pub(super) fn begin(self, run: &ExecutionRun, ids: &[String]) -> Option<PendingCancel> {
        if self.blocked() || !self.current() {
            return None;
        }
        let requests = ids
            .iter()
            .map(|id| {
                let leg = [&run.long_leg, &run.short_leg]
                    .into_iter()
                    .find(|leg| leg.order_ids.contains(id)
                        || leg.identity.as_ref().is_some_and(|identity| identity.internal_order_id == *id))?;
                Some(CancelRequest {
                    order_id: id.clone(),
                    exchange: leg.exchange.clone(),
                    symbol: leg.symbol.clone(),
                    context: MutationRequestContext::new_idempotent_attempt(format!(
                        "execution-cancel:{}:{id}",
                        run.run_id
                    )),
                    sent: false,
                    account_rejected: false,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        let batch = PendingCancel {
            version: 1,
            run_id: run.run_id.clone(),
            ticket_id: run.ticket_id.clone(),
            opportunity_id: run.opportunity_id.clone(),
            requests,
        };
        if !self.persist(&batch, None) {
            return None;
        }
        self.busy.set(true);
        self.state
            .set(ActionState::pending("撤单提交中").with_evidence(batch.evidence()));
        Some(batch)
    }

    pub(super) fn mark_sent(self, batch: &mut PendingCancel, index: usize) -> bool {
        if !self.current() {
            return false;
        }
        let old = batch.clone();
        batch.requests[index].sent = true;
        self.persist(batch, Some(&old))
    }

    pub(super) fn account_rejected(self, batch: &mut PendingCancel, index: usize) {
        let old = batch.clone();
        batch.requests[index].account_rejected = true;
        self.persist(batch, Some(&old));
    }

    fn persist(self, batch: &PendingCancel, previous: Option<&PendingCancel>) -> bool {
        match storage::write(&self.scope.get_value(), previous, Some(batch)) {
            Ok(()) => {
                self.pending.set(Some(batch.clone()));
                true
            }
            Err(message) => {
                self.problem.set(Some(message));
                false
            }
        }
    }

    pub(super) fn accept(self, rows: &[OrderRecord]) -> bool {
        if !self.current() || self.busy.get_untracked() {
            return false;
        }
        let Some(batch) = self.pending.get_untracked() else {
            return false;
        };
        let sent = batch.requests.iter().filter(|r| r.sent && !r.account_rejected).collect::<Vec<_>>();
        let mut matching = Vec::new();
        for request in &sent {
            let Some(row) = batch.recorded_order(request, rows) else { return false; };
            if !matching.iter().any(|known: &OrderRecord| known.intent.id == row.intent.id) {
                matching.push(row.clone());
            }
        }
        let mut submitted_evidence = batch.evidence();
        submitted_evidence.order_ids = matching.iter().map(|row| row.intent.id.clone()).collect();
        let Some(mut state) = (if sent.is_empty() {
            Some(ActionState::succeeded(
                "撤单未发送，可重新选择仍未成交的订单",
            ))
        } else {
            super::settled_cancel_state(&submitted_evidence, &matching)
        }) else {
            return false;
        };
        let rejected = batch.requests.iter().filter(|r| r.account_rejected).count();
        let queued = batch.requests.iter().filter(|r| !r.sent && batch.recorded_order(r, &matching).is_none()).count();
        if rejected > 0 {
            let settled = if sent.is_empty() { String::new() } else {
                format!("{}；{}", state.label().unwrap_or_default(),
                    state.problem().map(|problem| format!("{}；", problem.message)).unwrap_or_default())
            };
            let label = if sent.is_empty() { "撤单未发送：账户不匹配" } else { "撤单部分完成：账户不匹配" };
            state = ActionState::failed(label, ApiProblem::new(
                shared_types::problem::codes::ORDER_ACCOUNT_MISMATCH,
                format!("{settled}{rejected} 笔因账户归属未通过被拒绝，{queued} 笔未发送；恢复原账户后核对挂单，旧版缺少归属的订单需在交易所核对。"),
            )).with_evidence(batch.evidence());
        }
        if queued > 0 && rejected == 0 {
            // This is not a retry instruction: the next user action re-reads remaining orders.
            state = ActionState::failed(
                format!("{}；{queued} 笔未发送", state.label().unwrap_or_default()),
                ApiProblem::new(
                    "CANCEL_BATCH_INTERRUPTED",
                    "提交被中断；未发送的订单不会在刷新后自动撤销，请核对剩余挂单。",
                ),
            )
            .with_evidence(batch.evidence());
        }
        match storage::write(&self.scope.get_value(), Some(&batch), None) {
            Ok(()) => {
                self.pending.set(None);
                self.problem.set(None);
                self.state.set(state);
                true
            }
            Err(message) => {
                self.problem.set(Some(message));
                false
            }
        }
    }

    pub(super) fn unknown(self, message: String) {
        if !self.current() {
            return;
        }
        let Some(batch) = self.pending.get_untracked() else {
            return;
        };
        self.state
            .set(ActionState::accepted("撤单结果待核对").with_evidence(batch.evidence()));
        self.problem.set(Some(message));
    }

    pub(in crate::panels::modules::execution) fn panel(
        self,
        queue: RwSignal<super::super::orders::OrderQueue>,
        has_selection: Memo<bool>,
    ) -> impl IntoView {
        view! {
            <Show when=move || self.blocked() || (!has_selection.get() && !matches!(self.state.get(), ActionState::Idle))>
                <section class="execution-actionbar execution-recovery" role="region" aria-label="原撤单核对">
                    <div class="run-state">
                        <span>{move || if self.busy.get() { "正在处理原撤单".into() } else if self.blocked() { "原撤单结果待核对".into() }
                            else { self.state.get().label().unwrap_or_default().to_owned() }}</span>
                        <em class="run-state-detail">{move || self.problem.get().or_else(|| self.state.get().problem().map(|problem| problem.message.clone()))
                            .unwrap_or_else(|| "只查询原订单；已成交部分仍需在持仓页处理。".into())}</em>
                        <details class="execution-disclosure"><summary>"原撤单详情"</summary>
                            <p>{move || self.state.get().message("")}</p>
                        </details>
                    </div>
                    <Show when=move || self.blocked()>
                        <button type="button" class="dryrun-action" disabled=move || self.busy.get() || !self.current()
                            on:click=move |_| self.recheck(queue)>"核对原撤单"</button>
                    </Show>
                </section>
            </Show>
        }.into_any()
    }

    fn recheck(self, queue: RwSignal<super::super::orders::OrderQueue>) {
        if self.busy.get_untracked() || !self.current() {
            return;
        }
        if self.pending.get_untracked().is_none() {
            self.restore();
        }
        let Some(batch) = self.pending.get_untracked() else {
            return;
        };
        self.busy.set(true);
        let client = self.client();
        spawn_local(async move {
            let mut rows = Vec::new();
            let mut problem = None;
            for request in batch.requests.iter().filter(|r| r.sent && !r.account_rejected) {
                if !self.current() {
                    break;
                }
                if batch.recorded_order(request, &rows).is_some_and(|row| row.intent.id != request.order_id) {
                    continue;
                }
                match with_mutation_timeout("核对原撤单", client.trading_order(&request.order_id))
                    .await
                {
                    Ok(row) if request.matches(&row) => rows.push(row),
                    Ok(_) => problem = Some("订单身份不匹配，未采用该处理结果。".into()),
                    Err(error) => problem = Some(error.problem.message),
                }
            }
            if self.busy.try_get_untracked().is_none() {
                return;
            }
            self.busy.set(false);
            if !self.current() {
                return;
            }
            queue.update(|queue| {
                for row in &rows {
                    queue.apply_receipt(row.clone());
                }
            });
            let latest = queue.with_untracked(|queue| {
                batch
                    .requests
                    .iter()
                    .filter_map(|request| queue.order(&request.order_id).cloned())
                    .collect::<Vec<_>>()
            });
            if !self.accept(&latest) {
                self.unknown(
                    problem.unwrap_or_else(|| "原订单尚未确认最终结果或成交量；不重复撤单。".into()),
                );
            }
        });
    }
}

mod storage {
    use super::PendingCancel;
    #[cfg(target_arch = "wasm32")]
    fn session() -> Result<web_sys::Storage, String> {
        web_sys::window()
            .ok_or("浏览器不可用")?
            .session_storage()
            .map_err(|_| "无法读取撤单恢复记录")?
            .ok_or_else(|| "会话存储不可用，未发送撤单。".into())
    }
    pub(super) fn load(key: &str) -> Result<Option<PendingCancel>, String> {
        #[cfg(target_arch = "wasm32")]
        {
            let Some(raw) = session()?
                .get_item(key)
                .map_err(|_| "无法读取撤单恢复记录")?
            else {
                return Ok(None);
            };
            let batch: PendingCancel =
                serde_json::from_str(&raw).map_err(|_| "撤单记录损坏，请先核对后台订单。")?;
            let id = |s: &str| !s.is_empty() && s.len() <= 2048 && !s.chars().any(char::is_control);
            if batch.version != 1
                || !id(&batch.run_id)
                || !id(&batch.ticket_id)
                || !id(&batch.opportunity_id)
                || batch.requests.is_empty()
                || batch.requests.len() > 64
                || batch.requests.iter().enumerate().any(|(i, r)| {
                    !id(&r.order_id)
                        || !id(&r.exchange)
                        || !id(&r.symbol)
                        || !id(r.context.request_id())
                        || !r.context.idempotency_key().is_some_and(|key| {
                            key.starts_with(&format!(
                                "execution-cancel:{}:{}:",
                                batch.run_id, r.order_id
                            ))
                        })
                        || batch.requests[..i]
                            .iter()
                            .any(|other| other.order_id == r.order_id)
                })
            {
                return Err("撤单记录身份不完整，已暂停修改。".into());
            }
            Ok(Some(batch))
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = key;
            Err("撤单恢复需要浏览器会话存储".into())
        }
    }
    pub(super) fn write(
        key: &str,
        old: Option<&PendingCancel>,
        next: Option<&PendingCancel>,
    ) -> Result<(), String> {
        if load(key)?.as_ref() != old {
            return Err("原撤单记录已变化，未覆盖其他请求。".into());
        }
        #[cfg(target_arch = "wasm32")]
        {
            let storage = session()?;
            match next {
                Some(batch) => storage.set_item(
                    key,
                    &serde_json::to_string(batch).map_err(|_| "无法保存原撤单")?,
                ),
                None => storage.remove_item(key),
            }
            .map_err(|_| "无法保存撤单恢复记录，已暂停修改。")?;
            if load(key)?.as_ref() != next {
                return Err("撤单恢复记录未完整保存，已暂停修改。".into());
            }
            Ok(())
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = next;
            Err("撤单恢复需要浏览器会话存储".into())
        }
    }
}
