use leptos::prelude::*;
use shared_types::{
    OnchainReplenishmentDestinationStatus, OnchainReplenishmentLeg,
    OnchainReplenishmentPlanResponse, OnchainReplenishmentPlanStatus, OnchainReplenishmentRun,
    OnchainReplenishmentRunStatus, OnchainReplenishmentTransferStatus, OnchainTransferDirection,
    ONCHAIN_REPLENISHMENT_AUTHORIZATION_PHRASE,
};

use super::super::data::{OnchainData, OnchainReplenishmentData};
use super::super::format::usd;

pub(super) fn replenishment_panel(data: OnchainData, clock_ms: RwSignal<i64>) -> AnyView {
    let plan = Memo::new(move |_| data.replenishment.plan.get());
    let run = Memo::new(move |_| data.replenishment.run.get());
    view! {
        <Show when=move || plan.with(Option::is_some) || run.with(Option::is_some)
            || data.replenishment.building.get() || data.replenishment.recovery_problem.get().is_some()>
        <section class="onchain-replenishment-stack" aria-label="库存补充">
            {move || recovery_notice(data.replenishment.recovery_problem.get())}
            {move || plan_panel(plan.get(), data.replenishment.building.get(), data, clock_ms)}
            {move || run_panel(run.get(), data, clock_ms)}
        </section>
        </Show>
    }
    .into_any()
}

fn recovery_notice(problem: Option<String>) -> impl IntoView {
    problem.map(|problem| {
        view! {
            <div class="onchain-replenishment-recovery" role="alert">
                <strong>"补仓恢复需要检查"</strong>
                <p>{problem}</p>
            </div>
        }
    })
}

fn plan_panel(
    result: Option<Result<OnchainReplenishmentPlanResponse, String>>,
    building: bool,
    data: OnchainData,
    clock_ms: RwSignal<i64>,
) -> AnyView {
    let Some(result) = result else {
        return building
            .then(|| {
                view! {
                    <div class="onchain-replenishment-plan is-warning" role="status">
                        <strong>"正在核验补仓路径"</strong>
                        <span>"读取官方网络、费用、数量步长与目标地址…"</span>
                    </div>
                }
            })
            .into_any();
    };
    match result {
        Err(problem) => failure_row("补仓计划未生成", problem),
        Ok(plan) => ready_plan(plan, data, clock_ms),
    }
}

fn ready_plan(
    plan: OnchainReplenishmentPlanResponse,
    data: OnchainData,
    clock_ms: RwSignal<i64>,
) -> AnyView {
    let (status_label, tone) = plan_state(plan.status);
    let route = if plan.legs.is_empty() {
        "补仓腿缺失".to_owned()
    } else {
        plan.legs
            .iter()
            .map(route_label)
            .collect::<Vec<_>>()
            .join("；")
    };
    let amount = match plan.legs.as_slice() {
        [leg] => amount_label(leg),
        legs if !legs.is_empty() => format!("{} 条顺序资金腿", legs.len()),
        _ => "--".to_owned(),
    };
    let fee = match plan.legs.as_slice() {
        [leg] => fee_label(leg),
        _ => plan
            .transfer_cost_usd
            .map(usd)
            .unwrap_or_else(|| "总成本待核验".to_owned()),
    };
    let destination = match plan.legs.as_slice() {
        [leg] => destination_label(leg),
        legs if !legs.is_empty() => format!(
            "{}/{} 个目标已核验",
            legs.iter()
                .filter(|leg| {
                    leg.destination.status == OnchainReplenishmentDestinationStatus::Verified
                })
                .count(),
            legs.len()
        ),
        _ => "目标待核验".to_owned(),
    };
    let problem = plan
        .blockers
        .first()
        .cloned()
        .unwrap_or_else(|| "补仓范围已固定；授权后仍会在提交前重新核验".to_owned());
    let authorization_ready = plan.status == OnchainReplenishmentPlanStatus::ReadyForAuthorization
        && plan.submit_ready
        && plan.requires_live_authorization && plan.blockers.is_empty();
    let valid_until_ms = plan.valid_until_ms;
    let confirmation = data.replenishment.confirmation;
    let route_title = route.clone();
    let destination_title = destination.clone();
    let problem_title = problem.clone();
    view! {
        <div class=format!("onchain-replenishment-plan {tone}") role="status">
            <div class="onchain-replenishment-heading">
                <small>"库存补充计划"</small>
                <strong>{move || if clock_ms.get() >= valid_until_ms { "计划已过期" } else { status_label }}</strong>
                <span class="num">{plan.post_transfer_net_profit_usd.map_or_else(|| "收益待核算".to_owned(), usd)}</span>
            </div>
            <dl class="onchain-replenishment-facts">
                <div><dt>"路径"</dt><dd title=route_title>{route}</dd></div>
                <div><dt>"数量"</dt><dd class="num">{amount}</dd></div>
                <div><dt>"成本"</dt><dd>{fee}</dd></div>
                <div><dt>"目标"</dt><dd title=destination_title>{destination}</dd></div>
            </dl>
            <div class="onchain-replenishment-authorization">
                <input
                    aria-label="实盘补仓授权口令"
                    placeholder=ONCHAIN_REPLENISHMENT_AUTHORIZATION_PHRASE
                    bind:value=confirmation
                    disabled=move || { !authorization_ready || clock_ms.get() >= valid_until_ms
                        || data.saving.get() || data.replenishment.authorizing.get() }
                />
                <button
                    type="button"
                    class="row-action"
                    disabled=move || {
                        !authorization_ready
                            || !data.replenishment.loaded.get()
                            || data.saving.get() || data.replenishment.submitting.get() || data.replenishment.rechecking.get()
                            || clock_ms.get() >= valid_until_ms
                            || confirmation.get() != ONCHAIN_REPLENISHMENT_AUTHORIZATION_PHRASE
                            || data.replenishment.authorizing.get()
                            || data.replenishment.recovery_problem.get().is_some()
                    }
                    on:click=move |_| data.replenishment.authorize.run(confirmation.get_untracked())
                >
                    {move || if data.replenishment.authorizing.get() { "授权中…" } else { "授权 60 秒" }}
                </button>
            </div>
            <small class="onchain-replenishment-problem" title=problem_title>{problem}</small>
        </div>
    }
    .into_any()
}

