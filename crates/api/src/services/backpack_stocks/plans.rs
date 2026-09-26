use super::*;
use rust_decimal::Decimal;

impl BackpackStocks {
    pub(super) fn with_plan_costs<T>(
        &self,
        plan: &StockExecutionPlan,
        apply: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        self.exchange_conversion_store.with_costs(
            &plan.conversion_cost_ids(),
            &plan.terms.account_fingerprint,
            |sources| {
                plan.check_conversion_sources(&sources)?;
                apply()
            },
        )
    }

    pub(super) fn begin_stock_rfq(
        &self,
        id: &str,
        fingerprint: &str,
    ) -> Result<(StockExecutionPlan, bool), String> {
        let old = self.plan_store.get(id)?;
        if old.terms.account_fingerprint != fingerprint {
            return Err("股票计划属于其他账户凭证".into());
        }
        if old.rfq_acceptance.is_some() {
            return Ok((old, false));
        }
        self.with_plan_costs(&old, || {
            // Source costs precede RFQ/account locks in the same order as plan building.
            let _state = self.rfq_state_lock.lock();
            let account = self.account.read();
            let current = self.snapshot.read();
            let mut snapshot = current.clone();
            snapshot.rfqs = self.visible_rfqs();
            snapshot.rfq_connected = self.rfq_subscription.borrow().as_deref() == Some(fingerprint);
            snapshot.rfq_problem = self
                .rfq_store
                .problem()
                .or_else(|| self.rfq_problem.read().clone());
            let now = common::time::now_ms();
            let evidence = account
                .evidence
                .as_ref()
                .filter(|a| a.fingerprint == fingerprint)
                .ok_or("当前账户证据已失效，请重新预检")?;
            validate_for_submission(&old, &snapshot, evidence, now)?;
            let binding = old.terms.rfq.as_ref().ok_or("计划未绑定 RFQ")?;
            let record = self
                .stock_rfq(&binding.request_id)
                .ok_or("原 RFQ 缺失，不能提交")?;
            self.plan_store.begin_rfq(id, fingerprint, record, now)
        })
    }

    pub(super) fn begin_stock_order(
        &self,
        id: &str,
        fingerprint: &str,
    ) -> Result<(StockExecutionPlan, bool), String> {
        let old = self.plan_store.get(id)?;
        if old.terms.account_fingerprint != fingerprint {
            return Err("股票计划属于其他账户凭证".into());
        }
        if old.cex_order.is_some() {
            return Ok((old, false));
        }
        self.with_plan_costs(&old, || {
            let account = self.account.read();
            let snapshot = self.snapshot.read();
            let now = common::time::now_ms();
            let evidence = account
                .evidence
                .as_ref()
                .filter(|a| a.fingerprint == fingerprint)
                .ok_or("当前账户证据已失效，请重新预检")?;
            validate_for_submission(&old, &snapshot, evidence, now)?;
            self.plan_store.begin_order(id, fingerprint, now)
        })
    }

    pub(crate) fn with_plan_store(mut self, path: std::path::PathBuf) -> Self {
        self.plan_store = plan_store::PlanStore::load(Some(path), self.wallet_claims.clone());
        self
    }

