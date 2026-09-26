use super::*;

#[derive(Clone, PartialEq)]
struct TicketState {
    action: BuildActionState,
    badge: &'static str,
    tone: &'static str,
    label: &'static str,
    title: String,
    blocker: String,
    evidence: bool,
    transfer_missing: bool,
    evidence_label: &'static str,
    evidence_tone: &'static str,
    request: Option<OnchainExecutionBuildRequest>,
}

impl TicketState {
    fn blocked(badge: &'static str, label: &'static str, blocker: String) -> Self {
        Self {
            action: BuildActionState::Blocked,
            badge,
            tone: "is-warning",
            label,
            title: blocker.clone(),
            blocker,
            evidence: false,
            transfer_missing: false,
            evidence_label: "待核对",
            evidence_tone: "is-warning",
            request: None,
        }
    }

    fn from_state(
        state: &LoadState<OnchainComparisonSnapshot>,
        direction: OnchainComparisonDirection,
    ) -> Self {
        if let Some(problem) = state.problem() {
            return Self::blocked("状态待确认", "等待最新快照", problem.message.clone());
        }
        let Some(snapshot) = state.value() else {
            return Self::blocked("等待报价", "等待实时报价", "正在读取当前监控".into());
        };
        if !snapshot.config.enabled {
            let (_, detail, _) = inactive_next_step(snapshot);
            return Self::blocked("监控暂停", "启用套利监控", detail);
        }
        let Some(comparison) = snapshot
            .comparisons
            .iter()
            .find(|row| row.direction == direction)
        else {
            return Self::blocked(
                "等待报价",
                "等待实时报价",
                snapshot
                    .cex_problem
                    .clone()
                    .or_else(|| snapshot.provider_problem.clone())
                    .unwrap_or_else(|| "当前方向尚无双源实时报价".into()),
            );
        };
        let Some(row) = readiness_for(snapshot, direction) else {
            return Self::blocked(
                "数据依据待核对",
                "补齐执行数据依据",
                "当前方向尚无执行准备度数据依据".into(),
            );
        };
        let market_blocker =
            direction_market_gate(snapshot, comparison).map(|(_, _, detail)| detail);
        if raw_observation_mode(snapshot) {
            let mut state = Self::blocked(
                "仅观察",
                "仅观察",
                raw_observation_next_step(&snapshot.config),
            );
            state.title =
                market_blocker.unwrap_or_else(|| raw_observation_detail(&snapshot.config));
            return state;
        }
        let request = snapshot
            .quote_observed_at_ms
            .zip(snapshot.cex_observed_at_ms)
            .map(|(quote, cex)| OnchainExecutionBuildRequest {
                direction,
                expected_quote_observed_at_ms: quote,
                expected_cex_observed_at_ms: cex,
                replenishment_run_ids: Vec::new(),
                approval_run_ids: Vec::new(),
            });
        let setup = market_blocker.is_none()
            && !row.build_ready
            && (!snapshot.execution_readiness.wallet_address_configured
                || !snapshot.execution_readiness.chain_submission_ready);
        let buildable = market_blocker.is_none() && row.build_ready && request.is_some();
        let replenishable = market_blocker.is_none()
            && row.path.availability == OnchainPathAvailability::Replenishable
            && request.is_some();
        let depth = depth_probe_note(snapshot, comparison);
        let blocker = if buildable {
            depth.clone().or_else(|| row.blockers.first().cloned())
        } else {
            market_blocker
                .or_else(|| path_inventory_guidance(row))
                .or_else(|| row.blockers.first().cloned())
        }
        .unwrap_or_else(|| if buildable {
            "可构建交易检查；余额、充提与盘口仍以本次计划核对为准，尚未下单".into()
        } else {
            "执行条件尚待核对".into()
        });
        let action = if setup {
            BuildActionState::SetupRequired
        } else if replenishable {
            BuildActionState::Replenishable
        } else if buildable {
            BuildActionState::Buildable
        } else {
            BuildActionState::Blocked
        };
        let (badge, tone) = execution_state_badge(action);
        let (evidence_label, evidence_tone) =
            execution_overall_state(snapshot, comparison, row, buildable);
        let transfer_missing = row.path.replenishment.iter().any(|evidence| {
            matches!(
                evidence.status,
                OnchainTransferStatus::Unknown | OnchainTransferStatus::Refreshing
            )
        }) || row
            .inventory
            .iter()
            .any(|evidence| evidence.status != OnchainInventoryStatus::Ready);
        let title = match action {
            BuildActionState::SetupRequired => {
                "打开执行接入，填写公开钱包地址并配置当前链签名器".into()
            }
            BuildActionState::Replenishable => {
                "核对充提网络、费用和目标地址；生成计划，不会提币".into()
            }
            BuildActionState::Buildable => {
                "重新读取 firm quote、按需核对 交易所 深度并构建双腿计划".into()
            }
            BuildActionState::Blocked => blocker.clone(),
        };
        Self {
            action,
            badge,
            tone,
            title,
            blocker,
            evidence: true,
            transfer_missing,
            evidence_label,
            evidence_tone,
            request,
            label: build_action_label(
                snapshot.quality,
                comparison.net_spread_bps,
                snapshot.config.spread_alert.min_net_spread_bps,
                action,
                depth.is_some(),
                false,
            ),
        }
    }
}