fn run_panel(
    result: Option<Result<OnchainReplenishmentRun, String>>,
    data: OnchainData,
    clock_ms: RwSignal<i64>,
) -> AnyView {
    let Some(result) = result else {
        return ().into_any();
    };
    match result {
        Err(problem) => failure_row("补仓结果待确认", problem),
        Ok(run) => run_status(run, data.replenishment, clock_ms).into_any(),
    }
}

fn run_status(
    run: OnchainReplenishmentRun,
    data: OnchainReplenishmentData,
    clock_ms: RwSignal<i64>,
) -> impl IntoView {
    let current_leg_index = if run.status == OnchainReplenishmentRunStatus::ReadyForNextTransfer {
        run.transfers.len()
    } else {
        run.transfers
            .last()
            .map(|transfer| transfer.leg_index as usize)
            .unwrap_or(0)
    };
    let direction = run
        .plan
        .legs
        .get(current_leg_index)
        .or_else(|| run.plan.legs.first())
        .map(|leg| leg.direction);
    let submit_ready = run.status == OnchainReplenishmentRunStatus::AuthorizedAwaitingSubmit
        && !run.read_only_recovery;
    let transfer = run.transfers.last().cloned();
    let valuation_pending = run.status == OnchainReplenishmentRunStatus::ReadyForNextTransfer
        && run.transfers.iter().any(|transfer| {
            transfer.withdrawal_cost.as_ref().is_some_and(|cost| cost.confirmed && cost.usd_valuation.is_none())
                || transfer.network_cost.as_ref().is_some_and(|cost| cost.total_fee_exact.is_some() && cost.usd_valuation.is_none())
        });
    let (label, tone) = if valuation_pending {
        ("等待费用核算", "is-warning")
    } else if run.status == OnchainReplenishmentRunStatus::AwaitingDestinationCredit
        && transfer
            .as_ref()
            .is_some_and(|row| row.withdrawal_unlocked == Some(false))
    {
        ("已入账 · 待解锁", "is-warning")
    } else {
        run_state(run.status, direction)
    };
    let credited = transfer.as_ref().and_then(|row| {
        let leg = run.plan.legs.get(row.leg_index as usize)?;
        let amount = row.credited_amount_exact.as_deref()?;
        let availability = match row.withdrawal_unlocked {
            Some(false) => " · 可交易，提币待解锁",
            Some(true) => " · 充值已完成",
            None if leg.direction == OnchainTransferDirection::DepositToCex => {
                " · 提币可用性另行核验"
            }
            None => " · 链上已确认",
        };
        Some(format!("实际到账 {amount} {}{availability}", leg.asset))
    });
    let transfer_label = transfer.as_ref().map_or_else(
        || "尚未提交资金动作".to_owned(),
        |row| {
            let provider = row
                .provider_transfer_id
                .as_deref()
                .map(compact_id)
                .unwrap_or_else(|| compact_id(&row.client_transfer_id));
            format!("{} · {provider}", transfer_state(row.status, direction))
        },
    );
    let deposit_review = transfer.as_ref().and_then(|row| {
        let amount = row.reported_deposit_amount_exact.as_deref()?;
        let fee = row.deposit_fee_exact.as_deref()?;
        let asset = &run.plan.legs.get(row.leg_index as usize)?.asset;
        Some(format!(
            "平台记录 {amount} {asset} · 充值费 {fee} {asset} · 净到账待核实"
        ))
    });
    let withdrawal_fee = transfer
        .as_ref()
        .and_then(|row| row.withdrawal_cost.as_ref())
        .map(|cost| {
            let label = if cost.confirmed {
                "实扣提币费"
            } else {
                "提币费暂报，尚未完成"
            };
            let valuation = if cost.confirmed {
                cost.usd_valuation.as_ref().map_or_else(
                    || " · 美元折算待核".to_owned(),
                    |value| format!(" · 折算 ${}", value.usd_amount_exact),
                )
            } else { String::new() };
            let mut source = cost.source.clone();
            if let Some(value) = &cost.usd_valuation {
                source.push_str(&format!(" · 折算时间 {}", value.valued_at_ms));
                if let Some(quote) = &value.quote {
                    source.push_str(&format!(" · {}/{} ask {}", quote.venue, quote.symbol, quote.usd_ask));
                }
            }
            (format!("{label} {} {}{valuation}", cost.fee_exact, cost.asset), source)
        });
    let evidence = transfer
        .as_ref()
        .and_then(|row| row.evidence_source.clone())
        .unwrap_or_else(|| "持久状态机".to_owned());
    let network_fee = transfer
        .as_ref()
        .and_then(|row| row.network_cost.as_ref())
        .map(|cost| {
            let text = cost.total_fee_exact.as_ref().map_or_else(
                || {
                    cost.execution_fee_exact.as_ref().map_or_else(
                        || "网络费待核验".to_owned(),
                        |fee| format!("已知执行费 {fee} {} · 网络总费待核验", cost.asset),
                    )
                },
                |fee| {
                    let value = cost.usd_valuation.as_ref().map_or_else(
                        || "美元折算待核".to_owned(),
                        |value| format!("折算 ${}", value.usd_amount_exact),
                    );
                    format!("实扣网络费 {fee} {} · {value}", cost.asset)
                },
            );
            let mut source = cost.problem.as_ref().map_or_else(
                || cost.source.clone(),
                |problem| format!("{problem} · {}", cost.source),
            );
            if let Some(quote) = cost
                .usd_valuation
                .as_ref()
                .and_then(|value| value.quote.as_ref())
            {
                source.push_str(&format!(
                    " · {}/{} ask {} · 汇率时间 {}",
                    quote.venue, quote.symbol, quote.usd_ask, quote.observed_at_ms
                ));
            }
            (text, source)
        })
        .or_else(|| {
            (direction == Some(OnchainTransferDirection::DepositToCex)
                && transfer
                    .as_ref()
                    .is_some_and(|row| row.transaction_id.is_some()))
            .then(|| {
                (
                    "网络费待核验".to_owned(),
                    "尚无该交易的实扣网络费记录，不代表免费".to_owned(),
                )
            })
        });
    let problem = run.problem.clone();
    let progress = run_progress(&run);
    let run_id_title = run.run_id.clone();
    let run_id_label = compact_id(&run.run_id);
    let transfer_title = transfer_label.clone();
    let wait_run = run.clone();
    let evidence_title = evidence.clone();
    let next_action = if valuation_pending {
        "实际费用的美元汇率待就绪；保留已到账记录，暂不执行下一步".to_owned()
    } else { run.next_action.clone() };
    let next_action_title = next_action.clone();
    let submit = submit_ready.then(|| {
        submit_button(
            data,
            clock_ms,
            run.authorization.valid_until_ms,
            run.run_id.clone(),
            direction,
        )
    });
    let recheck = run.recheck_request().map(|request| {
        view! {
            <button type="button" class="row-action"
                disabled=move || data.rechecking.get() || data.submitting.get() || data.authorizing.get() || data.recovery_problem.get().is_some()
                on:click=move |_| data.recheck.run(request.clone())>
                {move || if data.rechecking.get() { "正在核验…" } else { "重新核验原转账" }}
            </button>
        }
    });
    let missing_provider_receipt = run.status == OnchainReplenishmentRunStatus::Paused
        && transfer.as_ref().is_some_and(|transfer| {
            transfer.status != OnchainReplenishmentTransferStatus::SourceCompleted
                && transfer.provider_transfer_id.as_deref().is_none_or(|id| id.trim().is_empty())
                && run.plan.legs.get(transfer.leg_index as usize).is_some_and(|leg| {
                    leg.direction == OnchainTransferDirection::WithdrawToChain
                        && matches!(shared_types::venue_family(&leg.venue), "bybit" | "kraken")
                })
        });
    let receipt_venue = transfer.as_ref()
        .and_then(|transfer| run.plan.legs.get(transfer.leg_index as usize))
        .map_or("交易所", |leg| if shared_types::venue_family(&leg.venue) == "kraken" { "Kraken" } else { "Bybit" });
    view! {
        <div class=format!("onchain-replenishment-run {tone}") role="status">
            <div class="onchain-replenishment-heading">
                <small>"补仓运行态"</small>
                <strong>{label}</strong>
                <span class="num" title=run_id_title>{run_id_label}</span>
            </div>
            <div class="onchain-replenishment-transfer">
                <span class="onchain-replenishment-progress">{progress}</span>
                <span class="onchain-replenishment-progress" title=transfer_title>{transfer_label}</span>
                {credited.map(|text| view! { <span class="onchain-replenishment-progress">{text}</span> })}
                {deposit_review.map(|text| view! { <span class="onchain-replenishment-progress">{text}</span> })}
                {withdrawal_fee.map(|(text, source)| view! { <span class="onchain-replenishment-progress" title=source>{text}</span> })}
                {network_fee.map(|(text, source)| view! { <span class="onchain-replenishment-progress" title=source>{text}</span> })}
                {move || wait_label(&wait_run, clock_ms.get()).map(|text| view! {
                    <span class="onchain-replenishment-progress num">{text}</span>
                })}
            </div>
            <span class="onchain-replenishment-evidence" title=evidence_title>{evidence}</span>
            <small class="onchain-replenishment-next" title=next_action_title>{next_action}</small>
            {problem.map(|problem| {
                let title = problem.clone();
                view! { <small class="onchain-replenishment-problem is-danger" title=title>{problem}</small> }
            })}
            {submit}
            {recheck}
            {missing_provider_receipt.then(|| view! {
                <small class="onchain-replenishment-next">{format!("缺少 {receipt_venue} 提币回执编号，请先在交易所核对提币记录；不能自动重新查询或重发。")}</small>
            })}
            {run.read_only_recovery.then(|| view! { <small class="onchain-replenishment-next">"只读恢复 · 不重发转账，不自动执行下一步"</small> })}
        </div>
    }
}

