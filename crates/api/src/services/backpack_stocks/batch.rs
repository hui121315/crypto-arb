use super::*;
use crate::services::onchain_comparison::stock_quotes;
use futures::{stream, StreamExt};
use shared_types::stocks::comparison::SOLANA_USDC;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Weak,
};
mod mint_cache;

// Ok(Some) means partial quotes, not a provider-wide failure that slows every stock.
type RoundResult = Result<Option<String>, String>;

impl BackpackStocks {
    pub(crate) fn set_batch(
        self: &Arc<Self>,
        request: StockBatchRequest,
        expected_revision: &str,
        hub: realtime::WsHub,
    ) -> Result<StockMarketSnapshot, common::AppError> {
        use axum::http::StatusCode;
        let mut batch = self.batch.write();
        if expected_revision.is_empty() || batch.revision != expected_revision {
            return Err(common::AppError::domain(StatusCode::CONFLICT, "STOCK_BATCH_CHANGED",
                "后台批量参数已变化或服务已重启。本次未修改，请核对当前参数后再应用草稿。"));
        }
        validate_request(&request).map_err(|error|
            common::AppError::domain(StatusCode::BAD_REQUEST, "STOCK_BATCH_REJECTED", error))?;
        if batch.request.as_ref() == Some(&request) {
            let saved = batch.clone();
            return Ok({
                drop(batch);
                let mut snapshot = self.snapshot();
                snapshot.batch = saved;
                snapshot
            });
        }
        self.batch_generation.fetch_add(1, Ordering::SeqCst);
        batch.revision = uuid::Uuid::new_v4().to_string();
        let keep_prices = batch
            .request
            .as_ref()
            .is_some_and(|r| r.budget_usdc == request.budget_usdc && r.keyed == request.keyed);
        batch
            .rows
            .retain(|r| request.assets.contains(&r.security.asset));
        for row in &mut batch.rows {
            row.refreshing = false;
            if !keep_prices {
                row.buy = None;
                row.sell = None;
            }
        }
        batch.running = false;
        batch.round_started_at_ms = None;
        batch.last_round_elapsed_ms = None;
        batch.waiting_for_viewers = false;
        batch.problem = None;
        batch.next_at_ms = request.enabled.then(common::time::now_ms);
        batch.metadata_at_ms = None;
        batch.request = Some(request);
        let saved = batch.clone();
        drop(batch);
        self.ensure_started(hub.clone());
        let mut worker = self.batch_worker.lock();
        if worker.as_ref().is_none_or(|h| h.is_finished()) {
            *worker = Some(tokio::spawn(run(Arc::downgrade(self), hub.clone())));
        }
        drop(worker);
        self.publish_batch(&hub);
        let mut snapshot = self.snapshot();
        snapshot.batch = saved;
        Ok(snapshot)
    }

    fn publish_batch(&self, hub: &realtime::WsHub) {
        let mut snapshot = self.snapshot.write();
        snapshot.observed_at_ms =
            common::time::now_ms().max(snapshot.observed_at_ms.saturating_add(1));
        drop(snapshot);
        self.publish(hub);
    }

    pub(super) fn batch_streams(&self) -> BTreeSet<String> {
        let batch = self.batch.read();
        if !batch.request.as_ref().is_some_and(|r| r.enabled) {
            return BTreeSet::new();
        }
        batch
            .rows
            .iter()
            .flat_map(|r| {
                r.security
                    .order_books
                    .iter()
                    .map(|m| format!("bookTicker.{}", m.symbol))
            })
            .collect()
    }

    pub(super) fn batch_connection(&self, connected: bool) {
        for row in &mut self.batch.write().rows {
            row.connected = connected;
            if !connected {
                row.books.clear();
            }
        }
    }

    pub(super) fn batch_frame(&self, text: &str, now: i64) {
        let mut batch = self.batch.write();
        if !batch.request.as_ref().is_some_and(|r| r.enabled) {
            return;
        }
        let mut changed = false;
        for row in &mut batch.rows {
            let mut snapshot = StockMarketSnapshot {
                security: Some(row.security.clone()),
                books: row.books.clone(),
                ..Default::default()
            };
            let _ = protocol::apply(&mut snapshot, text, now);
            if row.books != snapshot.books {
                row.connected = true;
                changed = true;
            }
            row.books = snapshot.books;
        }
        drop(batch);
        if changed {
            let mut snapshot = self.snapshot.write();
            snapshot.observed_at_ms = now.max(snapshot.observed_at_ms.saturating_add(1));
        }
    }

