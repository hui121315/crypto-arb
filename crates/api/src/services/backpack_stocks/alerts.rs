use super::*;
use rust_decimal::Decimal;
use shared_types::stocks::comparison::positive;
use shared_types::{WebhookEvent, WebhookEventKind, WEBHOOK_EVENT_VERSION};
use std::sync::Weak;

const METADATA_TTL_MS: i64 = 30_000;

#[derive(Default)]
struct Cursor {
    metadata_retry_at_ms: i64,
    enqueue_retry_at_ms: i64,
}

pub(super) fn needs_worker(s: &StockMarketSnapshot, now: i64) -> bool {
    (s.monitor.enabled && s.monitor.alerts.enabled)
        || s.alerts.recent.iter().any(|r| {
            now.saturating_sub(r.queued_at_ms) < 120_000
                && r.delivery
                    .as_ref()
                    .is_none_or(|d| d.status == shared_types::WebhookDeliveryStatus::Queued)
        })
}

struct Candidate {
    row: shared_types::stocks::comparison::StockDirectionEstimate,
    spread_pct: String,
    input_usdc: String,
    scope: String,
    peer: Option<StockPeerSelection>,
}

impl Candidate {
    fn direction(&self) -> String {
        self.peer
            .as_ref()
            .map(|p| format!("{} · {} · {}", p.venue, p.native_symbol, self.row.direction))
            .unwrap_or_else(|| self.row.direction.into())
    }
}

fn candidates(s: &StockMarketSnapshot, now: i64) -> Result<Vec<Candidate>, String> {
    let threshold = s.monitor.alerts.threshold()?;
    let (mint, _, decimals) = comparison::issuer(s)?;
    let Some(c) = s.comparison.as_ref() else {
        return Ok(vec![]);
    };
    let Some(request) = s.monitor.request.as_ref() else {
        return Ok(vec![]);
    };
    if c.asset != request.asset
        || c.keyed != request.keyed
        || c.mint.address != mint
        || c.mint.decimals != decimals
        || c.buy.input_raw != comparison::budget_raw(request)?
    {
        return Ok(vec![]);
    }
    let mut rows = Vec::new();
    for (index, row) in shared_types::stocks::comparison::evaluate(s, now)
        .into_iter()
        .enumerate()
    {
        let Some(gross) = row
            .gross_usdc
            .as_deref()
            .and_then(shared_types::stocks::comparison::positive)
        else {
            continue;
        };
        let input = if index == 0 {
            c.buy
                .input_raw
                .parse::<u64>()
                .ok()
                .map(|raw| Decimal::from(raw) / Decimal::from(1_000_000))
        } else {
            row.cex_notional_usdc
                .as_deref()
                .and_then(shared_types::stocks::comparison::positive)
        };
        let Some(input) = input.filter(|v| *v > Decimal::ZERO) else {
            continue;
        };
        let Some(pct) = gross
            .checked_div(input)
            .and_then(|v| v.checked_mul(Decimal::from(100)))
        else {
            continue;
        };
        if pct < threshold {
            continue;
        }
        let identity = serde_json::to_vec(&(
            c.asset.as_str(),
            s.security.as_ref().and_then(|s| s.cusip.as_deref()),
            mint,
            index,
        ))
        .map_err(|e| e.to_string())?;
        let hash = common::signing::hmac_sha256_hex(b"stock-spread-monitor-v1", &identity);
        rows.push(Candidate {
            row,
            spread_pct: pct.normalize().to_string(),
            input_usdc: input.normalize().to_string(),
            scope: format!("stock-spread-{}", &hash[..24]),
            peer: None,
        });
    }
    if s.monitor.alerts.include_peer {
        rows.extend(peer_candidates(s, now, threshold)?);
    }
    Ok(rows)
}