pub(super) fn ticket(
    data: OnchainData,
    open_execution_setup: Callback<()>,
    direction: PreviewContext,
    show_execution_result: Callback<()>,
    evidence_open: RwSignal<bool>,
) -> impl IntoView {
    let model = Memo::new(move |_| {
        data.state
            .with(|state| TicketState::from_state(state, direction.get()))
    });
    let config = Memo::new(move |_| {
        data.state.with(|state| {
            state
                .value()
                .map(|snapshot| snapshot.config.clone())
                .unwrap_or_default()
        })
    });
    let busy = move || {
        data.saving.get()
            || data.execution.building_execution.get()
            || data.execution.building_approval.get()
            || data.replenishment.building.get()
            || data.execution.submitting_execution.get()
            || data.execution.submitting_approval.get()
    };
    view! {
        <div class="onchain-direction-execution" aria-label=move || format!("{}执行准备度", direction_label(direction.get()))>
            <header class="onchain-execution-heading">
                <span><small>"执行判断"</small><strong>{move || direction_label(direction.get())}</strong></span>
                <em class=move || model.with(|model| model.tone)>{move || model.with(|model| model.badge)}</em>
            </header>
            {move || data.state.with(|state| {
                state.value().filter(|snapshot| snapshot.config.enabled).and_then(|snapshot| {
                    snapshot.comparisons.iter().find(|row| row.direction == direction.get())
                        .map(|row| execution_ticket_metrics(snapshot, row).into_any())
                }).unwrap_or_else(|| view! {
                    <dl class="onchain-ticket-summary" aria-label="执行规模与预估收益">
                        <div class="is-primary"><dt>"预估净收益"</dt><dd class="num">"--"</dd></div>
                        <div><dt>"本次可做"</dt><dd class="num">"--"</dd></div>
                    </dl>
                }.into_any())
            })}
            {move || data.state.with(|state| state.value().filter(|snapshot| snapshot.config.enabled).and_then(|snapshot| {
                let mut snapshot = snapshot.clone();
                if state.problem().is_some() {
                    snapshot.quality = OnchainComparisonQuality::Stale;
                }
                let comparison = snapshot.comparisons.iter().find(|row| row.direction == direction.get())?;
                let row = readiness_for(&snapshot, direction.get())?;
                Some(execution_readiness_strip(&snapshot, comparison, row).into_any())
            }))}
            <Show when=move || model.with(|model| model.evidence)>
                <details class="onchain-ticket-evidence" open=move || evidence_open.get()>
                    <summary on:click=move |event| {
                        event.prevent_default();
                        let opening = !evidence_open.get_untracked();
                        evidence_open.set(opening);
                        if opening && model.with_untracked(|model| model.transfer_missing)
                            && data.transfer_refreshing.try_get_untracked() == Some(false) {
                            data.refresh_transfer_networks.run(());
                        }
                    }>
                        <span>"费用与执行数据依据"</span>
                        <strong class=move || model.with(|model| model.evidence_tone)>
                            {move || if data.transfer_refreshing.get() { "充提读取中" } else { model.with(|model| model.evidence_label) }}
                        </strong>
                    </summary>
                    {move || data.state.with(|state| state.value().and_then(|snapshot| {
                        let comparison = snapshot.comparisons.iter().find(|row| row.direction == direction.get())?;
                        let row = readiness_for(snapshot, direction.get())?;
                        let (instrument, detail) = cex_instrument_label(&row.cex_instrument);
                        let title = detail.unwrap_or_else(|| instrument.clone());
                        Some(view! {
                            {execution_cost_breakdown(comparison)}
                            <div class="onchain-inventory-set">
                                {row.inventory.iter().map(inventory_chip).collect_view()}
                                {row.path.replenishment.iter().map(transfer_chip).collect_view()}
                                <span class=format!("onchain-inventory-chip {}", cex_instrument_tone(&row.cex_instrument)) title=title>
                                    <small>"执行规格"</small><strong>{instrument}</strong>
                                </span>
                            </div>
                        })
                    }))}
                </details>
            </Show>
            <small class="onchain-execution-blocker" title=move || model.with(|model| model.blocker.clone())>
                {move || model.with(|model| model.blocker.clone())}
            </small>
            <Show when=move || model.with(|model| model.evidence)>
                {replenishment_cost_selection::selection(data)}
                {approval_cost_selection::selection(data, config, direction)}
            </Show>
            <button type="button" class="row-action onchain-build-action"
                disabled=move || model.with(|model| model.action == BuildActionState::Blocked) || busy()
                title=move || model.with(|model| model.title.clone())
                on:click=move |_| {
                    if untrack(busy) { return; }
                    let state = data.current_state();
                    let current = TicketState::from_state(&state, direction.get_untracked());
                    match current.action {
                        BuildActionState::SetupRequired => open_execution_setup.run(()),
                        BuildActionState::Replenishable => {
                            if let Some(request) = current.request {
                                data.replenishment.build.run(OnchainReplenishmentBuildRequest {
                                    direction: request.direction,
                                    expected_quote_observed_at_ms: request.expected_quote_observed_at_ms,
                                    expected_cex_observed_at_ms: request.expected_cex_observed_at_ms,
                                });
                                show_execution_result.run(());
                            }
                        }
                        BuildActionState::Buildable => {
                            if let Some(mut request) = current.request {
                                request.replenishment_run_ids = data.execution.selected_replenishment.get_untracked();
                                request.approval_run_ids = data.execution.selected_approvals.get_untracked();
                                data.execution.build_execution.run(request);
                                show_execution_result.run(());
                            }
                        }
                        BuildActionState::Blocked => {}
                    }
                }
            >{move || if data.execution.building_execution.get() { "构建中…" }
                else if data.execution.building_approval.get() { "核对授权中…" }
                else if data.replenishment.building.get() { "核对补仓中…" }
                else { model.with(|model| model.label) }}
            </button>
        </div>
    }
}