fn wait_label(run: &OnchainReplenishmentRun, now_ms: i64) -> Option<String> {
    if run.read_only_recovery && matches!(run.status,
        OnchainReplenishmentRunStatus::AwaitingSourceFinality
        | OnchainReplenishmentRunStatus::AwaitingDestinationCredit
        | OnchainReplenishmentRunStatus::Paused) {
        return Some(format!("只读核验 {}/{} 轮", run.recovery_checks, shared_types::ONCHAIN_REPLENISHMENT_RECOVERY_LIMIT));
    }
    let deadline = run.automatic_wait_deadline_ms()?;
    let started = run.transfers.last()?.submission_attempted_at_ms;
    let elapsed = now_ms.saturating_sub(started).max(0) / 60_000;
    let remaining = deadline.saturating_sub(now_ms).max(0);
    let remaining_minutes = remaining.saturating_add(59_999) / 60_000;
    Some(if remaining > 0 {
        format!("已等待 {elapsed} 分钟 · 自动核验剩余 {remaining_minutes} 分钟")
    } else {
        format!("已等待 {elapsed} 分钟 · 自动核验窗口已到")
    })
}

fn run_progress(run: &OnchainReplenishmentRun) -> String {
    let completed = run
        .transfers
        .iter()
        .take_while(|transfer| {
            transfer.status == OnchainReplenishmentTransferStatus::DestinationCredited
                && transfer.withdrawal_unlocked != Some(false)
        })
        .count()
        .min(run.plan.legs.len());
    let mut text = format!("补仓 {completed}/{} 已完成", run.plan.legs.len());
    if let Some(cost) = run
        .plan
        .transfer_cost_usd
        .filter(|cost| cost.is_finite() && *cost >= 0.0)
    {
        text.push_str(&format!(" · 预计总费 {}", usd(cost)));
    }
    if completed > 0 {
        let spent = run.plan.legs[..completed]
            .iter()
            .try_fold(0.0_f64, |total, leg| {
                leg.economics
                    .reconciled_cost_usd
                    .or(leg.economics.estimated_cost_usd)
                    .filter(|cost| cost.is_finite() && *cost >= 0.0)
                    .map(|cost| total + cost)
                    .filter(|cost| cost.is_finite())
            });
        match spent {
            Some(cost) => {
                let label = if run.plan.legs[..completed]
                    .iter()
                    .all(|leg| leg.economics.reconciled_cost_usd.is_some())
                {
                    "含前序费用"
                } else {
                    "含前序估算"
                };
                text.push_str(&format!(" · {label} {}", usd(cost)));
            }
            None => text.push_str(" · 前序费用待核"),
        }
    }
    text
}