    async fn batch_round(
        self: &Arc<Self>,
        generation: u64,
        request: StockBatchRequest,
        hub: &realtime::WsHub,
    ) -> RoundResult {
        let now = common::time::now_ms();
        let context_needed = self
            .batch
            .read()
            .metadata_at_ms
            .is_none_or(|t| now < t || now - t >= CATALOG_TTL_MS);
        if context_needed {
            let catalog = self.catalog().await?;
            let assets = self.read("/api/v1/assets").await?;
            let mut rows = discover(&catalog, &assets, &request.assets)?;
            let mut state = self.batch.write();
            if self.batch_generation.load(Ordering::SeqCst) != generation {
                return Ok(None);
            }
            retain_quotes(&mut rows, &state.rows);
            state.rows = rows;
            state.metadata_at_ms = Some(common::time::now_ms());
        }
        let rows = self.batch.read().rows.clone();
        let targets = rows
            .iter()
            .filter_map(|r| {
                let t = r.token.as_ref()?;
                Some((t.contract_address.clone()?, t.native_decimals?))
            })
            .collect::<Vec<_>>();
        if targets.is_empty() {
            return Err("所选证券尚无可读取的官方 Solana 合约".into());
        }
        let mints = mint_cache::MintCache::new(targets);
        let raw = comparison::budget_raw(&StockQuoteRequest {
            asset: String::new(),
            budget_usdc: request.budget_usdc.clone(),
            keyed: request.keyed,
        })?;
        // A single bounded queue uses the existing provider-wide quota, never one task/socket per stock.
        let total = rows.len();
        let mut jobs = stream::iter(rows)
            .map(|row| {
                let raw = raw.clone();
                let keyed = request.keyed;
                let mints = &mints;
                let source = &self.quote_source;
                async move { quote_row(row, keyed, &raw, mints, source).await }
            })
            .buffer_unordered(2);
        let mut received = 0;
        while let Some(mut row) = jobs.next().await {
            if row.buy.is_some() && row.sell.is_some() && row.problem.is_none() {
                let now = common::time::now_ms();
                if row.token_price(true, now).is_some() && row.token_price(false, now).is_some() {
                    received += 1;
                } else {
                    row.problem = Some("双向报价未同时有效，等待下一轮更新".into());
                }
            }
            if !self.accept_batch_row(generation, row) {
                return Ok(None);
            }
            self.publish_batch(hub);
        }
        if received == 0 {
            Err("本轮未取得完整双向链上报价，请查看各股票的具体原因".into())
        } else if received < total {
            Ok(Some(format!("本轮仅 {received}/{total} 只股票取得双向链上报价；其余请查看各行原因")))
        } else {
            Ok(None)
        }
    }

    fn accept_batch_row(&self, generation: u64, row: StockBatchRow) -> bool {
        let mut batch = self.batch.write();
        // set_batch changes both configuration and generation while holding this same lock.
        if self.batch_generation.load(Ordering::SeqCst) != generation {
            return false;
        }
        if let Some(current) = batch
            .rows
            .iter_mut()
            .find(|r| r.security.asset == row.security.asset)
        {
            let books = std::mem::take(&mut current.books);
            let connected = current.connected;
            *current = row;
            current.books = books;
            current.connected = connected;
        }
        true
    }
}

