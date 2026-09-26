use super::*;
use serde_json::{json, Value};
mod execution;
pub(super) mod store;

impl BackpackStocks {
    pub(crate) fn with_exchange_conversion_store(mut self, path: std::path::PathBuf) -> Self {
        self.exchange_conversion_store = store::Store::load(Some(path), self.wallet_claims.clone());
        self
    }
    pub(crate) async fn size_exchange_conversion(
        self: &Arc<Self>,
        r: StockExchangeConversionSizingRequest,
        hub: &realtime::WsHub,
    ) -> Result<StockExchangeConversionSizing, String> {
        r.minimum()?;
        let _lock = self
            .order_lock
            .try_lock()
            .map_err(|_| "股票账户正在处理其他请求")?;
        let generation = self.generation.load(Ordering::SeqCst);
        let asset = self
            .snapshot
            .read()
            .security
            .as_ref()
            .map(|s| s.asset.clone())
            .ok_or("请先选择股票以接入兑换行情")?;
        let keys = (self.credential_loader)()?;
        let (market, book, account) = self.read_conversion_inputs(&keys, hub).await?;
        self.ensure_generation(generation, &asset)?;
        if (self.credential_loader)()?.fingerprint() != keys.fingerprint() {
            return Err("账户凭证在试算期间变化，旧结果已丢弃".into());
        }
        let input_usdt = r.size(&market, &book, &account.spot_taker_fee_bps)?;
        let request = StockExchangeConversionRequest {
            request_id: "stock-conversion-read-only".into(),
            input_usdt: input_usdt.clone(),
            minimum_usdc: r.minimum_usdc.clone(),
        };
        let now = common::time::now_ms();
        let terms = compile(&request, market, book, &account, now)?;
        Ok(StockExchangeConversionSizing {
            request: r,
            input_usdt,
            minimum_net_usdc: terms.minimum_net_usdc,
            fee_budget_usdc: terms.fee_budget_usdc,
            available_usdt: terms.available_usdt,
            bid_usdc: terms.book.bid.ok_or("兑换买价缺失")?,
            step_size: terms.market.step_size,
            checked_at_ms: now,
            valid_until_ms: terms.valid_until_ms,
        })
    }
    pub(crate) async fn build_exchange_conversion(
        self: &Arc<Self>,
        r: StockExchangeConversionRequest,
        hub: &realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        let _lock = self
            .order_lock
            .try_lock()
            .map_err(|_| "股票账户正在处理其他请求")?;
        r.amounts()?;
        if self.snapshot.read().security.is_none() {
            return Err("请先选择股票以接入兑换行情".into());
        }
        let keys = (self.credential_loader)()?;
        let fp = keys.fingerprint();
        if self.exchange_conversion_store.previous(&r, &fp)?.is_some() {
            return Ok(self.snapshot());
        }
        let (market, book, account) = self.read_conversion_inputs(&keys, hub).await?;
        let now = common::time::now_ms();
        let terms = compile(&r, market, book, &account, now)?;
        if (self.credential_loader)()?.fingerprint() != fp {
            return Err("兑换账户在试算期间变化，未保存计划".into());
        }
        let plan = StockExchangeConversionPlan {
            plan_id: store::id(&r, &terms)?,
            request: r,
            terms,
            revision: 1,
            updated_at_ms: now,
            cancelled_at_ms: None,
            order: None,
        };
        let client = store::client_id(&plan)?;
        if self.plan_store.records().iter().any(|p|matches!(p.terms.cex_instruction,Some(StockCexInstruction::OrderBook{client_id,..}) if client_id==client)) {return Err("兑换订单编号与股票计划冲突，请重新生成请求".into());}
        self.exchange_conversion_store.insert(plan, now)?;
        self.publish_rfq(hub);
        Ok(self.snapshot())
    }
    async fn read_conversion_inputs(
        self: &Arc<Self>,
        keys: &credentials::Credentials,
        hub: &realtime::WsHub,
    ) -> Result<(StockConversionMarket, StockBookQuote, StockAccountEvidence), String> {
        self.ensure_started(hub.clone());
        let market = parse_market(
            &self.read("/api/v1/market?symbol=USDT_USDC").await?,
            common::time::now_ms(),
        )?;
        let account = self.read_account(keys).await?;
        let book = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Ok(book) = self.conversion_book(common::time::now_ms()) {
                    break book;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .map_err(|_| "USDT/USDC WS 盘口尚未就绪，未生成兑换计划")?;
        Ok((market, book, account))
    }
    fn conversion_book(&self, now: i64) -> Result<StockBookQuote, String> {
        let s = self.snapshot.read();
        if !s.connected || s.conversion_book_problem.is_some() {
            return Err("USDT/USDC WS 未就绪，请等待行情后重试".into());
        }
        s.conversion_book
            .as_ref()
            .filter(|b| {
                now >= b.received_at_ms
                    && now >= b.source_at_ms
                    && now - b.received_at_ms < 10_000
                    && now - b.source_at_ms < 10_000
            })
            .cloned()
            .ok_or("USDT/USDC 暂无新鲜 WS 买价，未用 1:1 换算".into())
    }
    pub(crate) fn cancel_exchange_conversion(
        &self,
        r: StockPlanRevisionRequest,
        hub: &realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        let now = common::time::now_ms();
        self.exchange_conversion_store
            .change(&r.plan_id, now, |p| {
                if p.cancelled_at_ms.is_some() {
                    return Ok(false);
                }
                if p.order.is_some() {
                    return Err("兑换已记录提交，不能取消或释放；请核对原订单".into());
                }
                if p.revision != r.revision {
                    return Err("兑换版本已变化，请刷新后取消".into());
                }
                p.cancelled_at_ms = Some(now);
                Ok(true)
            })?;
        self.publish_rfq(hub);
        Ok(self.snapshot())
    }
}
fn parse_market(bytes: &[u8], now: i64) -> Result<StockConversionMarket, String> {
    let v: Value = serde_json::from_slice(bytes).map_err(|_| "USDT/USDC 市场规格未完整解析")?;
    let text = |v: &Value| {
        v.as_str()
            .map(str::to_owned)
            .ok_or("USDT/USDC 官方规格字段缺失".to_owned())
    };
    Ok(StockConversionMarket {
        symbol: text(&v["symbol"])?,
        base_symbol: text(&v["baseSymbol"])?,
        quote_symbol: text(&v["quoteSymbol"])?,
        market_type: text(&v["marketType"])?,
        order_book_state: text(&v["orderBookState"])?,
        min_quantity: text(&v["filters"]["quantity"]["minQuantity"])?,
        max_quantity: if v["filters"]["quantity"]["maxQuantity"].is_null() {
            None
        } else {
            Some(text(&v["filters"]["quantity"]["maxQuantity"])?)
        },
        step_size: text(&v["filters"]["quantity"]["stepSize"])?,
        tick_size: text(&v["filters"]["price"]["tickSize"])?,
        checked_at_ms: now,
    })
}
fn compile(
    r: &StockExchangeConversionRequest,
    market: StockConversionMarket,
    book: StockBookQuote,
    account: &StockAccountEvidence,
    now: i64,
) -> Result<StockExchangeConversionTerms, String> {
    let (input, _) = r.amounts()?;
    let balance = account
        .balances
        .get("USDT")
        .ok_or("Backpack 账户没有可用 USDT，不能以抵押价值替代")?;
    if account.liquidating || order_protocol::decimal(&balance.available)? < input {
        return Err("账户正在强平或可用 USDT 不足；不会自动借币或赎回".into());
    }
    let (fee, net) = exchange_conversion_amounts(r, &market, &book, &account.spot_taker_fee_bps)?;
    let balance_at_ms = account.balances_at_ms.min(balance.observed_at_ms);
    let valid_until_ms = store::valid_until(&market, &book, balance_at_ms, account.fees_at_ms);
    if now >= valid_until_ms
        || now < balance_at_ms
        || now < account.fees_at_ms
        || now < market.checked_at_ms
        || now < book.source_at_ms
        || now < book.received_at_ms
    {
        return Err("账户兑换余额或报价过期，请重新读取".into());
    }
    let hash = common::signing::hmac_sha256_hex(
        b"stock-cex-conversion-client-v1",
        &serde_json::to_vec(&(r, &account.fingerprint)).map_err(|_| "兑换请求编码失败")?,
    );
    let client_id = u32::from_str_radix(&hash[..8], 16)
        .map_err(|_| "兑换编号生成失败")?
        .max(1);
    let instruction = StockCexInstruction::OrderBook {
        client_id,
        symbol: STOCK_CONVERSION_SYMBOL.into(),
        side: StockRfqSide::Ask,
        quantity: input.normalize().to_string(),
        limit_price: book.bid.clone().ok_or("兑换买价缺失")?,
    };
    Ok(StockExchangeConversionTerms {
        account_fingerprint: account.fingerprint.clone(),
        market,
        book,
        available_usdt: balance.available.clone(),
        balance_at_ms,
        taker_fee_bps: account.spot_taker_fee_bps.clone(),
        fees_at_ms: account.fees_at_ms,
        fee_budget_usdc: fee.normalize().to_string(),
        minimum_net_usdc: net.normalize().to_string(),
        instruction,
        created_at_ms: now,
        valid_until_ms,
    })
}

#[cfg(test)]
pub(super) mod tests;