fn peer_candidates(
    s: &StockMarketSnapshot,
    now: i64,
    threshold: Decimal,
) -> Result<Vec<Candidate>, String> {
    let (Some(peer), Some(c)) = (&s.peer, &s.comparison) else {
        return Ok(vec![]);
    };
    let mut rows = vec![];
    for r in evaluate_peer(s, now) {
        let Some(gross) = r.gross_usdc.as_deref().and_then(positive) else {
            continue;
        };
        let input = if r.chain_buy {
            c.buy
                .input_raw
                .parse::<u64>()
                .ok()
                .map(|n| Decimal::from(n) / Decimal::from(1_000_000))
        } else {
            r.cex_notional_usdc.as_deref().and_then(positive)
        };
        let Some(input) = input.filter(|n| *n > Decimal::ZERO) else {
            continue;
        };
        let Some(pct) = gross
            .checked_div(input)
            .and_then(|n| n.checked_mul(Decimal::from(100)))
        else {
            continue;
        };
        if pct < threshold {
            continue;
        }
        let identity = serde_json::to_vec(&(
            &c.asset,
            &c.mint.address,
            &peer.selection,
            &peer.identity.underlying_isin,
            &peer.identity.product_isin,
            r.chain_buy,
        ))
        .map_err(|e| e.to_string())?;
        let hash = common::signing::hmac_sha256_hex(b"stock-peer-spread-v1", &identity);
        rows.push(Candidate {
            row: shared_types::stocks::comparison::StockDirectionEstimate {
                direction: if r.chain_buy {
                    "链买 / 所选交易所卖"
                } else {
                    "所选交易所买 / 链卖"
                },
                shares: r.shares,
                gross_usdc: r.gross_usdc,
                cex_notional_usdc: r.cex_notional_usdc,
                minimum_output: None,
                remainder_shares: r.remainder_shares,
                blockers: r.blockers,
            },
            spread_pct: pct.normalize().to_string(),
            input_usdc: input.normalize().to_string(),
            scope: format!("stock-peer-spread-{}", &hash[..24]),
            peer: Some(peer.selection.clone()),
        });
    }
    Ok(rows)
}

fn event_id(candidate: &Candidate, cooldown: u32, now: i64) -> String {
    format!(
        "{}-{}",
        candidate.scope,
        now.div_euclid(i64::from(cooldown) * 1000)
    )
}

fn cooling(w: &webhook::WebhookDispatcher, c: &Candidate, cooldown: u32, now: i64) -> bool {
    // Checking the preceding bucket also prevents back-to-back alerts across a boundary/restart.
    w.event_known(&event_id(c, cooldown, now))
        || w.event_known(&event_id(
            c,
            cooldown,
            now.saturating_sub(i64::from(cooldown) * 1000),
        ))
}

fn transfer_current(s: &StockMarketSnapshot, now: i64) -> bool {
    s.token_metadata_problem.is_none()
        && s.token_metadata_at_ms
            .is_some_and(|t| now >= t && now - t <= METADATA_TTL_MS)
}

fn flag(v: Option<bool>) -> &'static str {
    match v {
        Some(true) => "开放",
        Some(false) => "关闭",
        None => "未知",
    }
}