fn discover(
    catalog: &StockCatalog,
    bytes: &[u8],
    selected: &[String],
) -> Result<Vec<StockBatchRow>, String> {
    #[derive(serde::Deserialize)]
    struct Asset {
        symbol: String,
        tokens: Vec<StockChainToken>,
    }
    let entries: Vec<Asset> = serde_json::from_slice(bytes).map_err(|_| "官方合约目录响应无效")?;
    let mut tokens = BTreeMap::new();
    for a in entries.into_iter().filter(|a| selected.contains(&a.symbol)) {
        if tokens.insert(a.symbol, a.tokens).is_some() {
            return Err("官方资产目录含重复标识".into());
        }
    }
    selected
        .iter()
        .map(|asset| {
            let security = catalog
                .rows
                .iter()
                .find(|s| &s.asset == asset)
                .ok_or("股票不在官方证券目录")?
                .clone();
            let candidates = tokens
                .remove(asset)
                .unwrap_or_default()
                .into_iter()
                .filter(|t| {
                    t.blockchain == "Solana"
                        && t.contract_address.as_ref().is_some_and(|a| {
                            bs58::decode(a).into_vec().is_ok_and(|v| v.len() == 32)
                        })
                        && t.native_decimals.is_some_and(|n| n <= 12)
                })
                .collect::<Vec<_>>();
            let mut token = (candidates.len() == 1).then(|| candidates[0].clone());
            let profile = identity::backpack_issuer(&security).ok();
            let issuer_verified = profile.zip(token.as_ref()).is_some_and(|(p, t)| {
                t.contract_address.as_deref() == Some(p.solana_mint)
                    && t.native_decimals == Some(p.decimals)
            });
            let mut problem = token
                .is_none()
                .then(|| "官方未提供唯一有效的 Solana 合约和精度".into());
            if identity::backpack_issuer_profile(asset).is_some() && !issuer_verified {
                token = None;
                problem = Some("官方目录与既有发行方资料冲突，暂停询价".into());
            }
            Ok(StockBatchRow {
                security,
                token,
                issuer_verified,
                mint: None,
                buy: None,
                sell: None,
                books: vec![],
                connected: false,
                refreshing: true,
                problem,
                checked_at_ms: None,
            })
        })
        .collect()
}

fn validate_request(request: &StockBatchRequest) -> Result<(), String> {
    if !request.enabled {
        return Ok(());
    }
    comparison::budget_raw(&StockQuoteRequest {
        asset: String::new(),
        budget_usdc: request.budget_usdc.clone(),
        keyed: request.keyed,
    })?;
    if request.assets.is_empty()
        || request.assets.len() > STOCK_BATCH_LIMIT
        || request.assets.iter().collect::<BTreeSet<_>>().len() != request.assets.len()
        || request.assets.iter().any(|s| {
            s.is_empty()
                || s.len() > 40
                || !s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'.')
        })
        || !(5..=300).contains(&request.interval_secs)
    {
        return Err("请选择 1–32 只不重复股票，轮询间隔为 5–300 秒".into());
    }
    Ok(())
}

async fn quote_row(
    mut row: StockBatchRow,
    keyed: bool,
    raw: &str,
    mints: &mint_cache::MintCache,
    source: &stock_quotes::Source,
) -> StockBatchRow {
    let Some(address) = row.token.as_ref().and_then(|t| t.contract_address.clone()) else {
        row.refreshing = false;
        return row;
    };
    row.buy = None;
    row.sell = None;
    row.mint = None;
    match mints.get(&address, source).await {
        Ok(mint) => row.mint = Some(mint),
        Err(error) => {
            row.refreshing = false;
            row.problem = Some(error);
            row.checked_at_ms = Some(common::time::now_ms());
            return row;
        }
    }
    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let buy = source.jupiter(keyed, SOLANA_USDC, &address, raw).await?;
        let sell =
            source.jupiter(keyed, &address, SOLANA_USDC, &buy.minimum_output_raw).await;
        Ok::<_, String>((buy, sell))
    })
    .await;
    row.refreshing = false;
    row.checked_at_ms = Some(common::time::now_ms());
    match result {
        Ok(Ok((buy, sell))) => {
            row.buy = Some(buy);
            match sell {
                Ok(q) => {
                    row.sell = Some(q);
                    row.problem = None;
                }
                Err(e) => {
                    row.sell = None;
                    row.problem = Some(e);
                }
            }
        }
        Ok(Err(e)) => {
            row.buy = None;
            row.sell = None;
            row.problem = Some(e);
        }
        Err(_) => {
            row.buy = None;
            row.sell = None;
            row.problem = Some("询价超时，下一轮重试".into());
        }
    }
    row
}

