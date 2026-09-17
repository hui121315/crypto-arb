use super::*;

impl LiveVenueRouter {
    pub(crate) async fn account_read_for_venues(
        &self,
        venues: &[String],
        currency: Option<&str>,
    ) -> ExchangeResult<VenueAccountRead> {
        let requested = venues
            .iter()
            .map(|venue| normalized_venue(venue))
            .map(|venue| balance_route_key(&venue).to_owned())
            .collect::<BTreeSet<_>>();
        self.collect_account_read(currency, Some(&requested), BALANCE_ROUTE_TIMEOUT)
            .await
    }

    pub(crate) async fn account_evidence_for_venues(
        &self,
        venues: &[String],
    ) -> ExchangeResult<VenueAccountRead> {
        let requested = venues
            .iter()
            .map(|venue| normalized_venue(venue))
            .map(|venue| balance_route_key(&venue).to_owned())
            .collect::<BTreeSet<_>>();
        self.collect_account_read(None, Some(&requested), ACCOUNT_EVIDENCE_ROUTE_TIMEOUT)
            .await
    }

    pub(super) async fn collect_account_read(
        &self,
        currency: Option<&str>,
        requested: Option<&BTreeSet<String>>,
        route_timeout: Duration,
    ) -> ExchangeResult<VenueAccountRead> {
        let mut rows = Vec::new();
        let mut summaries = Vec::new();
        let mut asset_valuations = Vec::new();
        let mut failures = Vec::new();
        let mut seen = BTreeSet::new();
        let mut tasks = FuturesUnordered::new();
        for (route, adapter) in self.routes.iter() {
            let key = balance_route_key(route);
            if requested.is_some_and(|venues| !venues.contains(key)) {
                continue;
            }
            if seen.insert(key.to_owned()) {
                let route = route.clone();
                let adapter = Arc::clone(adapter);
                let budget = Arc::clone(&self.private_read_budget);
                tasks.push(async move {
                    let _permit = budget
                        .acquire_owned()
                        .await
                        .expect("private read budget remains open");
                    let result = with_route_timeout(
                        account_route_timeout(&route, route_timeout),
                        adapter.get_account_read(currency),
                    )
                    .await;
                    (route, result)
                });
            }
        }
        while let Some((route, result)) = tasks.next().await {
            match result {
                Ok(read) => {
                    rows.extend(read.balances);
                    summaries.extend(read.summaries);
                    asset_valuations.extend(read.asset_valuations);
                    failures.extend(
                        read.issues
                            .into_iter()
                            .map(RouteFailure::from_account_read_issue),
                    );
                }
                Err(error) => {
                    tracing::warn!(route = %route, %error, "live balance read failed");
                    failures.push(RouteFailure::new(route, "balances", error));
                }
            }
        }
        self.failures.record("balances", failures);
        rows.sort_by(|a, b| a.venue.cmp(&b.venue).then(a.currency.cmp(&b.currency)));
        summaries.sort_by(|a, b| a.venue.cmp(&b.venue));
        asset_valuations.sort_by(|a, b| a.venue.cmp(&b.venue).then(a.currency.cmp(&b.currency)));
        Ok(VenueAccountRead {
            balances: rows,
            summaries,
            asset_valuations,
            issues: Vec::new(),
        })
    }
}
