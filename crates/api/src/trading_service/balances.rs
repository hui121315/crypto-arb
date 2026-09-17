use super::*;
mod cache;
mod valuations;

pub(super) const BALANCE_ROUTE_OPERATION: &str = "balances";

impl TradingService {
    /// PR-DP-08 D-8：per-venue balance cache 读路径。
    ///
    /// **读流程**：
    /// 1. 收集 dispatcher 声明会推 cache 的 venue（`dispatcher_venues_for`）
    /// 2. 对每个 venue 检查 cache fresh entry（按当前 epoch + `ttl_ms`）
    /// 3. 若全 fresh 直接返回 merged rows（fast path，**不 fetch**）
    /// 4. 若有 missing venue → fetch full（`balances_from_credentials`），
    ///    结果按 venue 分组写回 cache（仅 missing venue），未声明 venue
    ///    （如 `hyperliquid:spot` / `hyperliquid:xyz`）的 rows 直接合并返回
    /// 5. fetch 失败时对每个 missing venue 尝试 stale rows，全部 stale 命中
    ///    则降级返回；任一缺失则 propagate error
    ///
    /// 解决了原 single-entry `AccountCache<Vec<VenueBalanceInfo>>` 在 dispatcher
    /// 用 single-venue credentials 推 cache + 生产读路径用 full credentials 读
    /// 时 cache key 哈希不匹配永远不命中的 bug（参见 reproduction test
    /// `private_ws_events::tests::per_venue_ws_balances_visible_under_any_credentials_read`）。
    pub(crate) async fn list_configured_balances(
        &self,
        credentials: AdapterCredentials,
    ) -> Result<Vec<VenueBalanceInfo>, exchange::ExchangeError> {
        let mut dispatcher_venues = dispatcher_venues_for(&credentials);
        if credentials.hyperliquid_live.is_some() {
            self.extend_fresh_hyperliquid_cache_venues(&mut dispatcher_venues);
        }
        let fetch = ConfiguredBalanceFetch::Credentials(Box::new(credentials));

        // 没有任何 dispatcher venue（mock 模式或纯无 live credentials）→ 走 mock fetch。
        if dispatcher_venues.is_empty() {
            return self.fetch_configured_balances(fetch, &[]).await;
        }

        self.list_configured_balance_venues(&dispatcher_venues, fetch)
            .await
    }

    pub(crate) async fn list_configured_balances_low_latency(
        self: &Arc<Self>,
        credentials: AdapterCredentials,
    ) -> Result<Vec<VenueBalanceInfo>, exchange::ExchangeError> {
        let mut venues = dispatcher_venues_for(&credentials);
        if credentials.hyperliquid_live.is_some() {
            self.extend_fresh_hyperliquid_cache_venues(&mut venues);
        }
        if venues.is_empty() {
            return self.list_configured_balances(credentials).await;
        }

        let epoch = self.account_cache_epoch();
        let now_ms = common::time::now_ms();
        let mut cached = self.read_balance_cache(&venues, epoch, now_ms);
        if cached.is_complete() {
            return Ok(cached.merged);
        }
        let Some(stale) = self.stale_balance_rows(epoch, now_ms, &cached.missing) else {
            return self.list_configured_balances(credentials).await;
        };
        for rows in stale {
            cached.merged.extend(rows);
        }

        let refresh_available = cached
            .missing
            .iter()
            .all(|venue| self.balance_fetch_lock(venue).try_lock_owned().is_ok());
        if refresh_available {
            let service = Arc::clone(self);
            tokio::spawn(async move {
                if let Err(error) = service.list_configured_balances(credentials).await {
                    tracing::warn!(%error, "background balance cache refresh failed");
                }
            });
        }
        Ok(cached.merged)
    }

    #[cfg(test)]
    pub(super) async fn list_configured_balance_venues_from_adapter_for_test(
        &self,
        venues: &[String],
    ) -> Result<Vec<VenueBalanceInfo>, exchange::ExchangeError> {
        let dispatcher_venues = scoped_balance_venues(venues);
        let fetch = ConfiguredBalanceFetch::Adapter;
        if dispatcher_venues.is_empty() {
            return self.fetch_configured_balances(fetch, &[]).await;
        }
        self.list_configured_balance_venues(&dispatcher_venues, fetch)
            .await
    }