fn submit_button(
    data: OnchainReplenishmentData,
    clock_ms: RwSignal<i64>,
    authorization_deadline: i64,
    run_id: String,
    direction: Option<OnchainTransferDirection>,
) -> impl IntoView {
    let submitting = data.submitting;
    let submit = data.submit;
    let disabled = move || {
        clock_ms.get() >= authorization_deadline
            || !data.loaded.get() || data.authorizing.get() || data.rechecking.get()
            || submitting.get()
            || data.recovery_problem.get().is_some()
    };
    let on_submit = move |_| submit.run(run_id.clone());
    let (title, idle_label) = match direction {
        Some(OnchainTransferDirection::DepositToCex) => (
            "提交前会重读报价、充值地址、链上余额与风险开关；交易哈希会先持久化，结果不确定时不会重复广播",
            "提交真实链上充值",
        ),
        _ => (
            "提交前会重读报价、充提证据、现货钱包余额与风险开关；结果不确定时不会自动重试",
            "提交真实提币",
        ),
    };
    view! {
        <button
            type="button"
            class="workbench-primary onchain-replenishment-submit"
            disabled=disabled
            title=title
            on:click=on_submit
        >
            {move || if submitting.get() { "提交中…" } else { idle_label }}
        </button>
    }
}

fn failure_row(label: &'static str, problem: String) -> AnyView {
    let title = problem.clone();
    view! {
        <div class="onchain-replenishment-plan is-danger" role="alert">
            <strong>{label}</strong>
            <span title=title>{problem}</span>
        </div>
    }
    .into_any()
}