    pub(crate) fn reserve_plan(
        &self,
        mut request: StockPlanRequest,
        hub: &realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        request.wallet_address = request.wallet_address.trim().into();
        let keys = (self.credential_loader)()?;
        let fingerprint = keys.fingerprint();
        if self.plan_store.previous(&request, &fingerprint)?.is_none() {
            let _preflight = self
                .preflight_lock
                .try_lock()
                .map_err(|_| "股票预检进行中")?;
            let _quote = self
                .quote_lock
                .try_lock()
                .map_err(|_| "股票询价进行中，请等待完整结果")?;
            let cost_ids = request
                .build
                .as_ref()
                .map(|b| b.conversion_cost_ids.clone())
                .unwrap_or_default();
            self.exchange_conversion_store
                .with_costs(&cost_ids, &fingerprint, |costs| {
                    let account = self.account.read();
                    let current = self.snapshot.read();
                    let mut snapshot = current.clone();
                    snapshot.exchange_conversions = costs;
                    snapshot.rfqs = self.visible_rfqs();
                    snapshot.rfq_connected =
                        self.rfq_subscription.borrow().as_deref() == Some(&fingerprint);
                    snapshot.rfq_problem = self
                        .rfq_store
                        .problem()
                        .or_else(|| self.rfq_problem.read().clone());
                    let now = common::time::now_ms();
                    let evidence = account
                        .evidence
                        .as_ref()
                        .filter(|a| a.fingerprint == fingerprint)
                        .ok_or("账户凭证或余额已变化，请重新预检")?;
                    let plan = prepare(request, &snapshot, evidence, now)?;
                    self.plan_store.reserve(plan, now)?;
                    Ok(())
                })?;
        }
        self.publish_plan(hub);
        Ok(self.snapshot())
    }

    pub(crate) fn cancel_plan(
        &self,
        id: &str,
        hub: &realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        // This cancels only a local, unsubmitted reservation, never an exchange order.
        self.plan_store.cancel(id, common::time::now_ms())?;
        self.publish_plan(hub);
        Ok(self.snapshot())
    }

    pub(super) fn publish_plan(&self, hub: &realtime::WsHub) {
        {
            let mut snapshot = self.snapshot.write();
            snapshot.observed_at_ms =
                common::time::now_ms().max(snapshot.observed_at_ms.saturating_add(1));
        }
        self.publish(hub);
    }
}

