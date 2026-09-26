use super::*;
use rust_decimal::Decimal;
use std::str::FromStr;

pub(super) struct Inputs {
    pub fingerprint: Option<String>,
    pub wallet: Option<StockWalletEvidence>,
    pub problems: Vec<String>,
}

impl BackpackStocks {
    pub(crate) async fn preflight(
        self: &Arc<Self>,
        request: StockPreflightRequest,
        hub: realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        let input_hub = hub.clone();
        self.preflight_with(request, hub, move |service, request, generation| async move {
            service.read_preflight_inputs(&request, generation, &input_hub).await
        }).await
    }

    pub(super) async fn preflight_with<I, F>(
        self: &Arc<Self>, mut request: StockPreflightRequest, hub: realtime::WsHub, read: I,
    ) -> Result<StockMarketSnapshot, String>
    where I: FnOnce(Arc<Self>, StockPreflightRequest, u64) -> F,
          F: std::future::Future<Output=Result<Inputs, String>>,
    {
        let _guard = self
            .preflight_lock
            .try_lock()
            .map_err(|_| "股票预检进行中，请等待结果")?;
        request.wallet_address = request
            .wallet_address
            .map(|w| w.trim().to_owned())
            .filter(|w| !w.is_empty());
        if let Some(wallet) = request.wallet_address.as_deref() {
            super::super::onchain_comparison::stock_inventory::validate_owner(wallet)?;
        }
        let generation = self.generation.load(Ordering::SeqCst);
        self.ensure_generation(generation, &request.asset)?;
        self.restock_source(&request)?;
        let inputs = read(self.clone(), request.clone(), generation).await?;
        self.ensure_generation(generation, &request.asset)?;
        let source = self.restock_source(&request)?;
        {
            let _rfq = self.rfq_state_lock.lock();
            let account = self.account.read();
            let mut snapshot = self.snapshot.write();
            if self.generation.load(Ordering::SeqCst) != generation {
                return Err("股票选择已变化，旧预检已丢弃".into());
            }
            snapshot.rfqs = self.visible_rfqs();
            snapshot.rfq_connected = inputs
                .fingerprint
                .as_deref()
                .is_some_and(|f| self.rfq_subscription.borrow().as_deref() == Some(f));
            snapshot.rfq_problem = self
                .rfq_store
                .problem()
                .or_else(|| self.rfq_problem.read().clone());
            snapshot.preflight = Some(if let Some(source) = source {
                let account = account.evidence.as_ref()
                    .filter(|a| inputs.fingerprint.as_deref() == Some(&a.fingerprint))
                    .ok_or("当前账户余额未读取，不能使用旧账户快照")?;
                let mut report = source.restock_report(&snapshot, account,
                    inputs.wallet.as_ref().ok_or("当前钱包余额未读取")?, common::time::now_ms())?;
                report.problems.extend(inputs.problems);
                report
            } else {report(
                &request,
                &snapshot,
                account.evidence.as_ref(),
                inputs,
                common::time::now_ms(),
            )});
        }
        self.publish_rfq(&hub);
        Ok(self.snapshot())
    }

    pub(super) fn restock_source(&self, request: &StockPreflightRequest) -> Result<Option<StockExecutionPlan>, String> {
        let Some(source) = &request.source_plan else { return Ok(None); };
        let plan = self.plan_store.get(&source.plan_id)?;
        if plan.revision != source.revision || plan.request.asset != request.asset
            || request.wallet_address.as_deref() != Some(&plan.request.wallet_address)
            || (self.credential_loader)()?.fingerprint() != plan.terms.account_fingerprint {
            return Err("补库来源计划版本、股票、钱包或账户不一致".into());
        }
        plan.restock_direction()?;
        Ok(Some(plan))
    }

