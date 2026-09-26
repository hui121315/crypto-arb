use super::*;
use crate::api::rest::{with_mutation_timeout, ApiError};
use crate::panels::shared::operation_journal::{validate_setting_response, OperationJournal};
use shared_types::{ActionRunKind, ActionRunStatus};

pub(super) struct MonitorData {
    pub pending: RwSignal<bool>,
    pub apply: Callback<(String, bool)>,
    pub recheck: Callback<()>,
}

pub(super) fn use_monitor(
    journal: OperationJournal, market: RwSignal<LoadState<StockMarketSnapshot>>,
    selecting: RwSignal<bool>, quoting: RwSignal<bool>, budget: RwSignal<String>,
    keyed: RwSignal<bool>, alerts: AlertData, notice: Notice,
) -> MonitorData {
    let reading = RwSignal::new(false);
    let pending = RwSignal::new(false);
    Effect::new(move |_| { journal.connection.track(); reading.set(false); });
    Effect::new(move |_| pending.set(journal.locked() || reading.get()));
    let refresh = Callback::new(move |_| spawn_local(read_current(journal, market, reading)));
    let recheck = journal.recheck(Callback::new(move |run: shared_types::ActionRun| {
        if run.status == ActionRunStatus::Succeeded {
            notice.inform("原监控操作已核对；当前运行状态以后台最新数据为准");
        } else {
            notice.set(Some(format!("上次监控修改未成功：{}", run.problem.map_or(run.message, |p|p.message))));
        }
        refresh.run(());
    }));
    let apply = Callback::new(move |(asset, enabled): (String, bool)| {
        if pending.get_untracked() || journal.locked() || selecting.get_untracked()
            || (enabled && quoting.get_untracked()) { return; }
        let Some(saved) = market.with_untracked(|s|s.value()
            .filter(|s|s.security.as_ref().is_some_and(|s|s.asset == asset))
            .map(|s|s.monitor.clone())) else { return; };
        if saved.revision.is_empty() {
            notice.set(Some("后台未提供单股监控版本，请更新后台后重读状态".into()));
            refresh.run(());
            return;
        }
        let request = if enabled {
            let config = match alerts.config(true) {
                Ok(config) => config,
                Err(error) => { notice.set(Some(error)); return; }
            };
            StockMonitorRequest { enabled, alerts: config,
                quote: StockQuoteRequest { asset: asset.clone(), budget_usdc: budget.get_untracked(), keyed: keyed.get_untracked() } }
        } else {
            // Stopping uses the applied configuration; malformed drafts must never block it.
            StockMonitorRequest { enabled: false, alerts: saved.alerts.clone(),
                quote: saved.request.clone().unwrap_or(StockQuoteRequest { asset: asset.clone(), budget_usdc: "0".into(), keyed: false }) }
        };
        let Some(attempt) = journal.begin(ActionRunKind::StockMonitorUpdate, asset) else { return; };
        let epoch = journal.epoch.get_untracked();
        let client = journal.client();
        notice.set(None);
        spawn_local(async move {
            let update = StockMonitorUpdateRequest { expected_revision: saved.revision, request: request.clone() };
            let result = client.monitor_stock(&update, &attempt.context).await.and_then(|receipt| {
                validate_setting_response(&attempt, &receipt)?;
                let expected_quote = if request.enabled { Some(&request.quote) } else { saved.request.as_ref() };
                if receipt.enabled != request.enabled || receipt.request.as_ref() != expected_quote
                    || receipt.alerts != request.alerts {
                    return Err(ApiError::client("STOCK_MONITOR_RECEIPT_MISMATCH", "监控处理结果参数不匹配，需核对原操作"));
                }
                Ok(receipt)
            });
            if !journal.current(epoch) { return; }
            match result {
                Ok(_) => {
                    journal.resolve(&attempt);
                    notice.inform("单股监控设置已保存");
                }
                Err(error) => {
                    notice.try_set(Some(error.problem.message.clone()));
                    journal.failed(&attempt, &error);
                }
            }
            journal.busy.set(false);
            read_current(journal, market, reading).await;
        });
    });
    MonitorData { pending, apply, recheck }
}

async fn read_current(journal: OperationJournal, market: RwSignal<LoadState<StockMarketSnapshot>>, reading: RwSignal<bool>) {
    if reading.try_get_untracked() != Some(false) { return; }
    reading.set(true);
    let epoch = journal.epoch.get_untracked();
    let client = journal.client();
    let result = with_mutation_timeout("读取单股监控当前状态", client.stock_market_snapshot()).await;
    if !journal.current(epoch) { return; }
    match result {
        Ok(snapshot) => super::apply_snapshot(market, snapshot),
        Err(error) => { market.try_update(|s|s.apply_result(Err(error.problem))); }
    }
    reading.try_set(false);
}