pub(super) fn prepare(
    request: StockPlanRequest,
    s: &StockMarketSnapshot,
    account: &StockAccountEvidence,
    now: i64,
) -> Result<StockExecutionPlan, String> {
    if request.build.as_ref().is_some_and(|b| {
        b.request_id != request.request_id
            || b.asset != request.asset
            || b.direction != request.direction
            || b.wallet_address != request.wallet_address
            || s.comparison.as_ref().is_none_or(|c| {
                c.keyed != b.keyed
                    || b.direction
                        .quote(c)
                        .is_none_or(|q| q.input_raw != b.input_raw)
            })
    }) {
        return Err("构建参数与当前计划不一致".into());
    }
    let report = s
        .preflight
        .as_ref()
        .filter(|p| {
            p.asset == request.asset
                && p.checked_at_ms == request.preflight_at_ms
                && p.wallet_address.as_deref() == Some(&request.wallet_address)
                && p.current(s, now)
        })
        .ok_or("预检或报价已变化，请重新检查库存与成本")?;
    if !report.problems.is_empty()
        || account.liquidating
        || now < account.balances_at_ms
        || now - account.balances_at_ms > 30_000
        || report.account_at_ms != Some(account.balances_at_ms)
        || report
            .wallet_at_ms
            .is_none_or(|t| now < t || now - t > 30_000)
    {
        return Err("账户、钱包或共享占用预检未通过，不能预留资金".into());
    }
    if s.trading_route
        .as_ref()
        .is_some_and(|r| r.kind == StockRouteKind::OrderBook)
        && (now < account.fees_at_ms
            || now - account.fees_at_ms > 300_000
            || Decimal::from_str_exact(&account.spot_taker_fee_bps)
                .ok()
                .and_then(|n| n.checked_div(Decimal::from(100)))
                != report
                    .spot_taker_fee_pct
                    .as_deref()
                    .and_then(|s| Decimal::from_str_exact(s).ok()))
    {
        return Err("账户费率在预检后变化或已过期，请重新预检".into());
    }
    let (mint, _, decimals) = comparison::issuer(s)?;
    let cost = s
        .chain_costs
        .iter()
        .find(|c| {
            c.direction == request.direction
                && c.current(s, &request.wallet_address, now)
                && c.simulation_passed
                && c.problems.is_empty()
                && c.wallet_required_lamports.is_some()
                && c.complete_native_usdc_budget(now).is_some()
                && c.mint.address == mint
                && c.mint.decimals == decimals
        })
        .ok_or("该方向缺少有效的链上费用、周转余额或 SOL 补回报价")?;
    let row = report
        .directions
        .iter()
        .find(|r| r.direction == request.direction.label())
        .ok_or("缺少该方向库存预检")?;
    let allocations = row
        .inventory
        .iter()
        .map(|i| {
            if i.sufficient != Some(true) {
                return Err("双边库存或 SOL 周转余额不足/未知".to_owned());
            }
            Ok(StockPlanAllocation {
                location: i.location.clone(),
                asset: i.asset.clone(),
                quantity: i.required.clone().ok_or("所需数量未知")?,
                available_at_reservation: i.available.clone().ok_or("可用数量未知")?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let needs_topup = request.direction == StockChainDirection::Sell
        && cost.wallet_debit_lamports.as_deref() != Some("0");
    if allocations.len() != if needs_topup { 4 } else { 3 } {
        return Err("库存预检缺少完整的两腿与 SOL 备款".into());
    }
    let cex = &allocations[0];
    if cex.location != "Backpack"
        || account.balances.get(&cex.asset).is_none_or(|b| {
            b.available != cex.available_at_reservation
                || now < b.observed_at_ms
                || now - b.observed_at_ms > 30_000
        })
    {
        return Err("Backpack 可用余额在预检后变化，请重新预检".into());
    }
    let difference = row
        .after_known_costs_usdc
        .clone()
        .filter(|n| positive(n).is_some())
        .ok_or("已知成本后没有正差额，不预留资金")?;
    let estimate = shared_types::stocks::comparison::evaluate(s, now)
        .into_iter()
        .find(|r| r.direction == request.direction.label())
        .ok_or("没有该方向报价")?;
    let quantity = estimate
        .shares
        .as_deref()
        .and_then(positive)
        .ok_or("对冲股数未知")?;
    let notional = estimate
        .cex_notional_usdc
        .as_deref()
        .and_then(positive)
        .ok_or("交易所可成交金额未知")?;
    let route = s.trading_route.as_ref().ok_or("交易时段未核验")?.clone();
    let rfq = if route.kind == StockRouteKind::Rfq {
        let side = if request.direction == StockChainDirection::Buy {
            StockRfqSide::Ask
        } else {
            StockRfqSide::Bid
        };
        let record = s
            .rfqs
            .iter()
            .find(|r| {
                r.request.asset == request.asset
                    && r.request.side == side
                    && r.account_fingerprint == account.fingerprint
                    && positive(&r.request.quantity) == Some(quantity)
                    && r.current_candidate(s.rfq_connected, now)
                        .and_then(|q| positive(&q.taker_price)?.checked_mul(quantity))
                        == Some(notional)
            })
            .ok_or("RFQ 候选、股数或账户已变化，不能预留旧报价")?;
        Some(StockPlanRfq {
            request_id: record.request.request_id.clone(),
            rfq_id: record.rfq_id.clone().ok_or("RFQ 远端编号未知")?,
            candidate: record.candidate.clone().ok_or("RFQ 候选缺失")?,
            expiry_time_ms: record.expiry_time_ms.ok_or("RFQ 到期时间未知")?,
        })
    } else if route.kind == StockRouteKind::OrderBook {
        None
    } else {
        return Err("当前时段没有可用的股票交易通道".into());
    };
    let book_valid_until = if route.kind == StockRouteKind::OrderBook {
        s.books
            .iter()
            .find(|b| Some(b.symbol.as_str()) == route.symbol.as_deref())
            .map(|b| b.source_at_ms.saturating_add(3001))
            .ok_or("缺少订单簿有效期")?
    } else {
        i64::MAX
    };
    let valid_until = report
        .valid_until_ms
        .min(cost.valid_until_ms)
        .min(route.valid_until_ms)
        .min(book_valid_until)
        .min(quote_valid_until(&cost.quote))
        .min(
            cost.native_valuation
                .as_ref()
                .map(|v| {
                    quote_valid_until(&v.quote).min(
                        v.replenishment
                            .as_ref()
                            .map(|p| p.valid_until_ms)
                            .unwrap_or(i64::MAX),
                    )
                })
                .unwrap_or(i64::MAX),
        )
        .min(cost.mint.checked_at_ms.saturating_add(60_001))
        .min(cost.mint.next_change_at_ms.unwrap_or(i64::MAX))
        .min(rfq.as_ref().map(|r| r.expiry_time_ms).unwrap_or(i64::MAX));
    if now >= valid_until {
        return Err("市场证据已过期，未保存或预留资金".into());
    }
    let side = if request.direction == StockChainDirection::Buy {
        StockRfqSide::Ask
    } else {
        StockRfqSide::Bid
    };
    let basis = if let Some(rfq) = &rfq {
        StockCexFeeBasis::RfqIncluded {
            quote_id: rfq.candidate.quote_id.clone(),
        }
    } else {
        StockCexFeeBasis::OrderBookQuote {
            taker_bps: account.spot_taker_fee_bps.clone(),
            observed_at_ms: account.fees_at_ms,
        }
    };
    let fee_budget = StockCexFeeBudget::calculate(&notional.to_string(), side, basis)
        .ok_or("交易所费用预算无效")?;
    if fee_budget.required(side, &quantity.to_string()).as_deref() != Some(cex.quantity.as_str())
        || row.cex_fee_usdc.as_deref() != Some(fee_budget.additional_fee.as_str())
    {
        return Err("交易所手续费、备款与预检不一致，请重新预检".into());
    }
    let mut terms = StockPlanTerms {
        account_fingerprint: account.fingerprint.clone(),
        security: s.security.clone().ok_or("证券身份未知")?,
        chain_cost: cost.clone(),
        route,
        cex_shares: quantity.normalize().to_string(),
        cex_notional_usdc: notional.normalize().to_string(),
        rfq,
        allocations,
        after_known_costs_usdc: difference,
        created_at_ms: now,
        market_valid_until_ms: valid_until,
        reserved_until_ms: now.saturating_add(60_000),
        cex_instruction: None,
        cex_fee_budget: Some(fee_budget),
        preflight_evidence: Some(report.clone()),
        conversion_costs: request
            .build
            .as_ref()
            .map(|b| {
                b.conversion_cost_ids
                    .iter()
                    .map(|id| {
                        s.exchange_conversions
                            .iter()
                            .find(|p| &p.plan_id == id)
                            .cloned()
                            .ok_or_else(|| "原兑换费用记录缺失".to_owned())
                    })
                    .collect::<Result<Vec<_>, String>>()
            })
            .transpose()?
            .unwrap_or_default(),
    };
    let difference = Decimal::from_str_exact(&terms.after_known_costs_usdc)
        .map_err(|_| "已知费用后差额无效")?
        .checked_sub(terms.conversion_fee_usdc()?)
        .ok_or("费用归集溢出")?;
    if difference <= Decimal::ZERO {
        return Err("计入已付兑换手续费后没有正差额，未预留资金".into());
    }
    terms.after_known_costs_usdc = difference.normalize().to_string();
    terms.cex_instruction = Some(order_compile::compile(&request, &terms)?);
    Ok(StockExecutionPlan {
        plan_id: plan_store::plan_id(&request, &terms)?,
        request,
        terms,
        phase: StockPlanPhase::Reserved,
        revision: 1,
        updated_at_ms: now,
        cex_order: None,
        rfq_acceptance: None,
        chain_submission: None,
        two_leg_started_at_ms: None,
        native_topups: vec![],
        recoveries: vec![],
        settlement: None,
    })
}

pub(super) fn validate_for_submission(
    plan: &StockExecutionPlan,
    snapshot: &StockMarketSnapshot,
    account: &StockAccountEvidence,
    now: i64,
) -> Result<(), String> {
    plan.submission_market_check(snapshot, now)?;
    let t = &plan.terms;
    let proof = t.preflight_evidence.as_ref().ok_or("原预检缺失")?;
    let (mint, _, decimals) = comparison::issuer(snapshot)?;
    if mint != t.chain_cost.mint.address || decimals != t.chain_cost.mint.decimals {
        return Err("官方证券与原链上合约不一致".into());
    }
    let fresh = |at: i64, age: i64| at > 0 && now >= at && now.saturating_sub(at) <= age;
    let decimal = |s: &str| Decimal::from_str_exact(s).ok();
    if account.fingerprint != t.account_fingerprint
        || account.liquidating
        || !fresh(account.balances_at_ms, 30_000)
        || proof
            .account_at_ms
            .is_none_or(|at| account.balances_at_ms < at)
    {
        return Err("原账户余额已过期、回退或正在清算".into());
    }
    let cex = t.allocations.first().ok_or("原计划缺少交易所备款")?;
    if cex.location != "Backpack"
        || account.balances.get(&cex.asset).is_none_or(|b| {
            !fresh(b.observed_at_ms, 30_000)
                || decimal(&b.available)
                    .zip(decimal(&cex.available_at_reservation))
                    .is_none_or(|(available, original)| available < original)
        })
    {
        return Err("Backpack 原备款余额已减少或未知，请重新构建".into());
    }
    if let StockCexFeeBasis::OrderBookQuote {
        taker_bps,
        observed_at_ms,
    } = &t.cex_fee_budget.as_ref().ok_or("原费用预算缺失")?.basis
    {
        if !fresh(account.fees_at_ms, 300_000)
            || account.fees_at_ms < *observed_at_ms
            || decimal(&account.spot_taker_fee_bps) != decimal(taker_bps)
        {
            return Err("Backpack 费率已变化或过期，请重新构建".into());
        }
    }
    // Check changed wallet evidence even within the same clock millisecond or a different draft.
    if let Some(latest) = snapshot.preflight.as_ref().filter(|p| {
        p.asset == proof.asset
            && p.wallet_address == proof.wallet_address
            && *p != proof
            && p.checked_at_ms >= proof.checked_at_ms
            && (p.wallet_at_ms.is_none() || p.wallet_at_ms >= proof.wallet_at_ms)
    }) {
        if latest.wallet_at_ms.is_none_or(|at| !fresh(at, 30_000))
            || !latest.problems.is_empty()
            || t.allocations
                .iter()
                .filter(|a| a.location == "Solana")
                .any(|a| {
                    let rows = latest
                        .directions
                        .iter()
                        .flat_map(|r| &r.inventory)
                        .filter(|i| i.location == a.location && i.asset == a.asset)
                        .collect::<Vec<_>>();
                    rows.is_empty()
                        || rows.iter().any(|i| {
                            i.available
                                .as_deref()
                                .and_then(decimal)
                                .zip(decimal(&a.available_at_reservation))
                                .is_none_or(|(v, original)| v < original)
                        })
                })
        {
            return Err("新钱包样本显示原备款减少或未知，请重新构建".into());
        }
    }
    if t.cex_instruction.as_ref() != Some(&order_compile::compile(&plan.request, t)?) {
        return Err("原股票指令与计划不一致".into());
    }
    Ok(())
}

fn positive(s: &str) -> Option<Decimal> {
    Decimal::from_str_exact(s)
        .ok()
        .filter(|n| *n > Decimal::ZERO)
}

fn quote_valid_until(q: &StockDexQuote) -> i64 {
    q.requested_at_ms
        .saturating_add(shared_types::stocks::comparison::STOCK_QUOTE_MAX_AGE_MS)
        .saturating_add(1)
        .min(q.expires_at_ms.unwrap_or(i64::MAX))
}

#[cfg(test)]
pub(super) mod tests;