    pub(super) async fn read_preflight_inputs(
        self: &Arc<Self>,
        request: &StockPreflightRequest,
        generation: u64,
        hub: &realtime::WsHub,
    ) -> Result<Inputs, String> {
        self.refresh_context(generation).await?;
        let mut problems = Vec::new();
        if self
            .snapshot()
            .token_metadata_at_ms
            .is_none_or(|t| common::time::now_ms() - t > 30_000)
        {
            match self
                .read("/api/v1/assets")
                .await
                .and_then(|bytes| protocol::asset_context(&bytes, &request.asset))
            {
                Ok((tokens, funding)) => {
                    self.ensure_generation(generation, &request.asset)?;
                    let mut snapshot = self.snapshot.write();
                    if self.generation.load(Ordering::SeqCst) != generation {
                        return Err("股票选择已变化，旧充提状态已丢弃".into());
                    }
                    snapshot.tokens = tokens;
                    snapshot.funding_assets = funding;
                    snapshot.token_metadata_at_ms = Some(common::time::now_ms());
                    snapshot.token_metadata_problem = None;
                }
                Err(_) => {
                    self.ensure_generation(generation, &request.asset)?;
                    let mut snapshot = self.snapshot.write();
                    if self.generation.load(Ordering::SeqCst) != generation {
                        return Err("股票选择已变化，旧充提状态已丢弃".into());
                    }
                    snapshot.token_metadata_problem = Some("官方充提状态读取失败，请勿依赖旧状态转币".into());
                    problems.push("官方充提状态读取失败，请勿依赖旧状态转币".into());
                }
            }
        }
        let account = match (self.credential_loader)() {
            Ok(keys) => {
                self.account_tracking_until_ms
                    .store(common::time::now_ms() + 60_000, Ordering::SeqCst);
                self.order_tracking_until_ms
                    .store(common::time::now_ms() + 60_000, Ordering::SeqCst);
                self.ensure_rfq_started(hub.clone());
                let mut subscription = self.account_subscription.subscribe();
                let fingerprint = keys.fingerprint();
                let prepared = tokio::time::timeout(Duration::from_secs(4), async {
                    loop {
                        if subscription.borrow_and_update().as_deref() == Some(&fingerprint) {
                            return true;
                        }
                        if subscription.changed().await.is_err() {
                            return false;
                        }
                    }
                })
                .await
                .unwrap_or(false);
                if !prepared {
                    problems.push("私有余额 WS 尚未准备好，本次只显示只读账户快照".into());
                }
                match tokio::time::timeout(Duration::from_secs(8), self.read_account(&keys)).await {
                    Ok(Ok(account)) => Some(account),
                    Ok(Err(_)) => {
                        problems
                            .push("Backpack 账户费率或余额读取失败，请检查读取权限与连接".into());
                        None
                    }
                    Err(_) => {
                        problems.push("Backpack 账户读取超时，没有使用旧余额放行".into());
                        None
                    }
                }
            }
            Err(_) => {
                problems
                    .push("请配置 Backpack API 公钥与 Secret seed，才能读取账户费率和库存".into());
                None
            }
        };
        self.ensure_generation(generation, &request.asset)?;
        let mint = self.snapshot().comparison.as_ref().map(|c| c.mint.clone());
        let wallet = match (request.wallet_address.as_deref(), mint.as_ref()) {
            (Some(owner), Some(mint)) => match tokio::time::timeout(
                Duration::from_secs(8),
                super::super::onchain_comparison::stock_inventory::read(owner, mint),
            )
            .await
            {
                Ok(Ok(wallet)) => {
                    problems.extend(wallet.problems.clone());
                    Some(wallet)
                }
                Ok(Err(_)) => {
                    problems.push("Solana 钱包余额未核实，请检查 RPC 与股票 Mint".into());
                    None
                }
                Err(_) => {
                    problems.push("Solana 钱包读取超时，库存保持未知".into());
                    None
                }
            },
            (None, _) => {
                problems.push("未填写 Solana 钱包地址，链上库存与 Gas 未核实".into());
                None
            }
            (_, None) => {
                problems.push("先取得已核实合约的链上报价，再检查钱包库存".into());
                None
            }
        };
        self.ensure_generation(generation, &request.asset)?;
        let now = common::time::now_ms();
        if let Some(owner) = request.wallet_address.as_deref() {
            if let Err(problem) = self.wallet_claims.check("solana", owner, now) {
                problems.push(problem);
            }
        }
        Ok(Inputs {
            fingerprint: account.map(|a| a.fingerprint),
            wallet,
            problems,
        })
    }
}

pub(super) fn report(
    request: &StockPreflightRequest,
    snapshot: &StockMarketSnapshot,
    account: Option<&StockAccountEvidence>,
    inputs: Inputs,
    now: i64,
) -> StockPreflight {
    let account = account.filter(|a| inputs.fingerprint.as_deref() == Some(&a.fingerprint));
    let directions = evaluate_preflight(snapshot, account, inputs.wallet.as_ref(), now);
    let funding = shared_types::stocks::funding::evaluate_funding(snapshot, &directions, account, inputs.wallet.as_ref(), now);
    StockPreflight {
        source_plan: None,
        funding,
        asset: request.asset.clone(),
        wallet_address: request.wallet_address.clone(),
        checked_at_ms: now,
        valid_until_ms: now + 5_000,
        price_basis: StockPriceBasis::from_snapshot(snapshot),
        spot_taker_fee_pct: account
            .as_ref()
            .and_then(|a| Decimal::from_str(&a.spot_taker_fee_bps).ok())
            .and_then(|bps| bps.checked_div(Decimal::from(100)))
            .map(|p| p.normalize().to_string()),
        account_at_ms: account.as_ref().map(|a| a.balances_at_ms),
        wallet_at_ms: inputs.wallet.as_ref().map(|w| w.checked_at_ms),
        directions,
        problems: inputs.problems,
    }
}