const fn plan_state(status: OnchainReplenishmentPlanStatus) -> (&'static str, &'static str) {
    match status {
        OnchainReplenishmentPlanStatus::ReadyForAuthorization => ("待明确授权", "is-warning"),
        OnchainReplenishmentPlanStatus::Unprofitable => ("搬运后不盈利", "is-danger"),
        OnchainReplenishmentPlanStatus::EvidencePending => ("证据待核验", "is-warning"),
        OnchainReplenishmentPlanStatus::Blocked => ("补仓已阻断", "is-danger"),
    }
}

const fn run_state(
    status: OnchainReplenishmentRunStatus,
    direction: Option<OnchainTransferDirection>,
) -> (&'static str, &'static str) {
    match status {
        OnchainReplenishmentRunStatus::AuthorizedAwaitingSubmit => ("已授权，待提交", "is-warning"),
        OnchainReplenishmentRunStatus::AuthorizationExpired => ("授权已过期", "is-danger"),
        OnchainReplenishmentRunStatus::ReadyForNextTransfer => {
            ("上一条已到账，核验下一条", "is-warning")
        }
        OnchainReplenishmentRunStatus::Submitting => match direction {
            Some(OnchainTransferDirection::DepositToCex) => ("链上交易提交中", "is-warning"),
            _ => ("交易所确认中", "is-warning"),
        },
        OnchainReplenishmentRunStatus::AwaitingSourceFinality => match direction {
            Some(OnchainTransferDirection::DepositToCex) => ("链上确认中", "is-warning"),
            _ => ("交易所提币处理中", "is-warning"),
        },
        OnchainReplenishmentRunStatus::AwaitingDestinationCredit => match direction {
            Some(OnchainTransferDirection::DepositToCex) => ("等待交易所入账", "is-warning"),
            _ => ("等待链上到账", "is-warning"),
        },
        OnchainReplenishmentRunStatus::Completed => ("补仓已到账", "is-positive"),
        OnchainReplenishmentRunStatus::Paused => ("已安全暂停", "is-danger"),
        OnchainReplenishmentRunStatus::Failed => ("补仓失败", "is-danger"),
    }
}

const fn transfer_state(
    status: OnchainReplenishmentTransferStatus,
    direction: Option<OnchainTransferDirection>,
) -> &'static str {
    match status {
        OnchainReplenishmentTransferStatus::SubmissionClaimed => "提交占位已落盘",
        OnchainReplenishmentTransferStatus::Submitted => match direction {
            Some(OnchainTransferDirection::DepositToCex) => "链上交易已广播",
            _ => "交易所已受理",
        },
        OnchainReplenishmentTransferStatus::SourceCompleted => match direction {
            Some(OnchainTransferDirection::DepositToCex) => "链上交易已确认",
            _ => "交易所已完成",
        },
        OnchainReplenishmentTransferStatus::DestinationCredited => "目标已到账",
        OnchainReplenishmentTransferStatus::Paused => "已暂停",
        OnchainReplenishmentTransferStatus::Failed => "已失败",
    }
}

fn route_label(leg: &OnchainReplenishmentLeg) -> String {
    let network = if leg.venue.eq_ignore_ascii_case("kraken") {
        leg.chain.as_str()
    } else {
        leg.network_evidence.network.as_deref().unwrap_or(&leg.chain)
    };
    match leg.direction {
        OnchainTransferDirection::WithdrawToChain => {
            format!(
                "{} → 链 · {} · {network}",
                leg.venue.to_uppercase(),
                leg.asset
            )
        }
        OnchainTransferDirection::DepositToCex => {
            format!(
                "链 → {} · {} · {network}",
                leg.venue.to_uppercase(),
                leg.asset
            )
        }
    }
}

fn amount_label(leg: &OnchainReplenishmentLeg) -> String {
    let amount = leg
        .transfer_amount_exact
        .clone()
        .unwrap_or_else(|| format!("{:.8}", leg.transfer_amount));
    format!("{amount} {}", leg.asset)
}

fn fee_label(leg: &OnchainReplenishmentLeg) -> String {
    let fee = leg
        .economics
        .fee_amount_exact
        .clone()
        .or_else(|| leg.economics.fee_amount.map(|value| format!("{value:.8}")));
    let debit = leg
        .economics
        .source_debit_upper_bound_exact
        .clone()
        .or_else(|| {
            leg.economics
                .source_debit_upper_bound
                .map(|value| format!("{value:.8}"))
        });
    match (fee, debit) {
        (Some(fee), Some(debit)) => format!("费 {fee} · 最多扣 {debit}"),
        _ => "费用待核验".to_owned(),
    }
}

fn destination_label(leg: &OnchainReplenishmentLeg) -> String {
    let state = match leg.destination.status {
        OnchainReplenishmentDestinationStatus::Verified => "已核验",
        OnchainReplenishmentDestinationStatus::ConfiguredUnverified => "未核验",
        OnchainReplenishmentDestinationStatus::Missing => "未配置",
    };
    leg.destination.address.as_deref().map_or_else(
        || state.to_owned(),
        |address| format!("{state} · {}", compact_id(address)),
    )
}

