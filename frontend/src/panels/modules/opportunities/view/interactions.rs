use super::*;

type OpportunityRowCallback = Callback<(usize, OpportunityRow)>;

#[derive(Clone, Copy)]
pub(super) struct OpportunityCallbacks {
    pub(super) open: OpportunityRowCallback,
    pub(super) inspect: OpportunityRowCallback,
    pub(super) inspect_detail: OpportunityRowCallback,
}

pub(super) fn opportunity_callbacks(
    visible_rows: Memo<Vec<OpportunityRow>>,
    quote_ready_ids: Memo<HashSet<String>>,
    active_meta: Memo<OpportunityCountMeta>,
    live_meta: Memo<OpportunityCountMeta>,
    live_ids: Memo<HashSet<String>>,
    selected_idx: RwSignal<usize>,
    selected_opp_id: RwSignal<String>,
    selected_detail: RwSignal<OpportunityDetailSeed>,
    requested_opp_id: RwSignal<Option<String>>,
    execution_runtime: ExecutionRuntime,
    active_module: RwSignal<ModuleId>,
) -> OpportunityCallbacks {
    let callbacks = OpportunityCallbacks {
        open: opportunity_open_callback(
            visible_rows,
            quote_ready_ids,
            active_meta,
            live_meta,
            live_ids,
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
    };
    OpportunityCallbacks {
        open: Callback::new(move |row| {
            requested_opp_id.set(None);
            callbacks.open.run(row);
        }),
        inspect: Callback::new(move |row| {
            requested_opp_id.set(None);
            callbacks.inspect.run(row);
        }),
        inspect_detail: Callback::new(move |row| {
            requested_opp_id.set(None);
            callbacks.inspect_detail.run(row);
        }),
    }
}

fn opportunity_open_callback(
    visible_rows: Memo<Vec<OpportunityRow>>,
    quote_ready_ids: Memo<HashSet<String>>,
    active_meta: Memo<OpportunityCountMeta>,
    live_meta: Memo<OpportunityCountMeta>,
    live_ids: Memo<HashSet<String>>,
    selected_idx: RwSignal<usize>,
    selected_opp_id: RwSignal<String>,
    selected_detail: RwSignal<OpportunityDetailSeed>,
    execution_runtime: ExecutionRuntime,
    active_module: RwSignal<ModuleId>,
) -> OpportunityRowCallback {
    Callback::new(move |(_, requested): (usize, OpportunityRow)| {
        if !quote_ready_ids.with_untracked(|ids| ids.contains(&requested.id))
            || active_meta
                .get_untracked()
                .aged_at(snapshot_clock())
                .preview_age_expired()
        {
            return;
        }
        if live_ids.with_untracked(|ids| ids.contains(&requested.id))
            && live_meta.get_untracked().aged_at(snapshot_clock()).preview_age_expired()
        {
            return;
        }
        let Some((idx, row)) = visible_rows.with_untracked(|rows| {
            rows.iter()
                .enumerate()
                .find(|(_, row)| row.id == requested.id && row.execution_eligible)
                .map(|(idx, row)| (idx, row.clone()))
        }) else {
            return;
        };
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
    allow_auto_select: Memo<bool>,
    selected_idx: RwSignal<usize>,
    selected_opp_id: RwSignal<String>,
    selected_detail: RwSignal<OpportunityDetailSeed>,
    requested_opp_id: RwSignal<Option<String>>,
) {
    Effect::new(move |_| {
        let list = filtered_rows.get();
        let current = selected_opp_id.get();
        let can_select = allow_auto_select.get();
        if let Some(requested) = requested_opp_id.get() {
            if selected_opp_id.get_untracked() != requested {
                selected_opp_id.set(requested.clone());
            }
            let seed = if let Some((idx, row)) =
                list.iter().enumerate().find(|(_, row)| row.id == requested)
            {
                if selected_idx.get_untracked() != idx {
                    selected_idx.set(idx);
                }
                detail_seed_from_row(row)
            } else {
                OpportunityDetailSeed::empty()
            };
            if selected_detail.with_untracked(|current| *current != seed) {
                selected_detail.set(seed);
            }
            return;
        }
        if list.is_empty() {
            if !selected_detail.with_untracked(|seed| seed.id.is_empty()) {
                selected_detail.set(OpportunityDetailSeed::empty());
            }
            return;
        }
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
            // A missing selection must not silently become a different trading route.
            if !selected_detail.with_untracked(|seed| seed.id.is_empty()) {
                selected_detail.set(OpportunityDetailSeed::empty());
            }
            return;
        }
        // A pending page/search still contains the previous scope's retained rows.
        if !can_select {
            if !selected_detail.with_untracked(|seed| seed.id.is_empty()) {
                selected_detail.set(OpportunityDetailSeed::empty());
            }
            return;
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
