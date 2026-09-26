use super::*;
use crate::api::rest::{ApiError, with_mutation_timeout};
use crate::state::read_scope::{ReadConnection, ReadScope};
use std::future::Future;

#[derive(Clone, Copy)]
pub(super) struct StockSource {
    scope: ReadScope,
    market: RwSignal<LoadState<StockMarketSnapshot>>,
}

impl StockSource {
    pub(super) fn new(
        market: RwSignal<LoadState<StockMarketSnapshot>>,
        reset: impl Fn() + 'static,
    ) -> Self {
        Self {
            scope: ReadScope::new(reset),
            market,
        }
    }

    pub(super) fn capture(self) -> ReadConnection {
        self.scope.capture()
    }

    pub(super) fn current(self, source: &ReadConnection) -> bool {
        self.scope.accepts(source)
    }

    pub(super) fn track(self) {
        self.scope.track();
    }

    fn matches_selection(self, snapshot: &StockMarketSnapshot) -> bool {
        self.market.try_with_untracked(|m| {
            m.value().is_none_or(|current| {
                current.security.as_ref().map(|s| &s.asset)
                    == snapshot.security.as_ref().map(|s| &s.asset)
            })
        }) == Some(true)
    }

    pub(super) async fn snapshot(
        self,
        source: &ReadConnection,
        request: impl Future<Output = Result<StockMarketSnapshot, ApiError>>,
    ) -> Option<Result<StockMarketSnapshot, ApiError>> {
        let mut result = request.await;
        if !self.current(source) {
            return None;
        }
        if result.as_ref().is_ok_and(|s| !self.matches_selection(s)) {
            // An old operation may finish after selecting another stock. Read current state,
            // never repeat the operation or restore its old selection from the response.
            let client = source.client();
            result =
                with_mutation_timeout("读取股票当前状态", client.stock_market_snapshot()).await;
            if !self.current(source) {
                return None;
            }
            if result.as_ref().is_ok_and(|s| !self.matches_selection(s)) {
                return Some(Err(ApiError::client(
                    "STOCK_SELECTION_CHANGED",
                    "股票选择已变化，保留当前页面；请从执行记录核对原操作",
                )));
            }
        }
        Some(result)
    }
}