fn compact_id(value: &str) -> String {
    if value.chars().count() <= 16 {
        return value.to_owned();
    }
    let head = value.chars().take(8).collect::<String>();
    let tail = value
        .chars()
        .rev()
        .take(6)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!("{head}..{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replenishment_render_displays_exact_credit_without_resubmit_button() {
        Owner::new().with(|| {
            let run: OnchainReplenishmentRun = serde_json::from_str(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"), "/../shared-types/fixtures/onchain_replenishment_locked.json"
            ))).unwrap();
            let data = OnchainReplenishmentData {
                loaded: RwSignal::new(true),
                rechecking: RwSignal::new(false), recheck: Callback::new(|_| {}),
                runs: RwSignal::new(Vec::new()),
                plan: RwSignal::new(None), building: RwSignal::new(false), build: Callback::new(|_| {}),
                confirmation: RwSignal::new(String::new()), authorizing: RwSignal::new(false), authorize: Callback::new(|_| {}),
                run: RwSignal::new(None), submitting: RwSignal::new(false), submit: Callback::new(|_| {}),
                recovery_problem: RwSignal::new(None),
            };
            let html = run_status(run.clone(), data, RwSignal::new(60)).to_html();
            assert!(html.contains("已入账 · 待解锁"));
            assert!(html.contains("实际到账 12.5 USDC"));
            assert!(html.contains("可交易，提币待解锁"));
            assert!(!html.contains("<button"));
            assert!(!html.contains("等待交易所入账"));
            assert!(html.contains("自动核验剩余 120 分钟"));
            let mut timed_out_locked = run.clone();
            timed_out_locked.status = OnchainReplenishmentRunStatus::Paused;
            timed_out_locked.read_only_recovery = true;
            timed_out_locked.recovery_checks = 12;
            let locked_html = run_status(timed_out_locked.clone(), data, RwSignal::new(8_000_000)).to_html();
            assert!(locked_html.contains("只读核验 12/12 轮"));
            assert!(locked_html.contains("实际到账 12.5 USDC"));
            assert!(locked_html.contains("重新核验原转账"));
            assert!(!locked_html.contains("自动核验剩余"));
            assert!(!locked_html.contains("onchain-replenishment-submit"));
            timed_out_locked.transfers[0].withdrawal_unlocked = Some(true);
            assert!(timed_out_locked.recheck_request().is_none());
            assert!(html.contains("网络费待核验"));
            let mut charged = run.clone();
            charged.transfers[0].network_cost = Some(shared_types::OnchainReplenishmentNetworkCost {
                chain: "solana".into(), transaction_id: "signature".into(), block_ref: "120".into(), payer: "wallet".into(),
                asset: "SOL".into(), execution_fee_exact: Some("0.000005".into()), additional_fee_exact: Some("0".into()),
                total_fee_exact: Some("0.000005".into()), source: "Solana getTransaction".into(), observed_at_ms: 60, problem: None,
                usd_valuation: Some(shared_types::OnchainReplenishmentCostValuation {
                    usd_amount_exact: "0.000505".into(), valued_at_ms: 60,
                    quote: Some(shared_types::OnchainUsdValuation {asset:"SOL".into(),venue:"kraken".into(),symbol:"SOL/USD".into(),
                        source:"ws_push".into(),usd_bid:99.0,usd_ask:101.0,observed_at_ms:60}),
                }),
            });
            let charged_html = run_status(charged.clone(), data, RwSignal::new(60)).to_html();
            assert!(charged_html.contains("实扣网络费 0.000005 SOL"));
            assert!(charged_html.contains("折算 $0.000505"));
            assert!(charged_html.contains("kraken/SOL/USD ask 101"));
            assert!(!charged_html.contains("网络费待核验"));
            charged.transfers[0].network_cost.as_mut().unwrap().usd_valuation = None;
            let unvalued_html = run_status(charged.clone(), data, RwSignal::new(60)).to_html();
            assert!(unvalued_html.contains("美元折算待核"));
            charged.transfers[0].network_cost.as_mut().unwrap().total_fee_exact = None;
            let partial_html = run_status(charged, data, RwSignal::new(60)).to_html();
            assert!(partial_html.contains("已知执行费 0.000005 SOL · 网络总费待核验"));
            assert!(!partial_html.contains("实扣网络费"));
            if let Ok(path) = std::env::var("REPLENISHMENT_NETWORK_FEE_RENDER_PATH") {
                let css = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/styles/.generated/input.css")).unwrap();
                std::fs::write(path, format!("<!doctype html><html lang=zh-CN><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\"><title>网络费用核验</title><style>{css}</style><body>{charged_html}{unvalued_html}{partial_html}</body></html>")).unwrap();
            }
            if let Ok(path) = std::env::var("REPLENISHMENT_RENDER_PATH") {
                let css = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/styles/.generated/input.css")).unwrap();
                std::fs::write(path, format!("<!doctype html><html lang=zh-CN><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\"><title>充值到账核验</title><style>{css}</style><body>{html}</body></html>")).unwrap();
            }
            let mut fee_review = run.clone();
            fee_review.status = OnchainReplenishmentRunStatus::Paused;
            fee_review.transfers[0].credited_amount_exact = None;
            fee_review.transfers[0].withdrawal_unlocked = None;
            fee_review.transfers[0].reported_deposit_amount_exact = Some("12.5".to_owned());
            fee_review.transfers[0].deposit_fee_exact = Some("0.1".to_owned());
            fee_review.transfers[0].status = OnchainReplenishmentTransferStatus::Paused;
            fee_review.next_action = "核实净到账后重新规划，不重复转账".to_owned();
            let review_html = run_status(fee_review.clone(), data, RwSignal::new(60)).to_html();
            assert!(review_html.contains("平台记录 12.5 USDC"));
            assert!(review_html.contains("充值费 0.1 USDC"));
            assert!(review_html.contains("净到账待核实"));
            assert!(!review_html.contains("实际到账"));
            assert!(review_html.contains("重新核验原转账"));
            assert!(!review_html.contains("onchain-replenishment-submit"));
            let mut bybit = fee_review;
            bybit.plan.legs[0].venue = "bybit".into();
            bybit.plan.legs[0].direction = OnchainTransferDirection::WithdrawToChain;
            bybit.transfers[0].provider_transfer_id = None;
            let missing_ack = run_status(bybit.clone(), data, RwSignal::new(60)).to_html();
            assert!(missing_ack.contains("缺少 Bybit 提币回执编号"));
            assert!(!missing_ack.contains("重新核验原转账"));
            assert!(!missing_ack.contains("onchain-replenishment-submit"));
            let mut kraken = bybit.clone();
            kraken.plan.legs[0].venue = "kraken".into();
            let missing_kraken = run_status(kraken, data, RwSignal::new(60)).to_html();
            assert!(missing_kraken.contains("缺少 Kraken 提币回执编号"));
            assert!(!missing_kraken.contains("重新核验原转账"));
            bybit.transfers[0].provider_transfer_id = Some("known-receipt".into());
            let known_ack = run_status(bybit, data, RwSignal::new(60)).to_html();
            assert!(known_ack.contains("重新核验原转账"));
            assert!(!known_ack.contains("缺少 Bybit 提币回执编号"));
            let mut read_only = run.clone();
            read_only.read_only_recovery = true;
            read_only.status = OnchainReplenishmentRunStatus::AuthorizedAwaitingSubmit;
            let read_only_html = run_status(read_only, data, RwSignal::new(60)).to_html();
            assert!(read_only_html.contains("只读恢复"));
            assert!(!read_only_html.contains("onchain-replenishment-submit"));
            let mut continuing = run.clone();
            continuing.status = OnchainReplenishmentRunStatus::ReadyForNextTransfer;
            continuing.transfers[0].withdrawal_unlocked = Some(true);
            continuing.plan.legs[0].economics.estimated_cost_usd = Some(0.1);
            let mut next = continuing.plan.legs[0].clone();
            next.direction = OnchainTransferDirection::WithdrawToChain;
            next.venue = "bitget".into();
            next.asset = "SOL".into();
            next.economics.estimated_cost_usd = Some(0.2);
            continuing.plan.legs.push(next);
            continuing.plan.transfer_cost_usd = Some(0.3);
            continuing.next_action = "上一条已到账；仅重新核验剩余补仓，不重复转账".into();
            let continuing_html = run_status(continuing.clone(), data, RwSignal::new(60)).to_html();
            assert!(continuing_html.contains("补仓 1/2 已完成"));
            assert!(continuing_html.contains("预计总费 $0.30"));
            assert!(continuing_html.contains("含前序估算 $0.10"));
            assert!(!continuing_html.contains("<button"));
            continuing.status = OnchainReplenishmentRunStatus::Paused;
            let mut paused_transfer = continuing.transfers[0].clone();
            paused_transfer.leg_index = 1;
            paused_transfer.status = OnchainReplenishmentTransferStatus::Paused;
            paused_transfer.provider_transfer_id = Some("second-transfer".into());
            paused_transfer.credited_amount_exact = None;
            paused_transfer.withdrawal_unlocked = None;
            paused_transfer.evidence_source = Some("Solana RPC".into());
            paused_transfer.withdrawal_cost = Some(shared_types::OnchainReplenishmentWithdrawalCost {
                asset: "SOL".into(), reported_amount_exact: "1".into(), fee_exact: "0.001".into(), confirmed: true,
                source: "Bitget withdrawal records".into(), observed_at_ms: 60, usd_valuation: None,
            });
            continuing.transfers.push(paused_transfer);
            continuing.next_action = "已保留交易哈希；先核对原交易、资产形态与实际到账，不要重复转账".into();
            continuing.problem = Some("交易已确认，WSOL 代币账户增加 1000000 原始单位，但原生 SOL 未到账；需按 WSOL 核对或解包，不能当作原生 SOL 继续执行".into());
            let paused_html = run_status(continuing.clone(), data, RwSignal::new(60)).to_html();
            assert!(paused_html.contains("不要重复转账"));
            assert!(paused_html.contains("不能当作原生 SOL 继续执行"));
            assert!(paused_html.contains("实扣提币费 0.001 SOL"));
            assert!(paused_html.contains("美元折算待核"));
            let mut waiting = continuing.clone();
            waiting.status = OnchainReplenishmentRunStatus::ReadyForNextTransfer;
            waiting.transfers[0].withdrawal_cost = waiting.transfers[1].withdrawal_cost.clone();
            waiting.transfers[0].withdrawal_cost.as_mut().unwrap().asset = "USDC".into();
            waiting.plan.legs[0].direction = OnchainTransferDirection::WithdrawToChain;
            waiting.transfers.truncate(1);
            let waiting_html = run_status(waiting, data, RwSignal::new(60)).to_html();
            assert!(waiting_html.contains("等待费用核算"));
            assert!(waiting_html.contains("暂不执行下一步"));
            assert!(!waiting_html.contains("<button"));
            continuing.transfers[1].withdrawal_cost.as_mut().unwrap().usd_valuation = Some(shared_types::OnchainReplenishmentCostValuation {
                usd_amount_exact: "0.101".into(), valued_at_ms: 60,
                quote: Some(shared_types::OnchainUsdValuation { asset: "SOL".into(), venue: "kraken".into(), symbol: "SOL/USD".into(), source: "ws_push".into(), usd_bid: 100.0, usd_ask: 101.0, observed_at_ms: 60 }),
            });
            let valued = run_status(continuing.clone(), data, RwSignal::new(60)).to_html();
            assert!(valued.contains("实扣提币费 0.001 SOL · 折算 $0.101"));
            assert!(valued.contains("kraken/SOL/USD ask 101"));
            continuing.transfers[1].withdrawal_cost.as_mut().unwrap().usd_valuation = None;
            continuing.transfers[1].withdrawal_cost.as_mut().unwrap().confirmed = false;
            let provisional = run_status(continuing.clone(), data, RwSignal::new(60)).to_html();
            assert!(provisional.contains("提币费暂报，尚未完成 0.001 SOL"));
            assert!(!provisional.contains("实扣提币费"));
            let cost = continuing.transfers[1].withdrawal_cost.as_mut().unwrap();
            cost.confirmed = true;
            cost.fee_exact = "0".into();
            assert!(run_status(continuing, data, RwSignal::new(60)).to_html().contains("实扣提币费 0 SOL"));
            if let Ok(path) = std::env::var("REPLENISHMENT_CONTINUATION_RENDER_PATH") {
                let css = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/styles/.generated/input.css")).unwrap();
                std::fs::write(path, format!("<!doctype html><html lang=zh-CN><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\"><title>补仓进度核验</title><style>{css}</style><body>{continuing_html}{paused_html}</body></html>")).unwrap();
            }
            let problem = "补仓恢复日志第 2 行未完整写入，资金动作已停用；请保留原日志并核对备份后重启";
            data.recovery_problem.set(Some(problem.to_owned()));
            let notice = recovery_notice(data.recovery_problem.get()).to_html();
            assert!(notice.contains("role=\"alert\""));
            assert!(notice.contains(problem));
            let mut authorized = run;
            authorized.status = OnchainReplenishmentRunStatus::AuthorizedAwaitingSubmit;
            authorized.transfers.clear();
            authorized.next_action = "在授权过期前重新核验计划并提交一次资金动作".to_owned();
            let blocked = run_status(authorized, data, RwSignal::new(60)).to_html();
            assert!(blocked.contains("提交真实链上充值"));
            assert!(blocked.contains("disabled"));
            if let Ok(path) = std::env::var("REPLENISHMENT_RECOVERY_RENDER_PATH") {
                let css = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/styles/.generated/input.css")).unwrap();
                std::fs::write(path, format!("<!doctype html><html lang=zh-CN><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\"><title>补仓恢复故障</title><style>{css}</style><body>{notice}{blocked}</body></html>")).unwrap();
            }
        });
    }

    #[test]
    fn replenishment_status_copy_distinguishes_source_and_destination_finality() {
        assert_eq!(
            run_state(
                OnchainReplenishmentRunStatus::AwaitingSourceFinality,
                Some(OnchainTransferDirection::WithdrawToChain),
            )
            .0,
            "交易所提币处理中"
        );
        assert_eq!(
            run_state(
                OnchainReplenishmentRunStatus::AwaitingDestinationCredit,
                Some(OnchainTransferDirection::WithdrawToChain),
            )
            .0,
            "等待链上到账"
        );
        assert_eq!(
            run_state(
                OnchainReplenishmentRunStatus::AwaitingDestinationCredit,
                Some(OnchainTransferDirection::DepositToCex),
            )
            .0,
            "等待交易所入账"
        );
    }

    #[test]
    fn kraken_replenishment_route_shows_chain_instead_of_funding_method_id() {
        let mut run: OnchainReplenishmentRun = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"), "/../shared-types/fixtures/onchain_replenishment_locked.json"
        ))).unwrap();
        let leg = &mut run.plan.legs[0];
        leg.venue = "kraken".into();
        leg.network_evidence.network = Some("3e7f8072-cc6d-4394-982a-5f4ca6ab27dd".into());
        assert_eq!(route_label(leg), "链 → KRAKEN · USDC · solana");
    }
}
