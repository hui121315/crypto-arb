use super::*;

impl LiveVenueRouter {
    pub(crate) async fn open_orders_for_venues(
        &self,
        venues: &[String],
        symbol: Option<&str>,
    ) -> ExchangeResult<Vec<OrderInfo>> {
        let requested = venues
            .iter()
            .map(|venue| normalized_venue(venue))
            .collect::<BTreeSet<_>>();
        let mut rows = Vec::new();
        let mut failures = Vec::new();
        let mut tasks = FuturesUnordered::new();
        for route in requested {
            let adapter = match self.route_for(&route) {
                Ok(adapter) => adapter,
                Err(error) => {
                    failures.push(RouteFailure::new(route, "open_orders", error));
                    continue;
                }
            };
            let budget = Arc::clone(&self.private_read_budget);
            tasks.push(async move {
                let _permit = budget
                    .acquire_owned()
                    .await
                    .expect("private read budget remains open");
                let result = with_route_timeout(
                    open_order_route_timeout(&route),
                    adapter.get_open_orders(symbol),
                )
                .await;
                (route, result)
            });
        }
        while let Some((route, result)) = tasks.next().await {
            match result {
                Ok(orders) => rows.extend(orders),
                Err(error) => {
                    tracing::warn!(route = %route, %error, "live open orders read failed");
                    failures.push(RouteFailure::new(route, "open_orders", error));
                }
            }
        }
        self.failures.record("open_orders", failures);
        rows.sort_by(|a, b| {
            a.exchange
                .cmp(&b.exchange)
                .then(a.symbol.cmp(&b.symbol))
                .then(a.order_id.cmp(&b.order_id))
        });
        Ok(rows)
    }

    pub(crate) async fn positions_for_venues(
        &self,
        venues: &[String],
        symbol: Option<&str>,
    ) -> ExchangeResult<Vec<PositionInfo>> {
        let requested = venues
            .iter()
            .map(|venue| normalized_venue(venue))
            .collect::<BTreeSet<_>>();
        let mut rows = Vec::new();
        let mut failures = Vec::new();
        let mut tasks = FuturesUnordered::new();
        for route in requested {
            let adapter = match self.route_for(&route) {
                Ok(adapter) => adapter,
                Err(error) => {
                    failures.push(RouteFailure::new(route, "positions", error));
                    continue;
                }
            };
            let budget = Arc::clone(&self.private_read_budget);
            tasks.push(async move {
                let _permit = budget
                    .acquire_owned()
                    .await
                    .expect("private read budget remains open");
                let result = with_route_timeout(
                    position_route_timeout(&route),
                    adapter.get_positions(symbol),
                )
                .await;
                (route, result)
            });
        }
        while let Some((route, result)) = tasks.next().await {
            match result {
                Ok(positions) => rows.extend(positions),
                Err(error) => {
                    tracing::warn!(route = %route, %error, "live position read failed");
                    failures.push(RouteFailure::new(route, "positions", error));
                }
            }
        }
        self.failures.record("positions", failures);
        rows.sort_by(|a, b| a.exchange.cmp(&b.exchange).then(a.symbol.cmp(&b.symbol)));
        Ok(rows)
    }
}