fn observation(s: &StockMarketSnapshot, c: &Candidate, now: i64) -> serde_json::Value {
    if c.peer.is_some() {
        return peer_observation(s, c, now);
    }
    let comparison = s.comparison.as_ref().unwrap();
    let security = s.security.as_ref().unwrap();
    let transfer = transfer_current(s, now)
        .then(|| {
            s.tokens.iter().find(|t| {
                t.blockchain == "Solana"
                    && t.contract_address.as_deref() == Some(comparison.mint.address.as_str())
            })
        })
        .flatten();
    let preflight = s.preflight.as_ref().filter(|p| p.current(s, now));
    let direction =
        preflight.and_then(|p| p.directions.iter().find(|r| r.direction == c.row.direction));
    let inventory = direction.map(|d| d.inventory.clone()).unwrap_or_default();
    let inventory_note = if inventory.is_empty() {
        "库存未核验：需准备双边对应资产与 SOL；未读取账户".to_owned()
    } else {
        inventory
            .iter()
            .map(|r| {
                format!(
                    "{} {}：{}",
                    r.location,
                    r.asset,
                    match r.sufficient {
                        Some(true) => "数量足够",
                        Some(false) => "余额不足",
                        None => "数量未知",
                    }
                )
            })
            .collect::<Vec<_>>()
            .join("；")
    };
    let transfer_note = format!(
        "Solana 股票充值{} / 提现{}；到账、限额与白名单仍需执行前确认",
        flag(transfer.and_then(|t| t.deposit_enabled)),
        flag(transfer.and_then(|t| t.withdraw_enabled))
    );
    let known = direction.and_then(|r| r.after_known_costs_usdc.as_deref());
    let funding_snapshot = s
        .preflight
        .as_ref()
        .filter(|p| {
            p.asset == security.asset && now >= p.checked_at_ms && now - p.checked_at_ms <= 30_000
        })
        .and_then(|p| {
            p.funding
                .iter()
                .find(|f| f.direction.label() == c.row.direction)
                .map(|f| (p.checked_at_ms, f))
        });
    let funding_note = funding_snapshot
        .map(|(_, f)| {
            f.needs
                .iter()
                .map(|n| {
                    format!(
                        "{} {} 缺 {}，可从 {} 核查补入（来源可调 {}）",
                        n.target,
                        n.asset,
                        n.shortfall.as_deref().unwrap_or("未知"),
                        n.source,
                        n.source_spare.as_deref().unwrap_or("未知")
                    )
                })
                .collect::<Vec<_>>()
                .join("；")
        })
        .filter(|note| !note.is_empty())
        .map(|note| format!("\n上次补库检查：{note}；需按当前金额复核，未转币。"))
        .unwrap_or_default();
    let mut blockers = c.row.blockers.clone();
    if let Some(p) = preflight {
        blockers.extend(p.problems.clone());
    }
    if let Some(d) = direction {
        blockers.extend(d.blockers.clone());
    }
    if transfer.is_none() {
        blockers.push("未取得新鲜的官方充提状态，不能假定可转币循环".into());
    }
    if transfer.is_some_and(|t| t.deposit_enabled != Some(true) || t.withdraw_enabled != Some(true))
    {
        blockers.push("股票充提受限；预置库存比较不等于买入后可以转移".into());
    }
    let message = format!(
        "{} · {}\n报价差额 +{}% / +{} USDC · 投入 {} USDC\n已知费用后差额 {}；完整净收益未核齐\n{}\n{}{funding_note}\n仅观察，不是已锁定利润；未下单、未转币。",
        security.ticker, c.row.direction, c.spread_pct, c.row.gross_usdc.as_deref().unwrap(), c.input_usdc,
        known.map(|v|format!("{v} USDC")).unwrap_or_else(||"未知".into()), inventory_note, transfer_note,
    );
    serde_json::json!({
        "classification":"stock_spread_observation", "message":message,
        "asset":security.asset,"ticker":security.ticker,"cusip":security.cusip,
        "chain":"solana","mint":comparison.mint.address,"direction":c.row.direction,
        "shares":c.row.shares,"inputUsdc":c.input_usdc,"grossUsdc":c.row.gross_usdc,"spreadPct":c.spread_pct,
        "afterKnownCostsUsdc":known,"completeNetUsdc":null,"executable":false,"fundAction":false,
        "inventory":inventory,"inventoryCheckedAtMs":preflight.map(|p|p.checked_at_ms),
        "transfer":transfer,"transferCheckedAtMs":transfer.and(s.token_metadata_at_ms),
        "fundingAssets": if transfer_current(s,now) {Some(&s.funding_assets)}else{None},
        "lastFundingCheck":funding_snapshot.map(|(at,f)|serde_json::json!({"checkedAtMs":at,"direction":f,"currentExecutionPermission":false})),
        "tradingRoute":s.trading_route,"blockers":blockers,"observedAtMs":now,
        "priceBasis":StockPriceBasis::from_snapshot(s),"view":"#stocks",
    })
}

