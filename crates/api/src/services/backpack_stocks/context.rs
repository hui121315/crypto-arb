use super::*;

impl BackpackStocks {
    pub(super) fn context_current(&self, now: i64) -> bool {
        self.context_error.read().is_none()
            && self
                .calendar
                .read()
                .as_ref()
                .is_some_and(|c| c.current(now))
            && self
                .snapshot
                .read()
                .token_metadata_at_ms
                .is_some_and(|t| now >= t && now - t < CATALOG_TTL_MS)
            && self
                .catalog
                .read()
                .as_ref()
                .is_some_and(|c| now >= c.observed_at_ms && now - c.observed_at_ms < CATALOG_TTL_MS)
    }

    pub(super) async fn refresh_context(&self, generation: u64) -> Result<(), String> {
        let _guard = self.context_lock.lock().await;
        let asset = self
            .snapshot
            .read()
            .security
            .as_ref()
            .map(|s| s.asset.clone())
            .ok_or("尚未选择股票")?;
        self.ensure_generation(generation, &asset)?;
        let now = common::time::now_ms();
        if self.context_current(now) {
            self.update_route(now);
            return Ok(());
        }
        let security = self
            .catalog()
            .await?
            .rows
            .into_iter()
            .find(|s| s.asset == asset)
            .ok_or("证券已不在官方目录中")?;
        let needs_tokens = self
            .snapshot
            .read()
            .token_metadata_at_ms
            .is_none_or(|t| now < t || now - t >= CATALOG_TTL_MS);
        let tokens = if needs_tokens {
            Some(protocol::asset_context(
                &self.read("/api/v1/assets").await?,
                &asset,
            )?)
        } else {
            None
        };
        let calendar_current = self
            .calendar
            .read()
            .as_ref()
            .is_some_and(|c| c.current(now));
        if !calendar_current {
            let sessions = self.read("/api/v1/market-sessions").await?;
            let holidays = self.read("/api/v1/market-holidays").await?;
            *self.calendar.write() = Some(calendar::Calendar::parse(
                &sessions,
                &holidays,
                common::time::now_ms(),
            )?);
        }
        self.ensure_generation(generation, &asset)?;
        {
            let mut snapshot = self.snapshot.write();
            if self.generation.load(Ordering::SeqCst) != generation {
                return Err("股票选择已改变，元数据已丢弃".into());
            }
            if snapshot.security.as_ref() != Some(&security) {
                snapshot.comparison = None;
                snapshot.trading_route = None;
            }
            snapshot.security = Some(security);
            if let Some((tokens, funding)) = tokens {
                if snapshot.tokens != tokens {
                    snapshot.comparison = None;
                }
                snapshot.tokens = tokens;
                snapshot.funding_assets = funding;
                snapshot.token_metadata_at_ms = Some(common::time::now_ms());
                snapshot.token_metadata_problem = None;
            }
            snapshot.observed_at_ms =
                common::time::now_ms().max(snapshot.observed_at_ms.saturating_add(1));
        }
        *self.context_error.write() = None;
        self.update_route(common::time::now_ms());
        Ok(())
    }

    pub(super) fn update_route(&self, now: i64) -> bool {
        let current = self.snapshot();
        let Some(security) = current.security else {
            return false;
        };
        let calendar = self.calendar.read();
        let error = self.context_error.read().clone();
        if error.is_none()
            && current.trading_route.as_ref().is_some_and(|r| {
                r.valid_until_ms > now
                    && r.calendar_at_ms == calendar.as_ref().map(|c| c.fetched_at_ms)
            })
        {
            return false;
        }
        let route = match error {
            Some(error) => calendar::unknown(
                format!("交易上下文更新失败：{error}"),
                calendar.as_ref().map(|c| c.fetched_at_ms),
            ),
            None => calendar
                .as_ref()
                .map(|c| c.route(&security, now))
                .unwrap_or_else(|| calendar::unknown("正在读取官方交易日历", None)),
        };
        drop(calendar);
        let mut snapshot = self.snapshot.write();
        if snapshot.security.as_ref() == Some(&security)
            && snapshot.trading_route.as_ref() != Some(&route)
        {
            snapshot.trading_route = Some(route);
            snapshot.observed_at_ms = now.max(snapshot.observed_at_ms.saturating_add(1));
            return true;
        }
        false
    }
}
