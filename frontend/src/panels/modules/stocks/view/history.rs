use super::*;

#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) struct Row {
    pub id: String,
    pub asset: String,
    pub route: String,
    pub amount: String,
    pub status: String,
    pub holds_funds: bool,
    pub priority: u8,
    pub created_at_ms: i64,
}

pub(super) fn picker(
    rows: Memo<Vec<Row>>,
    selected: RwSignal<Option<String>>,
    label: &'static str,
) -> (Memo<Option<String>>, impl IntoView) {
    let search = RwSignal::new(String::new());
    let sorted = Memo::new(move |_| {
        let mut rows = rows.get();
        rows.sort_by(|a, b| b.priority.cmp(&a.priority)
            .then_with(|| b.created_at_ms.cmp(&a.created_at_ms)).then_with(|| a.id.cmp(&b.id)));
        rows
    });
    let visible = Memo::new(move |_| {
        let query = search.get().trim().to_lowercase();
        sorted.with(|rows| rows.iter().filter(|r| query.is_empty()
            || format!("{} {} {} {}", r.asset, r.route, r.status, r.id).to_lowercase().contains(&query))
            .cloned().collect::<Vec<_>>())
    });
    Effect::new(move |_| {
        let current = selected.get();
        sorted.with(|rows| {
            if current.is_some() && !rows.iter().any(|r| Some(&r.id) == current.as_ref()) {
                selected.set(None);
            }
        });
    });
    // A successful build can select its exact record, including while a search is active.
    Effect::new(move |_| {
        let current = selected.get();
        if current.is_some() && !visible.with_untracked(|rows| rows.iter().any(|r| Some(&r.id) == current.as_ref())) {
            search.set(String::new());
        }
    });
    let active = Memo::new(move |_| {
        let current = selected.get();
        visible.with(|rows| rows.iter().find(|r| Some(&r.id) == current.as_ref()).map(|r| r.id.clone()))
    });
    let view = view! {<div class="stock-record-browser" hidden=move ||rows.with(Vec::is_empty)>
        <div class="stock-record-toolbar">
            <input type="search" aria-label=format!("搜索{label}") placeholder="股票、状态或计划编号"
                prop:value=move ||search.get() on:input=move |ev|search.set(event_target_value(&ev))/>
            <span>{move ||format!("{} / {} 笔 · {} 笔占用资金",visible.with(Vec::len),rows.with(Vec::len),rows.with(|r|r.iter().filter(|r|r.holds_funds).count()))}</span>
        </div>
        <div class="stock-record-scroll">
            <table class="stock-record-table" aria-label=label>
                <thead><tr><th>"股票 / 记录"</th><th>"路径"</th><th>"数量"</th><th>"状态"</th><th><span class="sr-only">"操作"</span></th></tr></thead>
                <tbody><For each=move ||visible.get() key=|r|r.id.clone() children=move |initial| {
                    let id=initial.id.clone();
                    let current_id=id.clone();let select_id=id.clone();let action_id=id.clone();
                    let text_id=id.clone();
                    let row_id=id.clone();
                    let row=Memo::new(move |_|rows.with(|rows|rows.iter().find(|r|r.id==row_id).cloned()).unwrap_or_else(||initial.clone()));
                    view!{<tr aria-selected=move ||(active.get().as_ref()==Some(&current_id)).to_string()>
                        <td><strong>{move ||row.with(|r|r.asset.clone())}</strong><small title=id.clone()>{id.clone()}</small></td>
                        <td data-label="路径">{move ||row.with(|r|r.route.clone())}</td><td data-label="数量" class="stock-record-amount">{move ||row.with(|r|r.amount.clone())}</td>
                        <td data-label="状态"><span data-priority=move ||row.with(|r|r.priority) class="stock-record-state">{move ||row.with(|r|r.status.clone())}</span></td>
                        <td><button type="button" class="row-action" id=format!("stock-record-toggle-{id}")
                            aria-label={let id=id.clone();move ||format!("{} {}",if active.get().as_ref()==Some(&id){"收起"}else{"查看"},id)}
                            aria-expanded=move ||(active.get().as_ref()==Some(&select_id)).to_string()
                            on:click=move |_|{if active.get_untracked().as_ref()==Some(&action_id){selected.set(None);}else{selected.set(Some(action_id.clone()));}}>
                            {move ||if active.get().as_ref()==Some(&text_id){"收起"}else{"查看"}}</button></td>
                    </tr>}
                }/></tbody>
            </table>
            {move ||visible.with(Vec::is_empty).then(||view!{<p class="stock-empty-inline" role="status">"没有匹配的记录"</p>})}
        </div>
    </div>};
    (active, view)
}