fn peer_observation(s: &StockMarketSnapshot, c: &Candidate, now: i64) -> serde_json::Value {
    let peer = s.peer.as_ref().expect("validated peer candidate");
    let comparison = s.comparison.as_ref().expect("validated chain quote");
    let security = s.security.as_ref().expect("validated security");
    let funding=s.peer_funding.as_ref().filter(|r|r.current(s,now));
    let funding_note=funding.map(|f|f.routes.iter().map(|r| {
        let state=if r.problem.is_some(){"读取未完成"}else if r.methods.is_empty(){"未返回可用方法"}
            else if r.methods.iter().any(|m|peer_funding_contract_matches(s,r,m)==Some(true)){"合约匹配，地址与额度待查"}
            else if r.methods.iter().any(|m|peer_funding_contract_matches(s,r,m).is_none()){"合约待核实"}else{"非当前链上合约或网络"};
        format!("{} {}：{state}",r.asset,r.direction.label())
    }).collect::<Vec<_>>().join("；")).unwrap_or_else(||"所选交易所充提尚未检查或已过期".into());
    let checked=s.peer_preflight.as_ref().filter(|r|r.selection==peer.selection && r.asset==security.asset && now>=r.checked_at_ms && now-r.checked_at_ms<=15_000);
    let readiness=checked.and_then(|_|evaluate_peer_preflight(s,now).into_iter().find(|r|r.chain_buy==(c.row.direction=="链买 / 所选交易所卖")));
    let inventory=readiness.as_ref().map(|r|r.inventory.clone()).unwrap_or_default();
    let note=readiness.as_ref().map(|r|format!("已知费用后差额 {} USDC；{}",
        r.after_known_costs_usdc.as_deref().unwrap_or("未知"),r.inventory.iter().map(|i|format!("{} {}：{}",i.location,i.asset,
            match i.sufficient {Some(true)=>"足够",Some(false)=>"不足",None=>"待核实"})).collect::<Vec<_>>().join("；")))
        .unwrap_or_else(||"交易费、换汇费、Gas 与该场所库存尚未核齐。".into());
    let message = format!("{} · {}\n费用前差额 +{}% / +{} USDC · 对齐 {} 股\n{note}\n{funding_note}\n不同发行方，不能直接互相充值；需预置双边库存，不能买完直接搬币。\n仅观察，不是已锁定利润；未下单、未转币。",
        security.ticker, c.direction(), c.spread_pct, c.row.gross_usdc.as_deref().unwrap(), c.row.shares.as_deref().unwrap_or("未知"));
    serde_json::json!({
        "classification":"stock_peer_spread_observation", "message":message,
        "asset":security.asset,"ticker":security.ticker,"cusip":security.cusip,
        "chain":"solana","mint":comparison.mint.address,"direction":c.direction(),
        "peer":peer.selection,"identity":peer.identity,"shareUnitVerified":peer.share_unit_verified,
        "quote":peer.quote,"quoteConversion":peer.quote_conversion,
        "shares":c.row.shares,"remainderShares":c.row.remainder_shares,
        "inputUsdc":c.input_usdc,"grossUsdc":c.row.gross_usdc,"spreadPct":c.spread_pct,
        "afterKnownCostsUsdc":readiness.as_ref().and_then(|r|r.after_known_costs_usdc.as_deref()),"completeNetUsdc":null,"executable":false,"fundAction":false,
        "inventory":inventory,"inventoryCheckedAtMs":checked.map(|r|r.checked_at_ms),"readiness":readiness,
        "transfer":{"directTransferSupported":false,"reason":"different_issuer_products","venueDepositEnabled":null,"venueWithdrawEnabled":null},
        "peerFunding":funding,
        "blockers":c.row.blockers,"observedAtMs":now,"view":"#stocks",
    })
}

