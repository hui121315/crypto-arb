use super::*;

type FundingPaymentRequest = (String, LiveAdapter, Option<String>);

impl LiveVenueRouter {
    pub(in crate::trading_service::live_adapters) async fn get_configured_funding_payments(
        &self,
        kucoin_symbols: &[String],
        start_time_ms: Option<i64>,
        end_time_ms: Option<i64>,
    ) -> ExchangeResult<Vec<FundingPaymentData>> {
        let unique_kucoin_symbols = kucoin_symbols
            .iter()
            .filter_map(|symbol| {
                let symbol = symbol.trim().to_ascii_uppercase();
                (!symbol.is_empty()).then_some(symbol)
            })
            .collect::<BTreeSet<_>>();
        if unique_kucoin_symbols.len() > KUCOIN_FUNDING_SYMBOL_LIMIT {
            return Err(ExchangeError::Parse(format!(
                "kucoin funding payment candidate count {} exceeds bounded fanout limit {}",
                unique_kucoin_symbols.len(),
                KUCOIN_FUNDING_SYMBOL_LIMIT
            )));
        }
        let unique_kucoin_symbols = unique_kucoin_symbols.into_iter().collect::<Vec<_>>();
        let requests = self.funding_payment_requests(None, Some(&unique_kucoin_symbols));
        self.execute_funding_payment_requests(requests, start_time_ms, end_time_ms)
            .await
    }

    pub(super) async fn get_routed_funding_payments(
        &self,
        symbol: Option<&str>,
        start_time_ms: Option<i64>,
        end_time_ms: Option<i64>,
    ) -> ExchangeResult<Vec<FundingPaymentData>> {
        let requests = self.funding_payment_requests(symbol, None);
        self.execute_funding_payment_requests(requests, start_time_ms, end_time_ms)
            .await
    }

    fn funding_payment_requests(
        &self,
        symbol: Option<&str>,
        kucoin_symbols: Option<&[String]>,
    ) -> Vec<FundingPaymentRequest> {
        let mut requests = Vec::new();
        let mut seen_routes = BTreeSet::new();
        for (route, adapter) in self.routes.iter() {
            let key = balance_route_key(route);
            if !seen_routes.insert(key.to_owned()) {
                continue;
            }
            if key == "kucoin" {
                if let Some(symbols) = kucoin_symbols {
                    let mut seen_symbols = BTreeSet::new();
                    for symbol in symbols {
                        let normalized = symbol.trim().to_ascii_uppercase();
                        if !normalized.is_empty() && seen_symbols.insert(normalized.clone()) {
                            requests.push((route.clone(), Arc::clone(adapter), Some(normalized)));
                        }
                    }
                    continue;
                }
            }
            requests.push((
                route.clone(),
                Arc::clone(adapter),
                symbol.map(str::to_owned),
            ));
        }
        requests
    }

    async fn execute_funding_payment_requests(
        &self,
        requests: Vec<FundingPaymentRequest>,
        start_time_ms: Option<i64>,
        end_time_ms: Option<i64>,
    ) -> ExchangeResult<Vec<FundingPaymentData>> {
        let mut rows = Vec::new();
        let mut failures = Vec::new();
        let mut tasks = FuturesUnordered::new();
        for (route, adapter, symbol) in requests {
            let budget = Arc::clone(&self.private_read_budget);
            tasks.push(async move {
                let _permit = budget
                    .acquire_owned()
                    .await
                    .expect("private read budget remains open");
                let result = with_route_timeout(
                    FUNDING_PAYMENT_ROUTE_TIMEOUT,
                    adapter.get_funding_payments(symbol.as_deref(), start_time_ms, end_time_ms),
                )
                .await;
                (route, result)
            });
        }
        while let Some((route, result)) = tasks.next().await {
            collect_funding_payment_result(route, result, &mut rows, &mut failures);
        }
        self.failures.record("funding_payments", failures);
        sort_funding_payments(&mut rows);
        Ok(rows)
    }
}

fn collect_funding_payment_result(
    route: String,
    result: ExchangeResult<Vec<FundingPaymentData>>,
    rows: &mut Vec<FundingPaymentData>,
    failures: &mut Vec<RouteFailure>,
) {
    match result {
        Ok(payments) => rows.extend(payments),
        Err(error) if is_unsupported_funding_payment_route(&error) => {
            log_unsupported_funding_payment_route(&route, &error);
        }
        Err(error) => record_funding_payment_failure(route, error, failures),
    }
}

fn log_unsupported_funding_payment_route(route: &str, error: &ExchangeError) {
    tracing::debug!(route, %error, "live funding payment read unsupported");
}

fn record_funding_payment_failure(
    route: String,
    error: ExchangeError,
    failures: &mut Vec<RouteFailure>,
) {
    tracing::warn!(route = %route, %error, "live funding payment read failed");
    failures.push(RouteFailure::new(route, "funding_payments", error));
}

fn sort_funding_payments(rows: &mut [FundingPaymentData]) {
    rows.sort_by(|a, b| {
        a.venue
            .cmp(&b.venue)
            .then(a.symbol.cmp(&b.symbol))
            .then(a.funding_time_ms.cmp(&b.funding_time_ms))
            .then(a.venue_event_id.cmp(&b.venue_event_id))
    });
}

fn is_unsupported_funding_payment_route(error: &ExchangeError) -> bool {
    matches!(
        error,
        ExchangeError::NotImplemented(feature) | ExchangeError::UnsupportedCapability(feature)
            if *feature == "get_funding_payments"
    )
}
