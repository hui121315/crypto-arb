use crate::panels::modules::timestamp::local_hms;
use leptos::prelude::*;
use shared_types::RuntimeProblem;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProblemCategory {
    Blocker,
    MarketData,
    Configuration,
    Transport,
    PendingEvidence,
    Other,
}

impl ProblemCategory {
    const ALL: [Self; 6] = [
        Self::Blocker,
        Self::MarketData,
        Self::Configuration,
        Self::Transport,
        Self::PendingEvidence,
        Self::Other,
    ];

    const fn index(self) -> usize {
        self as usize
    }

    const fn key(self) -> &'static str {
        match self {
            Self::Blocker => "blocker",
            Self::MarketData => "market-data",
            Self::Configuration => "configuration",
            Self::Transport => "transport",
            Self::PendingEvidence => "pending",
            Self::Other => "other",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Blocker => "操作阻断",
            Self::MarketData => "行情数据",
            Self::Configuration => "配置与权限",
            Self::Transport => "传输性能",
            Self::PendingEvidence => "待确认状态",
            Self::Other => "其它问题",
        }
    }

    const fn short_label(self) -> &'static str {
        match self {
            Self::Blocker => "阻断",
            Self::MarketData => "行情",
            Self::Configuration => "配置",
            Self::Transport => "传输",
            Self::PendingEvidence => "待确认",
            Self::Other => "其它",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ProblemBreakdown {
    counts: [usize; ProblemCategory::ALL.len()],
}

impl ProblemBreakdown {
    pub(super) fn summary_label(&self) -> String {
        ProblemCategory::ALL
            .into_iter()
            .filter_map(|category| {
                let count = self.count(category);
                (count > 0).then(|| format!("{} {count}", category.short_label()))
            })
            .collect::<Vec<_>>()
            .join(" · ")
    }

    fn count(&self, category: ProblemCategory) -> usize {
        self.counts[category.index()]
    }
}

pub(super) fn problem_breakdown(problems: &[RuntimeProblem]) -> ProblemBreakdown {
    let mut counts = [0; ProblemCategory::ALL.len()];
    for problem in problems {
        counts[problem_category(problem).index()] += 1;
    }
    ProblemBreakdown { counts }
}

pub(super) fn problem_evidence_ledger(problems: Memo<Vec<RuntimeProblem>>) -> impl IntoView {
    let breakdown = Memo::new(move |_| problems.with(|rows| problem_breakdown(rows)));

    view! {
        <Show when=move || problems.with(|rows| !rows.is_empty())>
            <details class="status-problem-ledger">
                <summary data-testid="status-problem-ledger-summary">
                    <strong>"运行问题"</strong>
                    <span>{move || breakdown.with(ProblemBreakdown::summary_label)}</span>
                    <span class="status-problem-ledger-chevron" aria-hidden="true"></span>
                </summary>
                <div
                    class="status-problem-list"
                    aria-label="系统运行问题分组"
                    data-testid="status-problem-list"
                >
                    {move || {
                        let rows = problems.get();
                        ProblemCategory::ALL
                            .into_iter()
                            .filter_map(|category| {
                                let group = rows
                                    .iter()
                                    .filter(|problem| problem_category(problem) == category)
                                    .cloned()
                                    .collect::<Vec<_>>();
                                (!group.is_empty()).then(|| view! {
                                    <section class="status-problem-group" data-kind=category.key()>
                                        <header class="status-problem-group-header">
                                            <strong>{category.label()}</strong>
                                            <span>{format!("{} 条", group.len())}</span>
                                        </header>
                                        <div role="list" aria-label=category.label()>
                                            {group
                                                .into_iter()
                                                .map(|problem| view! { <ProblemEvidenceRow problem/> })
                                                .collect_view()}
                                        </div>
                                    </section>
                                })
                            })
                            .collect_view()
                    }}
                </div>
            </details>
        </Show>
    }
}

fn problem_category(problem: &RuntimeProblem) -> ProblemCategory {
    let code = problem.code.to_ascii_uppercase();
    let scope = problem.scope.to_ascii_lowercase();
    let message = problem.message.to_ascii_lowercase();

    if scope.contains("credential")
        || code.contains("CREDENTIAL")
        || ((code.contains("BLOCK") || code.contains("REJECTED"))
            && (message.contains("配置")
                || message.contains("凭证")
                || message.contains("credential")))
    {
        return ProblemCategory::Configuration;
    }
    if code.contains("UNKNOWN") || message.contains("尚无运行态样本") || message.contains("尚无运行状态样本") {
        return ProblemCategory::PendingEvidence;
    }
    if scope == "market_data" {
        return ProblemCategory::MarketData;
    }
    if code.contains("BLOCK")
        || code.contains("REJECTED")
        || matches!(
            scope.as_str(),
            "risk" | "portfolio" | "trading_api" | "execution"
        )
    {
        return ProblemCategory::Blocker;
    }
    if scope.contains("transport") || scope.contains("private_ws") || scope == "ws" {
        return ProblemCategory::Transport;
    }
    ProblemCategory::Other
}

#[component]
fn ProblemEvidenceRow(problem: RuntimeProblem) -> impl IntoView {
    let venue = problem.venue.as_deref().unwrap_or("系统").to_uppercase();
    let checked_at = local_hms(problem.observed_at_ms)
        .map(|time| format!("检查 {time}"))
        .unwrap_or_else(|| "检查时间未知".to_string());
    let retry = problem.retry_after_ms.map(retry_label);

    view! {
        <article class="status-problem-row" role="listitem">
            <div class="status-problem-identity">
                <strong>{venue}</strong>
                <span>{problem.operation}</span>
            </div>
            <p>{problem.message}</p>
            <div class="status-problem-meta">
                <code>{problem.code}</code>
                <span>{problem.scope}</span>
                <span>{checked_at}</span>
                {retry.map(|label| view! { <span>{label}</span> })}
            </div>
        </article>
    }
}

fn retry_label(retry_after_ms: u64) -> String {
    if retry_after_ms >= 1_000 && retry_after_ms.is_multiple_of(1_000) {
        format!("重试 {}s", retry_after_ms / 1_000)
    } else {
        format!("重试 {retry_after_ms}ms")
    }
}