pub(super) fn close(selected: RwSignal<Option<String>>) -> impl IntoView {
    view!{<button type="button" class="row-action" on:click=move |_| {
        #[cfg(target_arch="wasm32")]
        let id=selected.get_untracked();
        selected.set(None);
        #[cfg(target_arch="wasm32")]
        request_animation_frame(move || {
            use wasm_bindgen::JsCast;
            if let Some(button)=id.and_then(|id|web_sys::window().and_then(|w|w.document())
                .and_then(|d|d.get_element_by_id(&format!("stock-record-toggle-{id}"))))
                .and_then(|el|el.dyn_into::<web_sys::HtmlElement>().ok()) {
                let _=button.focus();
            }
        });
    }>"收起详情"</button>}
}

pub(super) fn execution(p: &StockExecutionPlan, now: i64) -> Row {
    let phase = p.phase_at(now);
    let (status, priority) = match phase {
        StockPlanPhase::Reserved => ("已预留 · 未下单", 1),
        StockPlanPhase::SubmissionUnknown => ("提交待核对 · 保留占用", 2),
        StockPlanPhase::Cancelled => ("已取消", 0),
        StockPlanPhase::Expired => ("预留已到期", 0),
        StockPlanPhase::Settled => ("已收尾 · 预留已释放", 0),
    };
    Row { id:p.plan_id.clone(), asset:p.request.asset.clone(),
        route:p.request.direction.label().into(), amount:format!("{} 股",p.terms.cex_shares),
        status:status.into(), priority, holds_funds:p.holds_funds(now), created_at_ms:p.terms.created_at_ms }
}

pub(super) fn peer(p: &StockPeerPlan, now: i64) -> Row {
    let (status, priority) = match p.phase {
        StockPeerPlanPhase::Reserved if p.holds_funds(now) => ("已预留 · 未下单", 1),
        StockPeerPlanPhase::Reserved => ("预留已到期", 0),
        StockPeerPlanPhase::SubmissionUnknown => ("提交待核对 · 保留占用", 2),
        StockPeerPlanPhase::Cancelled => ("已取消", 0),
        StockPeerPlanPhase::Settled => ("已结算 · 预留已释放", 0),
    };
    Row { id:p.plan_id.clone(), asset:p.request.asset.clone(),
        route:format!("{} · {}",p.request.selection.native_symbol,super::peer_plans::direction_name(p.request.direction)),
        amount:format!("{} 股",p.terms.draft.quantity), status:status.into(), priority,
        holds_funds:p.holds_funds(now), created_at_ms:p.terms.created_at_ms }
}

pub(super) fn funding(p: &StockFundingPlan, now: i64) -> Row {
    let phase=p.phase_at(now);
    let conflict=p.transfer.as_ref().is_some_and(|t|t.evidence_conflict.is_some())
        ||p.withdrawal.as_ref().is_some_and(|w|w.evidence_conflict.is_some());
    let pending=p.transfer.as_ref().and_then(|t|t.deposit_scan.as_ref()).is_some_and(|s|s.completed_at_ms.is_none());
    let status=if conflict {"处理结果冲突 · 保留占用"} else if pending {"入账历史核对中"} else {phase.label()};
    let holds_funds=phase.holds_funds();
    Row { id:p.plan_id.clone(), asset:p.request.security_asset.clone(),
        route:format!("{} → {}",p.terms.need.source,p.terms.need.target),
        amount:format!("{} {}",p.terms.quantity,p.request.funding_asset), status:status.into(),
        priority:if conflict ||pending ||holds_funds && phase!=StockFundingPlanPhase::Reserved {2} else if holds_funds {1} else {0},
        holds_funds, created_at_ms:p.terms.created_at_ms }
}