    pub(super) async fn list_configured_balance_venues(
        &self,
        dispatcher_venues: &[String],
        fetch: ConfiguredBalanceFetch,
    ) -> Result<Vec<VenueBalanceInfo>, exchange::ExchangeError> {
        let epoch = self.account_cache_epoch();
        let now_ms = common::time::now_ms();

        // Step 1：收集 fresh cache rows 与 missing venues。
        let mut cached = self.read_balance_cache(dispatcher_venues, epoch, now_ms);
        if cached.is_complete() {
            return Ok(cached.merged);
        }

        let _guards = self.lock_balance_fetch_venues(&cached.missing).await;
        let locked_now_ms = common::time::now_ms();
        cached = self.read_balance_cache(dispatcher_venues, epoch, locked_now_ms);
        if cached.is_complete() {
            return Ok(cached.merged);
        }

        if let Some(error) = self.active_balance_backoff_error(&cached.missing, locked_now_ms) {
            self.merge_stale_balances_or_error(
                epoch,
                locked_now_ms,
                &cached.missing,
                &mut cached.merged,
                error,
            )?;
            return Ok(cached.merged);
        }

        // Step 2：missing venue 触发 fetch full。
        if let Some(fetched) = self
            .fetch_configured_balances_or_stale(
                fetch,
                epoch,
                locked_now_ms,
                &cached.missing,
                &mut cached.merged,
            )
            .await?
        {
            let failed = self.record_balance_route_failure_backoffs(locked_now_ms);
            self.merge_failed_balance_stale(
                epoch,
                locked_now_ms,
                &cached.missing,
                &failed,
                &mut cached.merged,
            );
            // Step 3：fetch 结果按 venue 分组写回 cache（仅 missing 的 dispatcher venues）。
            self.seed_balance_cache_from_full_read(epoch, &cached.missing, &fetched, &failed);
            let succeeded = cached
                .missing
                .iter()
                .filter(|venue| !failed.contains(&normalized_venue_name(venue)))
                .cloned()
                .collect::<Vec<_>>();
            self.clear_balance_backoffs(&succeeded);
            // Step 4：合并 fresh + fetched（fresh venue 已在 merged 中，跳过 fetched 同 venue）。
            cached.merge_fetched(fetched);
        }
        Ok(cached.merged)
    }

    pub(super) fn record_balance_route_failure_backoffs(
        &self,
        observed_at_ms: i64,
    ) -> HashSet<String> {
        let route_errors = self.route_failures.exchange_errors(BALANCE_ROUTE_OPERATION);
        let failed = route_errors
            .iter()
            .map(|(venue, _)| normalized_venue_name(venue))
            .collect::<HashSet<_>>();
        for (venue, error) in route_errors {
            self.record_balance_fetch_error(&[venue], &error, observed_at_ms);
        }
        failed
    }

    pub(super) async fn fetch_configured_balances(
        &self,
        fetch: ConfiguredBalanceFetch,
        venues: &[String],
    ) -> Result<Vec<VenueBalanceInfo>, exchange::ExchangeError> {
        self.route_failures
            .record(BALANCE_ROUTE_OPERATION, Vec::new());
        let read = match fetch {
            ConfiguredBalanceFetch::Credentials(credentials) => {
                if self.account_reader.load().is_some() {
                    self.fetch_adapter_account_read_for_venues(venues).await?
                } else if dispatcher_venues_for(&credentials).is_empty() {
                    self.engine.adapter().get_account_read(None).await?
                } else {
                    live_adapters::account_read_from_credentials(
                        *credentials,
                        Arc::clone(&self.route_failures),
                    )
                    .await?
                }
            }
            #[cfg(test)]
            ConfiguredBalanceFetch::Adapter => self.fetch_adapter_account_read().await?,
        };
        self.record_account_summaries(read.summaries);
        self.record_asset_valuations(read.asset_valuations);
        Ok(read.balances)
    }

    pub(crate) async fn list_scoped_balances(
        &self,
        venues: &[String],
    ) -> Result<Vec<VenueBalanceInfo>, exchange::ExchangeError> {
        let venues = scoped_balance_venues(venues);
        self.list_scoped_balance_venues(&venues).await
    }

    pub(crate) async fn refresh_scoped_balances(
        &self,
        venues: &[String],
    ) -> Result<Vec<VenueBalanceInfo>, exchange::ExchangeError> {
        let venues = scoped_balance_venues(venues);
        self.balance_cache.remove_many(&venues);
        self.list_scoped_balance_venues(&venues).await
    }
}
