use super::*;

/// 扫描槽的派生视图：单个 Memo 一次算出四个绑定所需的值。
/// 此前 class/title/dot/label 四个闭包各自 `state.get()`——每帧 WS 事件
/// 深拷贝整个 `OpportunityStreamEvent` 四次；Memo 化后每帧只派生一次小结构，
/// 且值不变时（PartialEq）不触发任何 DOM 更新。
#[derive(Clone, PartialEq)]
struct ScanSlotVm {
    slot_class: &'static str,
    title: String,
    dot: &'static str,
    label: String,
}

pub(crate) fn scan_status_slot(
    state: RwSignal<LoadState<OpportunityStreamEvent>>,
    runtime_problems: Memo<Vec<RuntimeProblem>>,
) -> impl IntoView {
    let vm = Memo::new(move |_| {
        let runtime_problems = runtime_problems.get();
        state.with(|state| {
            let meta = state.value().map(OpportunityCountMeta::from_stream_event);
            let problem = state.problem();
            ScanSlotVm {
                slot_class: scan_slot_class(state, meta.as_ref(), problem, &runtime_problems),
                title: scan_status_title(state, meta.as_ref(), problem, &runtime_problems),
                dot: dot_class(scan_degraded(
                    state,
                    meta.as_ref(),
                    problem,
                    &runtime_problems,
                )),
                label: scan_status_label(state, meta.as_ref(), problem, &runtime_problems),
            }
        })
    });
    view! {
        <div
            class=move || vm.with(|vm| vm.slot_class)
            title=move || vm.with(|vm| vm.title.clone())
        >
            <span class=move || vm.with(|vm| vm.dot)></span>
            <span class="slot-label">"扫描"</span>
            <span class="num">{move || vm.with(|vm| vm.label.clone())}</span>
        </div>
    }
}

pub(super) fn scan_degraded<T>(
    state: &LoadState<T>,
    meta: Option<&OpportunityCountMeta>,
    problem: Option<&ApiProblem>,
    runtime_problems: &[RuntimeProblem],
) -> bool {
    problem.is_some()
        || scan_runtime_problem(runtime_problems).is_some()
        || matches!(state, LoadState::Error(_))
        || scan_fresh_snapshot_stale(meta)
        || meta.is_some_and(|meta| {
            matches!(
                meta.status,
                OpportunityEnvelopeStatus::Degraded
                    | OpportunityEnvelopeStatus::Error
                    | OpportunityEnvelopeStatus::Stale
            ) || meta.scan.market_data_problem_count > 0
        })
}

pub(super) fn scan_slot_class<T>(
    state: &LoadState<T>,
    meta: Option<&OpportunityCountMeta>,
    problem: Option<&ApiProblem>,
    runtime_problems: &[RuntimeProblem],
) -> &'static str {
    if scan_degraded(state, meta, problem, runtime_problems) {
        "slot degraded"
    } else {
        "slot"
    }
}

pub(super) fn scan_status_label<T>(
    state: &LoadState<T>,
    meta: Option<&OpportunityCountMeta>,
    problem: Option<&ApiProblem>,
    runtime_problems: &[RuntimeProblem],
) -> String {
    if scan_runtime_problem(runtime_problems).is_some() {
        return "异常".into();
    }
    if problem.is_some() && meta.is_none() {
        return "错误".into();
    }
    let Some(meta) = meta else {
        return match state {
            LoadState::Loading => "加载".into(),
            LoadState::Error(_) => "错误".into(),
            _ => "-".into(),
        };
    };
    match meta.status {
        OpportunityEnvelopeStatus::Fresh if scan_fresh_snapshot_stale(Some(meta)) => "过期".into(),
        OpportunityEnvelopeStatus::Fresh if meta.scan.market_data_problem_count > 0 => {
            "降级".into()
        }
        OpportunityEnvelopeStatus::Fresh => scan_freshness_label(meta),
        OpportunityEnvelopeStatus::Warming => "预热".into(),
        OpportunityEnvelopeStatus::Stale => "过期".into(),
        OpportunityEnvelopeStatus::Degraded => "降级".into(),
        OpportunityEnvelopeStatus::Error => "错误".into(),
    }
}

fn scan_freshness_label(meta: &OpportunityCountMeta) -> String {
    let Some(freshness_ms) = meta.freshness_ms else {
        return "Fresh".into();
    };
    let freshness_ms = freshness_ms.max(0);
    if freshness_ms < 1_000 {
        format!("{freshness_ms}ms")
    } else {
        format!("{:.1}s", freshness_ms as f64 / 1_000.0)
    }
}

pub(super) fn scan_status_title<T>(
    state: &LoadState<T>,
    meta: Option<&OpportunityCountMeta>,
    problem: Option<&ApiProblem>,
    runtime_problems: &[RuntimeProblem],
) -> String {
    let state_label = match state {
        LoadState::Loading => "行情快照加载中".to_owned(),
        LoadState::Error(problem) => api_problem_summary(problem),
        _ => String::new(),
    };
    let meta_label = meta
        .map(OpportunityCountMeta::freshness_label)
        .unwrap_or_default();
    let problem_label = problem
        .map(|problem| format!("实时流问题：{}", stream_problem_label(problem)))
        .unwrap_or_default();
    let runtime_problem_label = scan_runtime_problem(runtime_problems)
        .map(|problem| format!("后台任务异常：{}", problem_summary(problem)))
        .unwrap_or_default();
    title_parts([
        state_label,
        meta_label,
        problem_label,
        runtime_problem_label,
    ])
}

pub(super) fn scan_runtime_problem(problems: &[RuntimeProblem]) -> Option<&RuntimeProblem> {
    problems.iter().find(|problem| {
        problem.scope == BACKGROUND_TASK_SCOPE
            && (problem.operation == ARBITRAGE_SNAPSHOT_TASK
                || problem.message.contains(ARBITRAGE_SNAPSHOT_TASK))
    })
}

pub(super) fn scan_fresh_snapshot_stale(meta: Option<&OpportunityCountMeta>) -> bool {
    meta.filter(|meta| meta.status == OpportunityEnvelopeStatus::Fresh)
        .and_then(scan_snapshot_age_secs)
        .is_some_and(|age_secs| age_secs > FRESH_SCAN_STALE_AFTER_SECS)
}

pub(super) fn scan_snapshot_age_secs(meta: &OpportunityCountMeta) -> Option<u64> {
    if let Some(freshness_ms) = meta.freshness_ms {
        return Some((freshness_ms.max(0) / 1_000) as u64);
    }
    let cached_at = meta.cached_at?;
    let age_ms = (js_sys::Date::now() as i64 - cached_at.timestamp_millis()).max(0);
    Some((age_ms / 1_000) as u64)
}
