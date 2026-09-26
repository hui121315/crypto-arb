use std::collections::BTreeMap;

use leptos::prelude::*;

use super::opportunity_view_model::OpportunityListViewModel;

const BLOCKER_ORDER: &[&str] = &[
    "行情数据不全",
    "执行规格未通过",
    "标的身份未通过",
    "指数成分未通过",
    "报价资产未对齐",
    "资金费数据不全",
    "价差收敛数据依据不足",
    "结算窗口未对齐",
    "退出和持有规则未补齐",
    "成本数据依据不完整",
    "等待构建时深度",
    "账户或权限未就绪",
    "存在执行阻断",
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum OpportunityEligibilityFilter {
    #[default]
    All,
    Eligible,
    Blocked(&'static str),
}

impl OpportunityEligibilityFilter {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::All => "全部",
            Self::Eligible => "可进入交易检查",
            Self::Blocked(label) => label,
        }
    }

    pub(crate) fn matches(self, row: &OpportunityListViewModel) -> bool {
        match self {
            Self::All => true,
            Self::Eligible => row.execution_eligible,
            Self::Blocked(label) => row.execution_blocker_summary() == Some(label),
        }
    }

    pub(crate) const fn is_all(self) -> bool {
        matches!(self, Self::All)
    }

    pub(crate) fn matches_with_readiness(self, row: &OpportunityListViewModel, ready: bool) -> bool {
        if ready {
            self.matches(row)
        } else {
            matches!(self, Self::All | Self::Blocked("行情数据不全"))
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OpportunityBlockerCount {
    label: &'static str,
    count: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct OpportunityEligibilitySummary {
    total: usize,
    eligible: usize,
    blockers: Vec<OpportunityBlockerCount>,
}

impl OpportunityEligibilitySummary {
    #[cfg(test)]
    pub(crate) fn from_rows<'a>(
        rows: impl IntoIterator<Item = &'a OpportunityListViewModel>,
    ) -> Self {
        Self::from_rows_with_readiness(rows.into_iter().map(|row| (row, true)))
    }

    pub(crate) fn from_rows_with_readiness<'a>(
        rows: impl IntoIterator<Item = (&'a OpportunityListViewModel, bool)>,
    ) -> Self {
        let mut summary = Self::default();
        let mut blockers = BTreeMap::<&'static str, usize>::new();
        for (row, ready) in rows {
            summary.total += 1;
            if ready && row.execution_eligible {
                summary.eligible += 1;
            } else {
                *blockers
                    .entry(if ready { row.execution_blocker_summary().unwrap_or("存在执行阻断") } else { "行情数据不全" })
                    .or_default() += 1;
            }
        }
        for &label in BLOCKER_ORDER {
            if let Some(count) = blockers.remove(label) {
                summary
                    .blockers
                    .push(OpportunityBlockerCount { label, count });
            }
        }
        summary.blockers.extend(
            blockers
                .into_iter()
                .map(|(label, count)| OpportunityBlockerCount { label, count }),
        );
        summary
    }

    fn count_for(&self, filter: OpportunityEligibilityFilter) -> usize {
        match filter {
            OpportunityEligibilityFilter::All => self.total,
            OpportunityEligibilityFilter::Eligible => self.eligible,
            OpportunityEligibilityFilter::Blocked(label) => self
                .blockers
                .iter()
                .find(|item| item.label == label)
                .map_or(0, |item| item.count),
        }
    }

    fn blocker_options(
        &self,
        selected: OpportunityEligibilityFilter,
    ) -> Vec<OpportunityBlockerCount> {
        let mut options = self.blockers.clone();
        if let OpportunityEligibilityFilter::Blocked(label) = selected {
            if !options.iter().any(|item| item.label == label) {
                options.push(OpportunityBlockerCount { label, count: 0 });
            }
        }
        options
    }
}

pub(crate) fn opportunity_eligibility_filter(
    summary: Memo<OpportunityEligibilitySummary>,
    selected: RwSignal<OpportunityEligibilityFilter>,
    page: Memo<Option<shared_types::OpportunityListPage>>,
) -> impl IntoView {
    view! {
        <Show when=move || page.with(Option::is_some) fallback=|| view! {
            <section class="opportunity-eligibility-strip" aria-label="本页可执行性筛选">
                <span>"候选数量待确认"</span>
            </section>
        }>
        <section class="opportunity-eligibility-strip" aria-label="本页可执行性筛选">
            <strong class="opportunity-eligibility-label">"本页可执行性"</strong>
            <div class="opportunity-eligibility-options" role="group" aria-label="按可执行性筛选">
                <EligibilityButton
                    label="全部"
                    count=Signal::derive(move || summary.get().total)
                    kind="all"
                    filter=OpportunityEligibilityFilter::All
                    selected
                    disabled=Signal::derive(|| false)
                />
                <EligibilityButton
                    label="可检查交易"
                    count=Signal::derive(move || summary.get().eligible)
                    kind="eligible"
                    filter=OpportunityEligibilityFilter::Eligible
                    selected
                    disabled=Signal::derive(move || {
                        summary.get().eligible == 0
                            && selected.get() != OpportunityEligibilityFilter::Eligible
                    })
                />
                {move || {
                    summary
                        .get()
                        .blocker_options(selected.get())
                        .into_iter()
                        .map(|item| {
                            let label = item.label;
                            let count = item.count;
                            view! {
                                <EligibilityButton
                                    label
                                    count=Signal::derive(move || count)
                                    kind="blocked"
                                    filter=OpportunityEligibilityFilter::Blocked(label)
                                    selected
                                    disabled=Signal::derive(|| false)
                                />
                            }
                        })
                        .collect_view()
                }}
            </div>
            <output class="opportunity-eligibility-count" aria-live="polite">
                {move || {
                    let summary = summary.get();
                    format!("显示 {} / {}", summary.count_for(selected.get()), summary.total)
                }}
            </output>
        </section>
        </Show>
    }
}

#[component]
fn EligibilityButton(
    label: &'static str,
    count: Signal<usize>,
    kind: &'static str,
    filter: OpportunityEligibilityFilter,
    selected: RwSignal<OpportunityEligibilityFilter>,
    disabled: Signal<bool>,
) -> impl IntoView {
    view! {
        <button
            type="button"
            class:is-active=move || selected.get() == filter
            data-kind=kind
            aria-pressed=move || (selected.get() == filter).to_string()
            disabled=move || disabled.get()
            on:click=move |_| selected.set(filter)
        >
            <span>{label}</span>
            <strong>{move || count.get()}</strong>
        </button>
    }
}