async fn run(service: Weak<BackpackStocks>, hub: realtime::WsHub) {
    let mut tick = tokio::time::interval(Duration::from_millis(500));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut job =
        None::<std::pin::Pin<Box<dyn std::future::Future<Output = RoundResult> + Send>>>;
    let mut generation = 0;
    let mut failures = 0_u32;
    let mut round_started = None::<tokio::time::Instant>;
    loop {
        tokio::select! {
            _=tick.tick()=>{},
            result=async{job.as_mut().unwrap().await},if job.is_some()=>{
                job=None;
                let Some(s)=service.upgrade()else{break};
                let mut b=s.batch.write();
                if s.batch_generation.load(Ordering::SeqCst)!=generation{continue;}
                let failed=result.is_err();
                b.running=false;b.problem=match result {Ok(problem)=>problem,Err(error)=>Some(error)};
                b.last_round_elapsed_ms=round_started.take().map(|at|u64::try_from(at.elapsed().as_millis()).unwrap_or(u64::MAX));
                b.round_started_at_ms=None;
                failures=if failed{failures.saturating_add(1)}else{0};
                for row in &mut b.rows {row.refreshing=false;}
                b.completed_rounds=b.completed_rounds.saturating_add(1);
                let interval=b.request.as_ref().map_or(15,|r|r.interval_secs);
                b.next_at_ms=Some(common::time::now_ms()+retry_delay_ms(interval,failures));
                drop(b);s.publish_batch(&hub);
            }
        }
        let Some(s) = service.upgrade() else { break };
        let current = s.batch_generation.load(Ordering::SeqCst);
        if current != generation {
            job = None;
            round_started = None;
            generation = current;
            failures = 0;
        }
        let viewers = hub.subscriber_count(realtime::channels::STOCKS) > 0;
        let mut b = s.batch.write();
        let enabled = b.request.as_ref().is_some_and(|r| r.enabled);
        if !enabled || !viewers {
            job = None;
            round_started = None;
            let changed = b.running || b.waiting_for_viewers != (enabled && !viewers);
            b.running = false;
            b.round_started_at_ms = None;
            b.waiting_for_viewers = enabled && !viewers;
            for row in &mut b.rows {
                row.refreshing = false;
            }
            drop(b);
            if changed {
                s.publish_batch(&hub);
            }
            continue;
        }
        let resumed = b.waiting_for_viewers;
        b.waiting_for_viewers = false;
        if job.is_some() || b.next_at_ms.is_some_and(|at| at > common::time::now_ms()) {
            drop(b);
            if resumed {
                s.publish_batch(&hub);
            }
            continue;
        }
        let request = b.request.clone().unwrap();
        b.running = true;
        b.round_started_at_ms = Some(common::time::now_ms());
        round_started = Some(tokio::time::Instant::now());
        b.problem = None;
        for row in &mut b.rows {
            row.refreshing = true;
        }
        drop(b);
        s.publish_batch(&hub);
        let h = hub.clone();
        job = Some(Box::pin(async move {
            s.batch_round(generation, request, &h).await
        }));
    }
}

fn retry_delay_ms(interval_secs: u32, failures: u32) -> i64 {
    let multiplier = 1_i64 << failures.saturating_sub(1).min(4);
    (i64::from(interval_secs.clamp(5, 300)) * 1000 * multiplier).min(300_000)
}

#[cfg(test)]
mod tests;

fn retain_quotes(rows: &mut [StockBatchRow], previous: &[StockBatchRow]) {
    for row in rows {
        if let Some(old) = previous
            .iter()
            .find(|old| old.security.asset == row.security.asset)
        {
            if row.security.order_books == old.security.order_books {
                row.books = old.books.clone();
                row.connected = old.connected;
            }
            if row.token == old.token
                && row.issuer_verified == old.issuer_verified
                && row.problem.is_none()
            {
                row.buy = old.buy.clone();
                row.sell = old.sell.clone();
                row.mint = old.mint.clone();
                row.checked_at_ms = old.checked_at_ms;
            }
        }
    }
}