impl BackpackStocks {
    fn update_alert_status(
        &self,
        hub: &realtime::WsHub,
        change: impl FnOnce(&mut StockAlertRuntime),
    ) {
        let mut s = self.snapshot.write();
        let old = s.alerts.clone();
        change(&mut s.alerts);
        if s.alerts == old {
            return;
        }
        s.observed_at_ms = common::time::now_ms().max(s.observed_at_ms.saturating_add(1));
        drop(s);
        self.publish(hub);
    }

    fn alert_phase(
        &self,
        hub: &realtime::WsHub,
        generation: u64,
        phase: StockAlertPhase,
        problem: Option<String>,
    ) {
        self.update_alert_status(hub, |s| {
            if self.generation.load(Ordering::SeqCst) != generation {
                return;
            }
            s.phase = phase;
            s.problem = problem;
        });
    }

    async fn alert_once(&self, hub: &realtime::WsHub, cursor: &mut Cursor) {
        if let Some(w) = &self.webhook {
            let status = w.status(common::time::now_ms()).await;
            self.update_alert_status(hub, |s| {
                for row in &mut s.recent {
                    if let Some(delivery) = status
                        .recent_deliveries
                        .iter()
                        .find(|r| r.event_id == row.event_id)
                    {
                        row.delivery = Some(delivery.clone());
                    }
                }
            });
        }
        let generation = self.generation.load(Ordering::SeqCst);
        let initial = self.snapshot();
        if !initial.monitor.enabled || !initial.monitor.alerts.enabled {
            self.alert_phase(hub, generation, StockAlertPhase::Disabled, None);
            return;
        }
        let Some(w) = self.webhook.as_ref().filter(|w| w.durable_outbox()) else {
            self.alert_phase(
                hub,
                generation,
                StockAlertPhase::NeedsWebhook,
                Some("Webhook 出站日志未就绪，不能发送股票提醒".into()),
            );
            return;
        };
        if !w.enabled_for(WebhookEventKind::StockSpread) {
            self.alert_phase(
                hub,
                generation,
                StockAlertPhase::NeedsWebhook,
                Some("请在设置启用 Webhook，并订阅“股票价差观察”事件".into()),
            );
            return;
        }
        let now = common::time::now_ms();
        if now < cursor.enqueue_retry_at_ms {
            return;
        }
        let candidates = match candidates(&initial, now) {
            Ok(rows) => rows,
            Err(e) => {
                self.alert_phase(hub, generation, StockAlertPhase::Degraded, Some(e));
                return;
            }
        };
        if candidates.is_empty() {
            self.alert_phase(
                hub,
                generation,
                if shared_types::stocks::comparison::evaluate(&initial, now)
                    .iter()
                    .any(|r| r.gross_usdc.is_some())
                    || (initial.monitor.alerts.include_peer
                        && evaluate_peer(&initial, now)
                            .iter()
                            .any(|r| r.gross_usdc.is_some()))
                {
                    StockAlertPhase::Watching
                } else {
                    StockAlertPhase::WaitingQuotes
                },
                None,
            );
            return;
        }
        let cooldown = initial.monitor.alerts.cooldown_secs;
        if candidates.iter().all(|c| cooling(w, c, cooldown, now)) {
            self.alert_phase(hub, generation, StockAlertPhase::Cooldown, None);
            return;
        }
        let asset = initial.security.as_ref().unwrap().asset.clone();
        if !transfer_current(&initial, now) && now >= cursor.metadata_retry_at_ms {
            cursor.metadata_retry_at_ms = now + METADATA_TTL_MS;
            let result = tokio::time::timeout(Duration::from_secs(8), self.read("/api/v1/assets"))
                .await
                .map_err(|_| "股票充提状态读取超时".to_owned())
                .and_then(|r| r)
                .and_then(|b| protocol::asset_context(&b, &asset));
            if self.ensure_generation(generation, &asset).is_err() {
                return;
            }
            let mut s = self.snapshot.write();
            if self.generation.load(Ordering::SeqCst) != generation
                || s.security
                    .as_ref()
                    .is_none_or(|security| security.asset != asset)
            {
                return;
            }
            match result {
                Ok((tokens, funding)) => {
                    s.tokens = tokens;
                    s.funding_assets = funding;
                    s.token_metadata_at_ms = Some(common::time::now_ms());
                    s.token_metadata_problem = None;
                }
                Err(_) => {
                    s.token_metadata_problem =
                        Some("官方充提状态暂不可用；提醒中按未知处理".into());
                }
            }
            s.observed_at_ms = common::time::now_ms().max(s.observed_at_ms.saturating_add(1));
            drop(s);
            self.publish(hub);
        }
        // Re-evaluate after metadata I/O: old quotes or a stopped selection must not enter the outbox.
        if self.ensure_generation(generation, &asset).is_err() {
            return;
        }
        let current = self.snapshot();
        let now = common::time::now_ms();
        let rows = match candidates_for_send(&current, now) {
            Ok(rows) => rows,
            Err(e) => {
                self.alert_phase(hub, generation, StockAlertPhase::Degraded, Some(e));
                return;
            }
        };
        if rows.is_empty() {
            self.alert_phase(hub, generation, StockAlertPhase::WaitingQuotes, None);
            return;
        }
        for c in rows {
            if cooling(w, &c, cooldown, now) {
                continue;
            }
            let id = event_id(&c, cooldown, now);
            let event = WebhookEvent {
                id: id.clone(),
                version: WEBHOOK_EVENT_VERSION.into(),
                kind: WebhookEventKind::StockSpread,
                occurred_at_ms: now,
                payload: observation(&current, &c, now),
            };
            if self.ensure_generation(generation, &asset).is_err() {
                return;
            }
            let result = w.enqueue(event, false).await;
            if self.ensure_generation(generation, &asset).is_err() {
                return;
            }
            if result.is_err() {
                cursor.enqueue_retry_at_ms = now + 5_000;
                self.alert_phase(
                    hub,
                    generation,
                    StockAlertPhase::Degraded,
                    Some("股票提醒未能写入出站队列；5 秒后重新检查最新报价".into()),
                );
                return;
            }
            if !w.event_known(&id) {
                return;
            }
            self.update_alert_status(hub, |s| {
                if self.generation.load(Ordering::SeqCst) != generation {
                    return;
                }
                let direction = c.direction();
                s.recent.retain(|r| r.direction != direction);
                s.recent.insert(
                    0,
                    StockAlertSummary {
                        event_id: id,
                        direction,
                        gross_usdc: c.row.gross_usdc.unwrap(),
                        spread_pct: c.spread_pct,
                        queued_at_ms: now,
                        delivery: None,
                    },
                );
                s.recent.truncate(8);
                s.phase = StockAlertPhase::Queued;
                s.problem = None;
            });
        }
    }
}

fn candidates_for_send(s: &StockMarketSnapshot, now: i64) -> Result<Vec<Candidate>, String> {
    if !s.monitor.enabled || !s.monitor.alerts.enabled {
        return Ok(vec![]);
    }
    candidates(s, now)
}

pub(super) async fn run(service: Weak<BackpackStocks>, hub: realtime::WsHub) {
    let mut cursor = Cursor::default();
    let mut tick = tokio::time::interval(Duration::from_millis(500));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tick.tick().await;
        let Some(s) = service.upgrade() else { break };
        s.alert_once(&hub, &mut cursor).await;
        let snapshot = s.snapshot.read();
        if !needs_worker(&snapshot, common::time::now_ms()) {
            break;
        }
    }
}

#[cfg(test)]
pub(super) mod tests;
