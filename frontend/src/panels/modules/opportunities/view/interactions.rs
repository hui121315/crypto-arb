use super::*;

type OpportunityRowCallback = Callback<(usize, OpportunityRow)>;

#[derive(Clone, Copy)]
pub(super) struct OpportunityCallbacks {
    pub(super) open: OpportunityRowCallback,
    pub(super) inspect: OpportunityRowCallback,
    pub(super) inspect_detail: OpportunityRowCallback,
}

pub(super) fn opportunity_callbacks(
    selected_idx: RwSignal<usize>,
    selected_opp_id: RwSignal<String>,
    selected_detail: RwSignal<OpportunityDetailSeed>,
    execution_runtime: ExecutionRuntime,
    active_module: RwSignal<ModuleId>,
) -> OpportunityCallbacks {
    OpportunityCallbacks {
        open: opportunity_open_callback(
            selected_idx,
            selected_opp_id,
            selected_detail,
            execution_runtime,
            active_module,
        ),
        inspect: opportunity_inspect_callback(selected_idx, selected_opp_id, selected_detail),
        inspect_detail: opportunity_inspect_detail_callback(
            selected_idx,
            selected_opp_id,
            selected_detail,
        ),
    }
}

fn opportunity_open_callback(
    selected_idx: RwSignal<usize>,
    selected_opp_id: RwSignal<String>,
    selected_detail: RwSignal<OpportunityDetailSeed>,
    execution_runtime: ExecutionRuntime,
    active_module: RwSignal<ModuleId>,
) -> OpportunityRowCallback {
    Callback::new(move |(idx, row): (usize, OpportunityRow)| {
        let detail_seed = detail_seed_from_row(&row);
        if detail_seed.id.is_empty() {
            return;
        }
        selected_idx.set(idx);
        selected_opp_id.set(detail_seed.id.clone());
        execution_runtime.seed_selection(ExecutionSelectionSeed::from_opportunities(row.as_ref()));
        selected_detail.set(detail_seed);
        active_module.set(ModuleId::Execution);
    })
}

fn opportunity_inspect_callback(
    selected_idx: RwSignal<usize>,
    selected_opp_id: RwSignal<String>,
    selected_detail: RwSignal<OpportunityDetailSeed>,
) -> OpportunityRowCallback {
    Callback::new(move |(idx, row): (usize, OpportunityRow)| {
        let detail_seed = detail_seed_from_row(&row);
        selected_idx.set(idx);
        selected_opp_id.set(detail_seed.id.clone());
        selected_detail.set(detail_seed);
    })
}

fn opportunity_inspect_detail_callback(
    selected_idx: RwSignal<usize>,
    selected_opp_id: RwSignal<String>,
    selected_detail: RwSignal<OpportunityDetailSeed>,
) -> OpportunityRowCallback {
    Callback::new(move |(idx, row): (usize, OpportunityRow)| {
        let detail_seed = detail_seed_from_row(&row);
        selected_idx.set(idx);
        selected_opp_id.set(detail_seed.id.clone());
        selected_detail.set(detail_seed);
        focus_opportunity_element("opportunity-detail-panel", true);
    })
}

pub(super) fn focus_opportunity_element(id: &str, align_to_top: bool) {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let Some(element) = document.get_element_by_id(id) else {
        return;
    };
    element.scroll_into_view_with_bool(align_to_top);
    if let Some(element) = element.dyn_ref::<web_sys::HtmlElement>() {
        let _ = element.focus();
    }
}

pub(super) fn bind_opportunity_selection(
    filtered_rows: Memo<Vec<OpportunityRow>>,
    selected_idx: RwSignal<usize>,
    selected_opp_id: RwSignal<String>,
    selected_detail: RwSignal<OpportunityDetailSeed>,
    execution_runtime: ExecutionRuntime,
) {
    Effect::new(move |_| {
        let list = filtered_rows.get();
        if list.is_empty() {
            if !selected_opp_id.get_untracked().is_empty() {
                selected_opp_id.set(String::new());
                selected_detail.set(OpportunityDetailSeed::empty());
                execution_runtime.clear_selection();
            }
            return;
        }
        let current = selected_opp_id.get_untracked();
        if !current.is_empty() {
            if let Some((idx, row)) = list.iter().enumerate().find(|(_, row)| row.id == current) {
                if selected_idx.get_untracked() != idx {
                    selected_idx.set(idx);
                }
                let seed = detail_seed_from_row(row);
                if selected_detail.with_untracked(|detail| *detail != seed) {
                    selected_detail.set(seed);
                }
                return;
            }
        }
        let idx = selected_idx.get_untracked().min(list.len() - 1);
        let detail_seed = detail_seed_at(idx, &list);
        if !detail_seed.id.is_empty() {
            selected_idx.set(idx);
            selected_opp_id.set(detail_seed.id.clone());
            selected_detail.set(detail_seed);
        }
    });
}
