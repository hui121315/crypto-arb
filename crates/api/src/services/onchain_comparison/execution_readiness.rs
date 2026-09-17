use super::quote::raw_units;
use super::wallet_inventory::WALLET_INVENTORY_MAX_AGE_MS;
use crate::services::account_state;
use crate::services::instrument_registry::{
    SpotInstrumentResolution, SpotInstrumentResolutionStatus,
};
use crate::state::AppState;
use onchain_monitor::{OnchainQuotePair, OnchainWalletAssetBalance, OnchainWalletInventory};
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};
use rust_decimal::Decimal;
use shared_types::{
    onchain_cex_base_token, onchain_cex_quote_token, InstrumentAssetClass, OnchainCexComparison,
    OnchainCexInstrumentEvidence, OnchainCexInstrumentStatus, OnchainComparisonDirection,
    OnchainComparisonQuality, OnchainComparisonSnapshot, OnchainDirectionReadiness,
    OnchainExecutionReadiness, OnchainInventoryEvidence, OnchainInventoryLocation,
    OnchainInventoryStatus, OnchainPathAvailability, OnchainPathKind, OnchainPathLeg,
    OnchainPathLegKind, OnchainPathReadiness, OnchainTransferDirection, OnchainTransferEvidence,
    OnchainTransferStatus, VenueBalanceEnvelope, VenueInstrument, EVM_NATIVE_TOKEN_ADDRESS,
};

const CEX_BALANCE_MAX_AGE_MS: i64 = 15_000;

#[cfg(test)]
#[path = "execution_readiness/replenishment_tests.rs"]
mod replenishment_tests;

struct DirectionReadinessContext<'a> {
    state: &'a AppState,
    snapshot: &'a OnchainComparisonSnapshot,
    quotes: &'a OnchainQuotePair,
    wallet: Option<&'a OnchainWalletInventory>,
    cex_balances: &'a VenueBalanceEnvelope,
    now_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct CexInventoryRequirement<'a> {
    asset: &'a str,
    required: f64,
}

#[derive(Clone, Copy)]
struct PathReadinessRequest<'a> {
    snapshot: &'a OnchainComparisonSnapshot,
    comparison: &'a OnchainCexComparison,
    direction: OnchainComparisonDirection,
    inventory: &'a [OnchainInventoryEvidence],
    instrument: &'a OnchainCexInstrumentEvidence,
    market_ready: bool,
    build_ready: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AllowanceBuildGate {
    Proven,
    FirmQuoteCheck,
}

#[derive(Clone, Copy)]
struct WalletEvidenceRequest<'a> {
    snapshot: &'a OnchainComparisonSnapshot,
    wallet: Option<&'a OnchainWalletInventory>,
    selected: Option<&'a OnchainWalletAssetBalance>,
    asset: &'a str,
    required: f64,
    gas: bool,
    now_ms: i64,
}

pub(super) fn attach(
    state: &AppState,
    quotes: &OnchainQuotePair,
    snapshot: &mut OnchainComparisonSnapshot,
    now_ms: i64,
) {
    attach_global(state, snapshot, now_ms);
    let account = account_state::cached_snapshot(state);
    let wallet = state.onchain_monitor().wallet_inventory();
    let context = DirectionReadinessContext {
        state,
        snapshot,
        quotes,
        wallet: wallet.as_deref(),
        cex_balances: &account.balances,
        now_ms,
    };
    let directions = snapshot
        .comparisons
        .iter()
        .filter_map(|row| direction_readiness(&context, row))
        .collect();
    snapshot.execution_readiness.directions = directions;
}

pub(super) fn attach_global(
    state: &AppState,
    snapshot: &mut OnchainComparisonSnapshot,
    now_ms: i64,
) {
    let wallet_address_configured = !snapshot.config.wallet_address.trim().is_empty();
    let chain_problem = if wallet_address_configured {
        super::execution_submit::readiness(state, &snapshot.config).err()
    } else {
        Some("链上钱包地址未配置".to_owned())
    };
    let risk = state.trading_service().risk_config();
    let cex_problem = if !risk.live_trading_enabled {
        Some("实盘总开关未开启".to_owned())
    } else if risk.kill_switch_active {
        Some("全局急停已启用".to_owned())
    } else {
        None
    };
    let global_blockers = chain_problem
        .iter()
        .chain(cex_problem.iter())
        .cloned()
        .collect();
    snapshot.execution_readiness = OnchainExecutionReadiness {
        wallet_address_configured,
        chain_submission_ready: chain_problem.is_none(),
        cex_live_mode_ready: cex_problem.is_none(),
        global_blockers,
        directions: Vec::new(),
        observed_at_ms: now_ms,
    };
}

fn direction_readiness(
    context: &DirectionReadinessContext<'_>,
    comparison: &OnchainCexComparison,
) -> Option<OnchainDirectionReadiness> {
    let DirectionReadinessContext {
        state,
        snapshot,
        quotes,
        wallet,
        cex_balances,
        now_ms,
    } = context;
    let config = &snapshot.config;
    let direction = comparison.direction;
    let resolution =
        crate::services::spot::split_spot_pair(&config.cex_symbol).map(|(base, quote)| {
            state
                .instrument_registry()
                .resolve_spot_instrument_evidence(&config.cex_venue, &base, &quote, *now_ms)
        });
    let instrument = resolution
        .as_ref()
        .and_then(|resolution| resolution.instrument.as_ref());
    let cex_instrument = project_cex_instrument(
        config,
        resolution.as_ref(),
        snapshot.quote_conversion.as_ref(),
    );
    let (onchain_asset, onchain_required, base_required) = match direction {
        OnchainComparisonDirection::BuyOnchainSellCex => (
            config.quote_token.as_str(),
            raw_units(&quotes.reverse.input_amount_raw, config.quote_decimals)?,
            raw_units(&quotes.reverse.output_amount_raw, config.base_decimals)?,
        ),
        OnchainComparisonDirection::BuyCexSellOnchain => {
            let base_required = raw_units(&quotes.forward.input_amount_raw, config.base_decimals)?;
            (config.base_token.as_str(), base_required, base_required)
        }
    };
    let chain_balance = wallet_asset_evidence(
        snapshot,
        *wallet,
        direction,
        onchain_asset,
        onchain_required,
        *now_ms,
    );
    let cex_requirement = cex_inventory_requirement(
        direction,
        instrument,
        snapshot.quote_conversion.as_ref(),
        base_required,
        comparison.cex_price,
        config.cex_taker_fee_bps,
    );
    let cex_balance = cex_requirement.map_or_else(
        || unresolved_cex_asset_evidence(snapshot, direction, base_required),
        |requirement| {
            cex_asset_evidence(
                snapshot,
                cex_balances,
                requirement.asset,
                requirement.required,
                *now_ms,
            )
        },
    );
    let gas_balance = wallet_gas_evidence(snapshot, *wallet, *now_ms);
    let inventory = vec![chain_balance, cex_balance, gas_balance];
    let inventory_ready = inventory
        .iter()
        .all(|row| row.status == OnchainInventoryStatus::Ready);
    let allowance_gate = allowance_build_gate(snapshot, direction);
    let allowance_buildable = allowance_gate.is_ok();
    let allowance_problem = allowance_gate.err();
    let market_ready = direction_market_ready(snapshot, comparison);
    let build_ready = market_ready
        && inventory_ready
        && allowance_buildable
        && cex_instrument.ready
        && cex_instrument.problem.is_none();
    let path = path_readiness(
        context,
        PathReadinessRequest {
            snapshot,
            comparison,
            direction,
            inventory: &inventory,
            instrument: &cex_instrument,
            market_ready,
            build_ready,
        },
    );
    let mut blockers = direction_blockers(
        context,
        comparison,
        &cex_instrument,
        &inventory,
        allowance_problem,
    );
    if path.availability == OnchainPathAvailability::TransferUnprofitable {
        push_unique_blocker(
            &mut blockers,
            format!(
                "当前价差无法覆盖补仓搬运成本：搬运费 ${:.4}，搬运后预计净利 ${:.4}",
                path.transfer_cost_usd.unwrap_or_default(),
                path.post_transfer_net_profit_usd.unwrap_or_default()
            ),
        );
    }
    Some(OnchainDirectionReadiness {
        direction,
        path,
        inventory,
        cex_instrument,
        build_ready,
        submit_ready: false,
        blockers,
    })
}

fn path_readiness(
    context: &DirectionReadinessContext<'_>,
    request: PathReadinessRequest<'_>,
) -> OnchainPathReadiness {
    let replenishment = replenishment_evidence(context, request.inventory);
    project_path_readiness(request, replenishment)
}

fn project_path_readiness(
    request: PathReadinessRequest<'_>,
    replenishment: Vec<OnchainTransferEvidence>,
) -> OnchainPathReadiness {
    let PathReadinessRequest {
        snapshot,
        comparison,
        direction,
        inventory,
        instrument,
        market_ready,
        build_ready,
    } = request;
    let legs = path_legs(snapshot, direction);
    let transfer_economics = transfer_economics(snapshot, comparison, &replenishment);
    let kind = if snapshot.quote_conversion.is_some() {
        OnchainPathKind::QuoteConvertedThreeLeg
    } else {
        OnchainPathKind::DirectTwoLeg
    };
    let availability = if matches!(
        snapshot.quality,
        OnchainComparisonQuality::RawCrossQuote | OnchainComparisonQuality::RawCustomPair
    ) {
        OnchainPathAvailability::MonitoringOnly
    } else if matches!(snapshot.quality, OnchainComparisonQuality::MappingInvalid) {
        OnchainPathAvailability::Blocked
    } else if matches!(
        snapshot.quality,
        OnchainComparisonQuality::Pending
            | OnchainComparisonQuality::Stale
            | OnchainComparisonQuality::UpstreamUnavailable
    ) {
        OnchainPathAvailability::EvidencePending
    } else if !instrument.ready || instrument.problem.is_some() {
        OnchainPathAvailability::Blocked
    } else if inventory
        .iter()
        .any(|row| row.status == OnchainInventoryStatus::Insufficient)
    {
        let missing = inventory
            .iter()
            .filter(|row| row.status == OnchainInventoryStatus::Insufficient)
            .count();
        if replenishment.len() == missing
            && replenishment
                .iter()
                .all(|row| row.status == OnchainTransferStatus::Ready)
        {
            match transfer_economics.post_transfer_net_profit_usd {
                Some(profit) if profit > 0.0 => OnchainPathAvailability::Replenishable,
                Some(_) => OnchainPathAvailability::TransferUnprofitable,
                None => OnchainPathAvailability::EvidencePending,
            }
        } else if replenishment.iter().any(|row| {
            matches!(
                row.status,
                OnchainTransferStatus::Refreshing | OnchainTransferStatus::Unknown
            )
        }) {
            OnchainPathAvailability::EvidencePending
        } else {
            OnchainPathAvailability::InventoryRequired
        }
    } else if inventory
        .iter()
        .any(|row| row.status == OnchainInventoryStatus::Unknown)
    {
        OnchainPathAvailability::EvidencePending
    } else if !market_ready {
        OnchainPathAvailability::MonitoringOnly
    } else if !snapshot.execution_readiness.chain_submission_ready
        || !snapshot.execution_readiness.cex_live_mode_ready
    {
        OnchainPathAvailability::SetupRequired
    } else if build_ready {
        OnchainPathAvailability::ReadyToBuild
    } else {
        OnchainPathAvailability::EvidencePending
    };
    let mut summary = legs
        .iter()
        .map(|leg| format!("{}→{} @ {}", leg.from_asset, leg.to_asset, leg.venue))
        .collect::<Vec<_>>()
        .join(" ｜ ");
    if !replenishment.is_empty() {
        let transfers = replenishment
            .iter()
            .map(|row| match row.direction {
                OnchainTransferDirection::WithdrawToChain => {
                    format!("{} {}→链上", row.venue.to_ascii_uppercase(), row.asset)
                }
                OnchainTransferDirection::DepositToCex => {
                    format!("链上 {}→{}", row.asset, row.venue.to_ascii_uppercase())
                }
            })
            .collect::<Vec<_>>()
            .join(" + ");
        summary.push_str(" ｜ 补仓 ");
        summary.push_str(&transfers);
        if let (Some(cost), Some(profit)) = (
            transfer_economics.transfer_cost_usd,
            transfer_economics.post_transfer_net_profit_usd,
        ) {
            summary.push_str(&format!(" ｜ 搬运费 ${cost:.4} · 搬运后 ${profit:.4}"));
        }
    }
    OnchainPathReadiness {
        kind,
        availability,
        legs,
        replenishment,
        transfer_cost_usd: transfer_economics.transfer_cost_usd,
        post_transfer_net_profit_usd: transfer_economics.post_transfer_net_profit_usd,
        summary,
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct TransferEconomics {
    pub(super) transfer_cost_usd: Option<f64>,
    pub(super) post_transfer_net_profit_usd: Option<f64>,
}

pub(super) fn transfer_economics(
    snapshot: &OnchainComparisonSnapshot,
    comparison: &OnchainCexComparison,
    replenishment: &[OnchainTransferEvidence],
) -> TransferEconomics {
    if replenishment.is_empty() {
        return TransferEconomics::default();
    }
    let Some(usd_rate) = super::usd_valuation::rate(
        snapshot.quote_usd_valuation.as_ref(),
        &snapshot.config.quote_token,
        snapshot.config.max_age_ms,
        snapshot.observed_at_ms,
    ) else {
        return TransferEconomics::default();
    };
    let mut cost = 0.0;
    for row in replenishment {
        let Some(fee) = row.fee.filter(|fee| fee.is_finite() && *fee >= 0.0) else {
            return TransferEconomics::default();
        };
        let Some(price) = transfer_asset_quote_price(snapshot, comparison, &row.asset) else {
            return TransferEconomics::default();
        };
        cost += fee * price * usd_rate;
        if row.direction == OnchainTransferDirection::DepositToCex {
            let gas = snapshot.config.gas_usd;
            if !gas.is_finite() || gas < 0.0 {
                return TransferEconomics::default();
            }
            cost += gas;
        }
    }
    let trading_profit = comparison.observable_notional_usd * comparison.net_spread_bps / 10_000.0;
    if !cost.is_finite() || !trading_profit.is_finite() {
        return TransferEconomics::default();
    }
    TransferEconomics {
        transfer_cost_usd: Some(cost),
        post_transfer_net_profit_usd: Some(trading_profit - cost),
    }
}

fn transfer_asset_quote_price(
    snapshot: &OnchainComparisonSnapshot,
    comparison: &OnchainCexComparison,
    asset: &str,
) -> Option<f64> {
    let asset = asset.trim();
    // Comparison prices are already in the configured quote, including CEX conversion.
    // Only that same asset is one quote unit; a stablecoin Base still has a market price.
    if asset.eq_ignore_ascii_case(snapshot.config.quote_token.trim()) {
        return Some(1.0);
    }
    if asset.eq_ignore_ascii_case(&snapshot.config.base_token)
        && comparison.cex_price.is_finite()
        && comparison.cex_price > 0.0
    {
        return Some(comparison.cex_price);
    }
    None
}

fn replenishment_evidence(
    context: &DirectionReadinessContext<'_>,
    inventory: &[OnchainInventoryEvidence],
) -> Vec<OnchainTransferEvidence> {
    inventory
        .iter()
        .filter(|row| row.status == OnchainInventoryStatus::Insufficient)
        .filter_map(|row| replenishment_for_missing(context, row))
        .collect()
}

fn replenishment_for_missing(
    context: &DirectionReadinessContext<'_>,
    missing: &OnchainInventoryEvidence,
) -> Option<OnchainTransferEvidence> {
    let config = &context.snapshot.config;
    let contract = configured_contract(config, &missing.asset)?;
    let amount = inventory_shortfall(missing)?;
    let asset_decimals = if missing.asset.eq_ignore_ascii_case(&config.base_token) {
        config.base_decimals
    } else {
        config.quote_decimals
    };
    let (direction, available) = match missing.location {
        OnchainInventoryLocation::Onchain => {
            let source = cex_asset_evidence(
                context.snapshot,
                context.cex_balances,
                &missing.asset,
                amount.to_f64()?,
                context.now_ms,
            );
            (OnchainTransferDirection::WithdrawToChain, source.available?)
        }
        OnchainInventoryLocation::Cex => {
            let available = wallet_available_asset(
                context.snapshot,
                context.wallet,
                &missing.asset,
                context.now_ms,
            )?;
            (OnchainTransferDirection::DepositToCex, available)
        }
    };
    let mut evidence = context
        .state
        .instrument_registry()
        .onchain_transfer_evidence(
            &config.cex_venue,
            &missing.asset,
            &config.chain,
            Some(contract),
            direction,
            amount,
            asset_decimals,
            context.now_ms,
        );
    check_replenishment_source_balance(&mut evidence, available);
    Some(evidence)
}

fn inventory_shortfall(missing: &OnchainInventoryEvidence) -> Option<Decimal> {
    if missing.status != OnchainInventoryStatus::Insufficient {
        return None;
    }
    let required = Decimal::from_f64(missing.required)?;
    let available = Decimal::from_f64(missing.available?)?;
    if available < Decimal::ZERO {
        return None;
    }
    let amount = required.checked_sub(available)?;
    (amount > Decimal::ZERO).then_some(amount)
}

fn check_replenishment_source_balance(evidence: &mut OnchainTransferEvidence, available: f64) {
    if evidence.status != OnchainTransferStatus::Ready {
        return;
    }
    let debit = evidence.amount_exact.as_deref().and_then(|amount| {
        let amount = amount.parse::<Decimal>().ok()?;
        match evidence.direction {
            OnchainTransferDirection::DepositToCex => Some(amount),
            OnchainTransferDirection::WithdrawToChain => {
                amount.checked_add(evidence.fee_exact.as_deref()?.parse::<Decimal>().ok()?)
            }
        }
    });
    let available = Decimal::from_f64(available).filter(|value| *value >= Decimal::ZERO);
    match (available, debit) {
        (Some(available), Some(debit)) if available < debit => {
            evidence.status = OnchainTransferStatus::Blocked;
            evidence.problem = Some(format!(
                "{} 来源可用 {}，低于按最小数量和精度编译后的最高扣款 {}（含提币费）",
                evidence.asset,
                available.normalize(),
                debit.normalize()
            ));
        }
        (Some(_), Some(_)) => {}
        _ => {
            evidence.status = OnchainTransferStatus::Unknown;
            evidence.problem = Some("补仓来源余额或精确扣款数量尚未核实".to_owned());
        }
    }
}

fn configured_contract<'a>(
    config: &'a shared_types::OnchainComparisonConfig,
    asset: &str,
) -> Option<&'a str> {
    let contract = if asset.eq_ignore_ascii_case(&config.base_token) {
        Some(config.base_mint.as_str())
    } else if asset.eq_ignore_ascii_case(&config.quote_token) {
        Some(config.quote_mint.as_str())
    } else {
        None
    };
    contract
        .map(str::trim)
        .filter(|contract| !contract.is_empty())
}

fn wallet_available_asset(
    snapshot: &OnchainComparisonSnapshot,
    wallet: Option<&OnchainWalletInventory>,
    asset: &str,
    now_ms: i64,
) -> Option<f64> {
    let wallet = wallet.filter(|wallet| {
        wallet.matches_config(&snapshot.config)
            && now_ms >= wallet.observed_at_ms
            && now_ms.saturating_sub(wallet.observed_at_ms) <= WALLET_INVENTORY_MAX_AGE_MS
    })?;
    if asset.eq_ignore_ascii_case(&snapshot.config.base_token) {
        wallet.base.available
    } else if asset.eq_ignore_ascii_case(&snapshot.config.quote_token) {
        wallet.quote.available
    } else {
        None
    }
}

fn path_legs(
    snapshot: &OnchainComparisonSnapshot,
    direction: OnchainComparisonDirection,
) -> Vec<OnchainPathLeg> {
    let config = &snapshot.config;
    let base = config.base_token.trim().to_ascii_uppercase();
    let onchain_quote = config.quote_token.trim().to_ascii_uppercase();
    let cex_quote = snapshot
        .quote_conversion
        .as_ref()
        .map(|evidence| evidence.cex_quote.trim().to_ascii_uppercase())
        .or_else(|| {
            onchain_cex_quote_token(&config.cex_symbol).map(|quote| quote.to_ascii_uppercase())
        })
        .unwrap_or_else(|| "QUOTE?".to_owned());
    let chain = config.chain.trim().to_ascii_uppercase();
    let venue = config.cex_venue.trim().to_ascii_uppercase();
    let chain_leg = |from_asset: &str, to_asset: &str| OnchainPathLeg {
        kind: OnchainPathLegKind::OnchainSwap,
        venue: chain.clone(),
        from_asset: from_asset.to_owned(),
        to_asset: to_asset.to_owned(),
    };
    let cex_leg = |kind, from_asset: &str, to_asset: &str| OnchainPathLeg {
        kind,
        venue: venue.clone(),
        from_asset: from_asset.to_owned(),
        to_asset: to_asset.to_owned(),
    };
    match (direction, snapshot.quote_conversion.is_some()) {
        (OnchainComparisonDirection::BuyOnchainSellCex, false) => vec![
            chain_leg(&onchain_quote, &base),
            cex_leg(OnchainPathLegKind::CexSpot, &base, &cex_quote),
        ],
        (OnchainComparisonDirection::BuyCexSellOnchain, false) => vec![
            cex_leg(OnchainPathLegKind::CexSpot, &cex_quote, &base),
            chain_leg(&base, &onchain_quote),
        ],
        (OnchainComparisonDirection::BuyOnchainSellCex, true) => vec![
            chain_leg(&onchain_quote, &base),
            cex_leg(OnchainPathLegKind::CexSpot, &base, &cex_quote),
            cex_leg(
                OnchainPathLegKind::CexQuoteConversion,
                &cex_quote,
                &onchain_quote,
            ),
        ],
        (OnchainComparisonDirection::BuyCexSellOnchain, true) => vec![
            cex_leg(
                OnchainPathLegKind::CexQuoteConversion,
                &onchain_quote,
                &cex_quote,
            ),
            cex_leg(OnchainPathLegKind::CexSpot, &cex_quote, &base),
            chain_leg(&base, &onchain_quote),
        ],
    }
}

fn direction_blockers(
    context: &DirectionReadinessContext<'_>,
    comparison: &OnchainCexComparison,
    cex_instrument: &OnchainCexInstrumentEvidence,
    inventory: &[OnchainInventoryEvidence],
    allowance_problem: Option<String>,
) -> Vec<String> {
    let config = &context.snapshot.config;
    let mut blockers = Vec::new();
    if !cex_instrument.ready {
        push_unique_blocker(
            &mut blockers,
            cex_instrument.problem.clone().unwrap_or_else(|| {
                format!(
                    "{} {} 现货执行规格尚未核验",
                    config.cex_venue.to_uppercase(),
                    config.cex_symbol
                )
            }),
        );
    } else if let Some(problem) = cex_instrument.problem.clone() {
        push_unique_blocker(&mut blockers, problem);
    }
    for problem in inventory.iter().filter_map(inventory_blocker) {
        push_unique_blocker(&mut blockers, problem);
    }
    if let Some(problem) = direction_market_problem(context.snapshot, comparison) {
        push_unique_blocker(&mut blockers, problem);
    }
    if let Some(problem) = allowance_problem {
        push_unique_blocker(&mut blockers, problem);
    }
    if let Err(problem) = super::execution_submit::readiness(context.state, config) {
        push_unique_blocker(&mut blockers, problem);
    }
    push_unique_blocker(
        &mut blockers,
        format!(
            "{} 现货下单权限与私有终态需要在提交前重新核验",
            config.cex_venue.to_uppercase()
        ),
    );
    blockers
}

fn direction_market_ready(
    snapshot: &OnchainComparisonSnapshot,
    comparison: &OnchainCexComparison,
) -> bool {
    quality_allows_depth_probe(snapshot.quality)
        && comparison.net_spread_bps > 0.0
        && comparison.net_spread_bps >= snapshot.config.spread_alert.min_net_spread_bps.max(0.0)
}

fn quality_allows_depth_probe(quality: OnchainComparisonQuality) -> bool {
    matches!(
        quality,
        OnchainComparisonQuality::Fresh | OnchainComparisonQuality::LowLiquidity
    )
}

fn direction_market_problem(
    snapshot: &OnchainComparisonSnapshot,
    comparison: &OnchainCexComparison,
) -> Option<String> {
    if snapshot.quality == OnchainComparisonQuality::Stale {
        return Some("链上报价或 CEX WS 最优价已过期".to_owned());
    }
    if snapshot.quality == OnchainComparisonQuality::RawCrossQuote {
        return Some(if snapshot.config.quote_identity_resolved {
            format!(
                "链上 Quote {} 与 CEX Quote {} 不同且没有汇率换算；只能观察原始价格，不能判断净收益或构建交易计划",
                snapshot.config.quote_token,
                onchain_cex_quote_token(&snapshot.config.cex_symbol).unwrap_or("未知")
            )
        } else {
            format!(
                "链上 Quote {} 的符号身份尚未核验；只能观察原始价格，不能判断净收益或构建交易计划",
                snapshot.config.quote_token
            )
        });
    }
    if snapshot.quality == OnchainComparisonQuality::RawCustomPair {
        return Some(if snapshot.config.base_identity_resolved {
            format!(
                "链上 Base {} 与 CEX Base {} 是不同资产；只能观察两个市场的原始价格，不能判断净收益或构建交易计划",
                snapshot.config.base_token,
                onchain_cex_base_token(&snapshot.config.cex_symbol).unwrap_or("未知")
            )
        } else {
            format!(
                "链上 Base {} 的符号身份尚未核验；只能观察原始价格，不能判断净收益或构建交易计划",
                snapshot.config.base_token
            )
        });
    }
    let minimum_bps = snapshot.config.spread_alert.min_net_spread_bps.max(0.0);
    if comparison.net_spread_bps <= 0.0 {
        return Some(format!(
            "本方向费后净差 {:.4}% 无法覆盖手续费、滑点与 Gas",
            comparison.net_spread_bps / 100.0
        ));
    }
    if comparison.net_spread_bps < minimum_bps {
        return Some(format!(
            "本方向费后净差 {:.4}% 低于配置门槛 {:.4}%",
            comparison.net_spread_bps / 100.0,
            minimum_bps / 100.0
        ));
    }
    (!quality_allows_depth_probe(snapshot.quality))
        .then(|| "当前双源报价尚未达到可构建状态".to_owned())
}

fn push_unique_blocker(blockers: &mut Vec<String>, problem: String) {
    if !blockers.iter().any(|row| row == &problem) {
        blockers.push(problem);
    }
}

fn cex_inventory_requirement<'a>(
    direction: OnchainComparisonDirection,
    instrument: Option<&'a VenueInstrument>,
    conversion: Option<&'a shared_types::OnchainQuoteConversionEvidence>,
    base_required: f64,
    normalized_cex_price: f64,
    fee_bps: f64,
) -> Option<CexInventoryRequirement<'a>> {
    let instrument = instrument?;
    match direction {
        OnchainComparisonDirection::BuyOnchainSellCex => Some(CexInventoryRequirement {
            asset: instrument.canonical_symbol.as_str(),
            required: base_required,
        }),
        OnchainComparisonDirection::BuyCexSellOnchain => {
            let fee_rate = (fee_bps.max(0.0) / 10_000.0).clamp(0.0, 0.25);
            let conversion_multiplier = if conversion.is_some() {
                1.0 / (1.0 - fee_rate)
            } else {
                1.0
            };
            Some(CexInventoryRequirement {
                asset: conversion
                    .map(|evidence| evidence.onchain_quote.as_str())
                    .or(instrument.quote_asset.as_deref())?,
                required: base_required
                    * normalized_cex_price
                    * (1.0 + fee_rate)
                    * conversion_multiplier,
            })
        }
    }
}

fn project_cex_instrument(
    config: &shared_types::OnchainComparisonConfig,
    resolution: Option<&SpotInstrumentResolution>,
    conversion: Option<&shared_types::OnchainQuoteConversionEvidence>,
) -> OnchainCexInstrumentEvidence {
    let Some(resolution) = resolution else {
        return OnchainCexInstrumentEvidence {
            venue: config.cex_venue.clone(),
            requested_symbol: config.cex_symbol.clone(),
            native_symbol: None,
            status: OnchainCexInstrumentStatus::Unavailable,
            ready: false,
            source: "instrument_registry".to_owned(),
            observed_at_ms: None,
            problem: Some("CEX 交易对不是明确的 Base/Quote 格式".to_owned()),
        };
    };
    let instrument = resolution.instrument.as_ref();
    let validation =
        instrument.map(|instrument| validate_cex_instrument(config, instrument, conversion));
    let status = project_instrument_status(resolution.status, validation.as_ref());
    let ready = status == OnchainCexInstrumentStatus::Ready
        && validation.as_ref().is_some_and(|(ready, _)| *ready);
    let problem = resolution_problem(config, resolution, validation);
    OnchainCexInstrumentEvidence {
        venue: config.cex_venue.clone(),
        requested_symbol: config.cex_symbol.clone(),
        native_symbol: instrument.map(|instrument| instrument.native_symbol.clone()),
        status,
        ready,
        source: instrument
            .and_then(|instrument| instrument.source_url.clone())
            .unwrap_or_else(|| "instrument_registry".to_owned()),
        observed_at_ms: instrument
            .map(|instrument| instrument.checked_at_ms)
            .or(resolution.checked_at_ms),
        problem,
    }
}

fn project_instrument_status(
    status: SpotInstrumentResolutionStatus,
    validation: Option<&(bool, Option<String>)>,
) -> OnchainCexInstrumentStatus {
    match status {
        SpotInstrumentResolutionStatus::Ready if validation.is_some_and(|(ready, _)| *ready) => {
            OnchainCexInstrumentStatus::Ready
        }
        SpotInstrumentResolutionStatus::Ready | SpotInstrumentResolutionStatus::Incomplete => {
            OnchainCexInstrumentStatus::Incomplete
        }
        SpotInstrumentResolutionStatus::Syncing => OnchainCexInstrumentStatus::Syncing,
        SpotInstrumentResolutionStatus::Unlisted => OnchainCexInstrumentStatus::Unlisted,
        SpotInstrumentResolutionStatus::Stale => OnchainCexInstrumentStatus::Stale,
        SpotInstrumentResolutionStatus::Unavailable => OnchainCexInstrumentStatus::Unavailable,
        SpotInstrumentResolutionStatus::Unsupported => OnchainCexInstrumentStatus::Unsupported,
    }
}

fn resolution_problem(
    config: &shared_types::OnchainComparisonConfig,
    resolution: &SpotInstrumentResolution,
    validation: Option<(bool, Option<String>)>,
) -> Option<String> {
    let venue = config.cex_venue.to_uppercase();
    let pair = &config.cex_symbol;
    match resolution.status {
        SpotInstrumentResolutionStatus::Ready | SpotInstrumentResolutionStatus::Incomplete => {
            validation.and_then(|(_, problem)| problem)
        }
        SpotInstrumentResolutionStatus::Syncing => Some(format!(
            "{venue} 官方 Spot 规格正在首次同步；完成后自动核验 {pair}"
        )),
        SpotInstrumentResolutionStatus::Unlisted => Some(format!(
            "{venue} 最新官方 Spot 目录没有 {pair} 精确交易对；请改选已挂牌市场"
        )),
        SpotInstrumentResolutionStatus::Stale => Some(format!(
            "{venue} {pair} 官方 Spot 规格已过期；系统正在刷新，期间禁止构建"
        )),
        SpotInstrumentResolutionStatus::Unavailable => Some(format!(
            "{venue} 官方 Spot 规格刷新失败；系统会自动重试：{}",
            resolution
                .problem
                .as_ref()
                .map_or("上游暂不可用", |problem| problem.message.as_str())
        )),
        SpotInstrumentResolutionStatus::Unsupported => Some(format!(
            "{venue} 尚未接入可核验的官方 Spot 规格：{}",
            resolution
                .problem
                .as_ref()
                .map_or("adapter 未提供 instrument registry", |problem| problem
                    .message
                    .as_str())
        )),
    }
}

fn validate_cex_instrument(
    config: &shared_types::OnchainComparisonConfig,
    instrument: &VenueInstrument,
    conversion: Option<&shared_types::OnchainQuoteConversionEvidence>,
) -> (bool, Option<String>) {
    if let Some(problem) = instrument_identity_problem(config, instrument)
        .or_else(|| instrument_spec_problem(config, instrument))
    {
        return (false, Some(problem));
    }
    let instrument_quote = instrument.quote_asset.as_deref();
    let direct_quote_match =
        instrument_quote.is_some_and(|quote| quote.eq_ignore_ascii_case(&config.quote_token));
    let conversion_match = conversion.is_some_and(|evidence| {
        evidence.venue.eq_ignore_ascii_case(&config.cex_venue)
            && instrument_quote.is_some_and(|quote| quote.eq_ignore_ascii_case(&evidence.cex_quote))
            && evidence
                .onchain_quote
                .eq_ignore_ascii_case(&config.quote_token)
    });
    if !direct_quote_match && !conversion_match {
        return (
            true,
            Some(format!(
                "{} {} 使用 {}，链上使用 {}；未提供两者汇率证据，只能观察原始价格，不能判断净收益或执行",
                config.cex_venue.to_uppercase(),
                instrument.native_symbol,
                instrument.quote_asset.as_deref().unwrap_or("未知 Quote"),
                config.quote_token
            )),
        );
    }
    (true, None)
}

fn instrument_identity_problem(
    config: &shared_types::OnchainComparisonConfig,
    instrument: &VenueInstrument,
) -> Option<String> {
    if !instrument
        .product_type
        .as_deref()
        .is_some_and(|product| product.eq_ignore_ascii_case("spot"))
    {
        return Some(format!(
            "{} {} 不是 Spot instrument，禁止误送到现货下单通道",
            config.cex_venue.to_uppercase(),
            instrument.native_symbol
        ));
    } else if instrument.asset_class != InstrumentAssetClass::Crypto {
        return Some(format!(
            "{} {} 官方资产类型不是加密资产，禁止与链上代币配对",
            config.cex_venue.to_uppercase(),
            instrument.native_symbol
        ));
    } else if !instrument
        .canonical_symbol
        .eq_ignore_ascii_case(&config.base_token)
    {
        return Some(format!(
            "{} {} 的经济标的 {} 与链上 {} 不一致",
            config.cex_venue.to_uppercase(),
            instrument.native_symbol,
            instrument.canonical_symbol,
            config.base_token
        ));
    }
    None
}

fn instrument_spec_problem(
    config: &shared_types::OnchainComparisonConfig,
    instrument: &VenueInstrument,
) -> Option<String> {
    if instrument.listing_status != shared_types::InstrumentListingStatus::Trading {
        return Some(format!(
            "{} {} 官方挂牌状态不是 Trading，当前不可下单",
            config.cex_venue.to_uppercase(),
            instrument.native_symbol
        ));
    }
    if !instrument.has_official_provenance() {
        return Some(format!(
            "{} {} 缺少官方 endpoint 或 schema 证据",
            config.cex_venue.to_uppercase(),
            instrument.native_symbol
        ));
    }
    if !instrument.execution_supported {
        return Some(format!(
            "{} {} 官方挂牌但当前请求编译器不支持执行",
            config.cex_venue.to_uppercase(),
            instrument.native_symbol
        ));
    }
    if instrument
        .quote_asset
        .as_deref()
        .is_none_or(|quote| quote.trim().is_empty())
    {
        return Some(format!(
            "{} {} 官方 Spot 规格缺少 Quote 资产",
            config.cex_venue.to_uppercase(),
            instrument.native_symbol
        ));
    }
    if instrument.price_tick.is_none()
        || instrument.qty_step.is_none()
        || (instrument.min_notional.is_none() && instrument.min_qty.is_none())
    {
        let mut missing = Vec::new();
        if instrument.price_tick.is_none() {
            missing.push("价格精度");
        }
        if instrument.qty_step.is_none() {
            missing.push("数量精度");
        }
        if instrument.min_notional.is_none() && instrument.min_qty.is_none() {
            missing.push("最小下单限制");
        }
        return Some(format!(
            "{} {} 官方已挂牌，但执行规格缺少：{}",
            config.cex_venue.to_uppercase(),
            instrument.native_symbol,
            missing.join("、")
        ));
    }
    None
}

fn wallet_asset_evidence(
    snapshot: &OnchainComparisonSnapshot,
    wallet: Option<&OnchainWalletInventory>,
    direction: OnchainComparisonDirection,
    asset: &str,
    required: f64,
    now_ms: i64,
) -> OnchainInventoryEvidence {
    let selected = match direction {
        OnchainComparisonDirection::BuyOnchainSellCex => wallet.map(|row| &row.quote),
        OnchainComparisonDirection::BuyCexSellOnchain => wallet.map(|row| &row.base),
    };
    wallet_evidence(WalletEvidenceRequest {
        snapshot,
        wallet,
        selected,
        asset,
        required,
        gas: false,
        now_ms,
    })
}

fn wallet_gas_evidence(
    snapshot: &OnchainComparisonSnapshot,
    wallet: Option<&OnchainWalletInventory>,
    now_ms: i64,
) -> OnchainInventoryEvidence {
    let asset = if snapshot.config.chain.eq_ignore_ascii_case("solana") {
        "SOL Gas"
    } else {
        "Native Gas"
    };
    wallet_evidence(WalletEvidenceRequest {
        snapshot,
        wallet,
        selected: wallet.map(|row| &row.gas),
        asset,
        required: 0.0,
        gas: true,
        now_ms,
    })
}

fn wallet_evidence(request: WalletEvidenceRequest<'_>) -> OnchainInventoryEvidence {
    let WalletEvidenceRequest {
        snapshot,
        wallet,
        selected,
        asset,
        required,
        gas,
        now_ms,
    } = request;
    let config = &snapshot.config;
    let (available, source, observed_at_ms, problem) = if config.wallet_address.trim().is_empty() {
        (
            None,
            "wallet_config".to_owned(),
            None,
            Some("未配置链上钱包地址".to_owned()),
        )
    } else if config.rpc.mode == shared_types::OnchainRpcMode::Custom && !snapshot.rpc_status.ready
    {
        (
            None,
            "wallet_rpc".to_owned(),
            snapshot.rpc_status.observed_at_ms,
            Some(
                snapshot
                    .rpc_status
                    .problem
                    .clone()
                    .unwrap_or_else(|| "自定义 RPC 尚未通过网络核验".to_owned()),
            ),
        )
    } else if wallet.is_none_or(|row| !row.matches_config(config)) {
        (
            None,
            "wallet_inventory".to_owned(),
            None,
            Some("等待与当前钱包、链和代币身份一致的余额快照".to_owned()),
        )
    } else if wallet
        .is_some_and(|row| now_ms.saturating_sub(row.observed_at_ms) > WALLET_INVENTORY_MAX_AGE_MS)
    {
        (
            None,
            "wallet_inventory".to_owned(),
            wallet.map(|row| row.observed_at_ms),
            Some("链上钱包余额快照已过期".to_owned()),
        )
    } else {
        selected.map_or_else(
            || {
                (
                    None,
                    "wallet_inventory".to_owned(),
                    wallet.map(|row| row.observed_at_ms),
                    Some("链上资产余额证据缺失".to_owned()),
                )
            },
            |row| {
                (
                    row.available,
                    row.source.clone(),
                    wallet.map(|row| row.observed_at_ms),
                    row.problem.clone(),
                )
            },
        )
    };
    let status = inventory_status(available, required, gas, problem.as_deref());
    OnchainInventoryEvidence {
        location: OnchainInventoryLocation::Onchain,
        scope: format!("{}:wallet", config.chain),
        asset: asset.to_owned(),
        required,
        available,
        status,
        source,
        observed_at_ms,
        problem,
    }
}

fn cex_asset_evidence(
    snapshot: &OnchainComparisonSnapshot,
    balances: &VenueBalanceEnvelope,
    asset: &str,
    required: f64,
    now_ms: i64,
) -> OnchainInventoryEvidence {
    let venue = &snapshot.config.cex_venue;
    let age_ms = now_ms.saturating_sub(balances.observed_at_ms);
    let fresh = now_ms >= balances.observed_at_ms && age_ms <= CEX_BALANCE_MAX_AGE_MS;
    let available = fresh
        .then(|| {
            balances
                .rows
                .iter()
                .filter(|row| spot_balance_venue_matches(&row.venue, venue))
                .filter(|row| row.currency.eq_ignore_ascii_case(asset))
                .map(|row| row.available)
                .filter(|value| value.is_finite() && *value >= 0.0)
                .sum::<f64>()
        })
        .filter(|_| {
            balances.rows.iter().any(|row| {
                spot_balance_venue_matches(&row.venue, venue)
                    && row.currency.eq_ignore_ascii_case(asset)
            })
        });
    let problem = if !fresh {
        Some(format!(
            "{} 账户余额快照已过期或时间无效",
            venue.to_uppercase()
        ))
    } else if available.is_none() {
        Some(format!(
            "{} 现货账户尚无 {asset} 可用余额证据",
            venue.to_uppercase()
        ))
    } else {
        None
    };
    OnchainInventoryEvidence {
        location: OnchainInventoryLocation::Cex,
        scope: format!("{}:spot", venue.to_ascii_lowercase()),
        asset: asset.to_owned(),
        required,
        available,
        status: inventory_status(available, required, false, problem.as_deref()),
        source: balances.source.clone(),
        observed_at_ms: Some(balances.observed_at_ms),
        problem,
    }
}

fn unresolved_cex_asset_evidence(
    snapshot: &OnchainComparisonSnapshot,
    direction: OnchainComparisonDirection,
    required: f64,
) -> OnchainInventoryEvidence {
    let venue = &snapshot.config.cex_venue;
    let asset_role = match direction {
        OnchainComparisonDirection::BuyOnchainSellCex => "Base",
        OnchainComparisonDirection::BuyCexSellOnchain => "Quote",
    };
    let problem = format!(
        "{} 官方现货规格尚未提供 {asset_role} 资产，无法核验执行余额",
        venue.to_uppercase()
    );
    OnchainInventoryEvidence {
        location: OnchainInventoryLocation::Cex,
        scope: format!("{}:spot", venue.to_ascii_lowercase()),
        asset: format!("{asset_role} 待核验"),
        required,
        available: None,
        status: OnchainInventoryStatus::Unknown,
        source: "instrument_registry".to_owned(),
        observed_at_ms: None,
        problem: Some(problem),
    }
}

fn spot_balance_venue_matches(row_venue: &str, configured_venue: &str) -> bool {
    row_venue.eq_ignore_ascii_case(configured_venue)
        || row_venue.rsplit_once(':').is_some_and(|(family, product)| {
            family.eq_ignore_ascii_case(configured_venue) && product.eq_ignore_ascii_case("spot")
        })
}

fn inventory_status(
    available: Option<f64>,
    required: f64,
    gas: bool,
    problem: Option<&str>,
) -> OnchainInventoryStatus {
    if problem.is_some() {
        return OnchainInventoryStatus::Unknown;
    }
    match available {
        Some(value) if gas && value > 0.0 => OnchainInventoryStatus::Ready,
        Some(_) if gas => OnchainInventoryStatus::Insufficient,
        Some(value) if value + f64::EPSILON >= required => OnchainInventoryStatus::Ready,
        Some(_) => OnchainInventoryStatus::Insufficient,
        None => OnchainInventoryStatus::Unknown,
    }
}

fn inventory_blocker(row: &OnchainInventoryEvidence) -> Option<String> {
    match row.status {
        OnchainInventoryStatus::Ready => None,
        OnchainInventoryStatus::Insufficient => Some(format!(
            "{} {} 余额不足：需要 {:.8}，可用 {:.8}",
            row.scope,
            row.asset,
            row.required,
            row.available.unwrap_or_default()
        )),
        OnchainInventoryStatus::Unknown => Some(
            row.problem
                .clone()
                .unwrap_or_else(|| format!("{} {} 余额尚未核验", row.scope, row.asset)),
        ),
    }
}

fn allowance_build_gate(
    snapshot: &OnchainComparisonSnapshot,
    direction: OnchainComparisonDirection,
) -> Result<AllowanceBuildGate, String> {
    if snapshot.config.chain.eq_ignore_ascii_case("solana") {
        return Ok(AllowanceBuildGate::Proven);
    }
    let input_token = match direction {
        OnchainComparisonDirection::BuyOnchainSellCex => &snapshot.config.quote_mint,
        OnchainComparisonDirection::BuyCexSellOnchain => &snapshot.config.base_mint,
    };
    if input_token.eq_ignore_ascii_case(EVM_NATIVE_TOKEN_ADDRESS) {
        return Ok(AllowanceBuildGate::Proven);
    }
    match snapshot.config.provider.as_str() {
        // 0x v2 returns the authoritative allowance target in the firm quote issues object.
        "zeroex_swap_v2" => Ok(AllowanceBuildGate::FirmQuoteCheck),
        "okx_dex_v6" => Ok(AllowanceBuildGate::FirmQuoteCheck),
        "cow_protocol" => Err(
            "CoW Protocol 的 ERC-20 授权与 EIP-712 订单终态尚未接入，禁止构建双腿计划".to_owned(),
        ),
        provider => Err(format!(
            "Provider {provider} 尚不能核验 EVM ERC-20 allowance，禁止构建双腿计划"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{InstrumentListingStatus, InstrumentMetadataSource};

    #[test]
    fn exact_spot_scope_accepts_kraken_spot_but_not_futures() {
        assert!(spot_balance_venue_matches("kraken:spot", "kraken"));
        assert!(spot_balance_venue_matches("binance", "binance"));
        assert!(!spot_balance_venue_matches("kraken:futures", "kraken"));
    }

    #[test]
    fn proven_zero_is_insufficient_not_unknown() {
        assert_eq!(
            inventory_status(Some(0.0), 1.0, false, None),
            OnchainInventoryStatus::Insufficient
        );
        assert_eq!(
            inventory_status(None, 1.0, false, None),
            OnchainInventoryStatus::Unknown
        );
    }

    #[test]
    fn managed_rpc_waits_for_wallet_snapshot_without_demanding_custom_rpc() {
        let mut snapshot = OnchainComparisonSnapshot::default();
        snapshot.config.wallet_address = "public-wallet".to_owned();
        snapshot.config.rpc.mode = shared_types::OnchainRpcMode::ProviderManaged;
        let evidence = wallet_evidence(WalletEvidenceRequest {
            snapshot: &snapshot,
            wallet: None,
            selected: None,
            asset: "USDC",
            required: 100.0,
            gas: false,
            now_ms: 1,
        });

        assert_eq!(evidence.source, "wallet_inventory");
        assert!(evidence
            .problem
            .is_some_and(|problem| problem.contains("等待与当前钱包")));
    }

    #[test]
    fn direction_gate_uses_profit_threshold_and_defers_depth_to_build() {
        let mut snapshot = OnchainComparisonSnapshot::default();
        snapshot.quality = OnchainComparisonQuality::Fresh;
        snapshot.config.spread_alert.min_net_spread_bps = 20.0;
        snapshot.config.min_liquidity_usd = 100.0;
        let mut comparison = OnchainCexComparison {
            direction: OnchainComparisonDirection::BuyOnchainSellCex,
            onchain_price: 100.0,
            cex_price: 101.0,
            gross_spread_bps: 40.0,
            cex_fee_bps: 10.0,
            quote_conversion_fee_bps: 0.0,
            slippage_bps: 5.0,
            gas_usd: 0.5,
            gas_bps: 5.0,
            total_cost_bps: 20.0,
            net_spread_bps: 19.99,
            observable_notional_usd: 100.0,
            executable: false,
        };

        assert!(!direction_market_ready(&snapshot, &comparison));
        assert!(direction_market_problem(&snapshot, &comparison)
            .is_some_and(|problem| problem.contains("低于配置门槛 0.2000%")));

        comparison.net_spread_bps = 20.0;
        assert!(direction_market_ready(&snapshot, &comparison));
        comparison.observable_notional_usd = 99.99;
        snapshot.quality = OnchainComparisonQuality::LowLiquidity;
        assert!(direction_market_ready(&snapshot, &comparison));
        assert_eq!(direction_market_problem(&snapshot, &comparison), None);

        snapshot.quality = OnchainComparisonQuality::UpstreamUnavailable;
        assert!(!direction_market_ready(&snapshot, &comparison));
        assert!(direction_market_problem(&snapshot, &comparison)
            .is_some_and(|problem| problem.contains("尚未达到可构建状态")));
    }

    #[test]
    fn raw_observation_has_an_explicit_execution_blocker() {
        let mut snapshot = OnchainComparisonSnapshot::default();
        let comparison = OnchainCexComparison {
            direction: OnchainComparisonDirection::BuyOnchainSellCex,
            onchain_price: 1.0,
            cex_price: 1.0,
            gross_spread_bps: 0.0,
            cex_fee_bps: 0.0,
            quote_conversion_fee_bps: 0.0,
            slippage_bps: 0.0,
            gas_usd: 0.0,
            gas_bps: 0.0,
            total_cost_bps: 0.0,
            net_spread_bps: 0.0,
            observable_notional_usd: 100.0,
            executable: false,
        };

        snapshot.quality = OnchainComparisonQuality::RawCrossQuote;
        snapshot.config.cex_symbol = "SOL/USD".to_owned();
        assert!(direction_market_problem(&snapshot, &comparison)
            .is_some_and(|problem| problem.contains("没有汇率换算")));

        snapshot.quality = OnchainComparisonQuality::RawCustomPair;
        snapshot.config.cex_symbol = "ETH/USDC".to_owned();
        assert!(direction_market_problem(&snapshot, &comparison)
            .is_some_and(|problem| problem.contains("不同资产")));
    }

    #[test]
    fn zeroex_erc20_allowance_is_checked_by_firm_quote_without_blocking_build() {
        let mut snapshot = OnchainComparisonSnapshot::default();
        snapshot.config.chain = "ethereum".to_owned();
        snapshot.config.provider = "zeroex_swap_v2".to_owned();
        snapshot.config.quote_mint = "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".to_owned();

        assert_eq!(
            allowance_build_gate(&snapshot, OnchainComparisonDirection::BuyOnchainSellCex,),
            Ok(AllowanceBuildGate::FirmQuoteCheck)
        );

        snapshot.config.provider = "okx_dex_v6".to_owned();
        assert_eq!(
            allowance_build_gate(&snapshot, OnchainComparisonDirection::BuyOnchainSellCex,),
            Ok(AllowanceBuildGate::FirmQuoteCheck)
        );

        snapshot.config.base_mint = EVM_NATIVE_TOKEN_ADDRESS.to_owned();
        assert_eq!(
            allowance_build_gate(&snapshot, OnchainComparisonDirection::BuyCexSellOnchain),
            Ok(AllowanceBuildGate::Proven)
        );
    }

    #[test]
    fn duplicate_direction_blockers_are_collapsed() {
        let mut blockers = Vec::new();
        push_unique_blocker(&mut blockers, "未配置链上钱包地址".to_owned());
        push_unique_blocker(&mut blockers, "未配置链上钱包地址".to_owned());

        assert_eq!(blockers, vec!["未配置链上钱包地址"]);
    }

    #[test]
    fn cex_instrument_must_be_official_spot_crypto_with_exact_identity() {
        let config = shared_types::OnchainComparisonConfig::default();
        let mut instrument = spot_instrument();

        let resolution = spot_resolution(instrument.clone());
        let ready = project_cex_instrument(&config, Some(&resolution), None);
        assert!(ready.ready);
        assert_eq!(ready.status, OnchainCexInstrumentStatus::Ready);
        assert_eq!(ready.native_symbol.as_deref(), Some("SOLUSDC"));

        instrument.quote_asset = Some("USD".to_owned());
        let resolution = spot_resolution(instrument.clone());
        let raw_quote_comparison = project_cex_instrument(&config, Some(&resolution), None);
        assert!(raw_quote_comparison.ready);
        assert!(raw_quote_comparison
            .problem
            .as_deref()
            .is_some_and(|problem| problem.contains("未提供两者汇率证据")));

        let conversion = shared_types::OnchainQuoteConversionEvidence {
            venue: config.cex_venue.clone(),
            symbol: "USDC/USD".to_owned(),
            source: "ws_push".to_owned(),
            cex_quote: "USD".to_owned(),
            onchain_quote: config.quote_token.clone(),
            source_bid: 0.9997,
            source_ask: 0.9998,
            cex_to_onchain_bid: 1.0002,
            cex_to_onchain_ask: 1.0003,
            cex_to_onchain_capacity: 10_000.0,
            onchain_to_cex_capacity: 10_000.0,
            freshness_ms: 10,
            observed_at_ms: 990,
        };
        let converted = project_cex_instrument(&config, Some(&resolution), Some(&conversion));
        assert!(converted.ready);
        assert!(converted.problem.is_none());

        instrument.execution_supported = false;
        let resolution = spot_resolution(instrument.clone());
        let unsupported = project_cex_instrument(&config, Some(&resolution), None);
        assert!(!unsupported.ready);
        assert_eq!(unsupported.status, OnchainCexInstrumentStatus::Incomplete);
        assert!(unsupported
            .problem
            .as_deref()
            .is_some_and(|problem| problem.contains("请求编译器不支持")));

        instrument.execution_supported = true;
        instrument.asset_class = InstrumentAssetClass::Equity;
        let resolution = spot_resolution(instrument);
        let blocked = project_cex_instrument(&config, Some(&resolution), None);
        assert!(!blocked.ready);
        assert!(blocked
            .problem
            .as_deref()
            .is_some_and(|problem| problem.contains("不是加密资产")));
    }

    #[test]
    fn instrument_syncing_is_not_misreported_as_missing_or_unlisted() {
        let config = shared_types::OnchainComparisonConfig::default();
        let resolution = SpotInstrumentResolution {
            status: SpotInstrumentResolutionStatus::Syncing,
            instrument: None,
            checked_at_ms: None,
            problem: None,
        };

        let evidence = project_cex_instrument(&config, Some(&resolution), None);

        assert_eq!(evidence.status, OnchainCexInstrumentStatus::Syncing);
        assert!(!evidence.ready);
        assert!(evidence
            .problem
            .as_deref()
            .is_some_and(|problem| problem.contains("首次同步")));
        assert!(!evidence
            .problem
            .as_deref()
            .is_some_and(|problem| problem.contains("没有")));
    }

    #[test]
    fn cex_inventory_requirement_uses_the_asset_consumed_before_each_direction() {
        let mut instrument = spot_instrument();
        instrument.native_symbol = "PUPS/USD".to_owned();
        instrument.canonical_symbol = "PUPS".to_owned();
        instrument.quote_asset = Some("USD".to_owned());

        assert_eq!(
            cex_inventory_requirement(
                OnchainComparisonDirection::BuyOnchainSellCex,
                Some(&instrument),
                None,
                2.0,
                10.0,
                100.0,
            ),
            Some(CexInventoryRequirement {
                asset: "PUPS",
                required: 2.0,
            })
        );
        assert_eq!(
            cex_inventory_requirement(
                OnchainComparisonDirection::BuyCexSellOnchain,
                Some(&instrument),
                None,
                2.0,
                10.0,
                100.0,
            ),
            Some(CexInventoryRequirement {
                asset: "USD",
                required: 20.2,
            })
        );
        assert_eq!(
            cex_inventory_requirement(
                OnchainComparisonDirection::BuyCexSellOnchain,
                None,
                None,
                2.0,
                10.0,
                100.0,
            ),
            None
        );
    }

    #[test]
    fn cross_quote_buy_cex_requires_pre_conversion_inventory_and_both_fees() {
        let mut instrument = spot_instrument();
        instrument.native_symbol = "PUPS/USDT".to_owned();
        instrument.canonical_symbol = "PUPS".to_owned();
        instrument.quote_asset = Some("USDT".to_owned());
        let conversion = shared_types::OnchainQuoteConversionEvidence {
            venue: "binance".to_owned(),
            symbol: "USDC/USDT".to_owned(),
            source: "ws_push".to_owned(),
            cex_quote: "USDT".to_owned(),
            onchain_quote: "USDC".to_owned(),
            source_bid: 0.999,
            source_ask: 1.001,
            cex_to_onchain_bid: 0.999,
            cex_to_onchain_ask: 1.001,
            cex_to_onchain_capacity: 10_000.0,
            onchain_to_cex_capacity: 10_000.0,
            freshness_ms: 10,
            observed_at_ms: 990,
        };

        let requirement = cex_inventory_requirement(
            OnchainComparisonDirection::BuyCexSellOnchain,
            Some(&instrument),
            Some(&conversion),
            2.0,
            10.01,
            100.0,
        )
        .expect("cross-quote inventory requirement");

        assert_eq!(requirement.asset, "USDC");
        assert!((requirement.required - (2.0 * 10.01 * 1.01 / 0.99)).abs() < 1e-12);
    }

    #[test]
    fn path_legs_make_direct_and_cross_quote_execution_order_explicit() {
        let mut snapshot = OnchainComparisonSnapshot::default();
        snapshot.config.base_token = "PUPS".to_owned();
        snapshot.config.quote_token = "USDC".to_owned();
        snapshot.config.cex_venue = "kraken".to_owned();
        snapshot.config.cex_symbol = "PUPS/USDC".to_owned();

        let direct = path_legs(&snapshot, OnchainComparisonDirection::BuyOnchainSellCex);
        assert_eq!(direct.len(), 2);
        assert_eq!(
            (direct[0].from_asset.as_str(), direct[0].to_asset.as_str()),
            ("USDC", "PUPS")
        );
        assert_eq!(
            (direct[1].from_asset.as_str(), direct[1].to_asset.as_str()),
            ("PUPS", "USDC")
        );

        snapshot.config.cex_symbol = "PUPS/USDT".to_owned();
        snapshot.quote_conversion = Some(shared_types::OnchainQuoteConversionEvidence {
            venue: "kraken".to_owned(),
            symbol: "USDT/USDC".to_owned(),
            source: "ws_push".to_owned(),
            cex_quote: "USDT".to_owned(),
            onchain_quote: "USDC".to_owned(),
            source_bid: 1.0,
            source_ask: 1.0,
            cex_to_onchain_bid: 1.0,
            cex_to_onchain_ask: 1.0,
            cex_to_onchain_capacity: 10_000.0,
            onchain_to_cex_capacity: 10_000.0,
            freshness_ms: 10,
            observed_at_ms: 990,
        });
        let converted = path_legs(&snapshot, OnchainComparisonDirection::BuyCexSellOnchain);
        assert_eq!(converted.len(), 3);
        assert_eq!(converted[0].kind, OnchainPathLegKind::CexQuoteConversion);
        assert_eq!(
            (
                converted[0].from_asset.as_str(),
                converted[0].to_asset.as_str()
            ),
            ("USDC", "USDT")
        );
        assert_eq!(
            (
                converted[1].from_asset.as_str(),
                converted[1].to_asset.as_str()
            ),
            ("USDT", "PUPS")
        );
        assert_eq!(
            (
                converted[2].from_asset.as_str(),
                converted[2].to_asset.as_str()
            ),
            ("PUPS", "USDC")
        );
    }

    #[test]
    fn path_availability_distinguishes_matching_inventory_from_missing_inventory() {
        let mut snapshot = OnchainComparisonSnapshot::default();
        snapshot.quote_usd_valuation = Some(super::super::usd_valuation::fixture("USDC", 1.0, 0));
        snapshot.quality = OnchainComparisonQuality::Fresh;
        snapshot.config.base_token = "PUPS".to_owned();
        snapshot.config.quote_token = "USDC".to_owned();
        snapshot.config.gas_usd = 0.1;
        snapshot.execution_readiness.chain_submission_ready = true;
        snapshot.execution_readiness.cex_live_mode_ready = true;
        let comparison = OnchainCexComparison {
            direction: OnchainComparisonDirection::BuyOnchainSellCex,
            onchain_price: 1.0,
            cex_price: 1.0,
            gross_spread_bps: 120.0,
            cex_fee_bps: 10.0,
            quote_conversion_fee_bps: 0.0,
            slippage_bps: 10.0,
            gas_usd: 0.1,
            gas_bps: 10.0,
            total_cost_bps: 20.0,
            net_spread_bps: 100.0,
            observable_notional_usd: 100.0,
            executable: true,
        };
        let instrument = OnchainCexInstrumentEvidence {
            status: OnchainCexInstrumentStatus::Ready,
            ready: true,
            ..OnchainCexInstrumentEvidence::default()
        };
        let inventory = |location, asset: &str, status| OnchainInventoryEvidence {
            location,
            scope: "test".to_owned(),
            asset: asset.to_owned(),
            required: 1.0,
            available: Some(if status == OnchainInventoryStatus::Ready {
                1.0
            } else {
                0.0
            }),
            status,
            source: "test".to_owned(),
            observed_at_ms: Some(1),
            problem: None,
        };
        let mut rows = vec![
            inventory(
                OnchainInventoryLocation::Onchain,
                "USDC",
                OnchainInventoryStatus::Ready,
            ),
            inventory(
                OnchainInventoryLocation::Cex,
                "PUPS",
                OnchainInventoryStatus::Ready,
            ),
        ];

        assert_eq!(
            project_path_readiness(
                PathReadinessRequest {
                    snapshot: &snapshot,
                    comparison: &comparison,
                    direction: OnchainComparisonDirection::BuyOnchainSellCex,
                    inventory: &rows,
                    instrument: &instrument,
                    market_ready: true,
                    build_ready: true,
                },
                Vec::new(),
            )
            .availability,
            OnchainPathAvailability::ReadyToBuild
        );

        rows[1].status = OnchainInventoryStatus::Insufficient;
        rows[1].available = Some(0.0);
        assert_eq!(
            project_path_readiness(
                PathReadinessRequest {
                    snapshot: &snapshot,
                    comparison: &comparison,
                    direction: OnchainComparisonDirection::BuyOnchainSellCex,
                    inventory: &rows,
                    instrument: &instrument,
                    market_ready: true,
                    build_ready: false,
                },
                Vec::new(),
            )
            .availability,
            OnchainPathAvailability::InventoryRequired
        );

        let mut transfer = OnchainTransferEvidence {
            direction: OnchainTransferDirection::DepositToCex,
            venue: "binance".to_owned(),
            asset: "PUPS".to_owned(),
            chain: "solana".to_owned(),
            network: Some("SOL".to_owned()),
            amount: 1.0,
            amount_exact: Some("1".to_owned()),
            status: OnchainTransferStatus::Ready,
            fee: Some(0.0),
            fee_exact: Some("0".to_owned()),
            minimum: Some(0.0),
            minimum_exact: Some("0".to_owned()),
            amount_step: None,
            requires_tag: false,
            contract_verified: true,
            credit_confirmations: Some(1),
            unlock_confirmations: Some(1),
            network_status: None,
            source: Some("official_fixture".to_owned()),
            observed_at_ms: Some(1),
            problem: None,
        };
        let request = || PathReadinessRequest {
            snapshot: &snapshot,
            comparison: &comparison,
            direction: OnchainComparisonDirection::BuyOnchainSellCex,
            inventory: &rows,
            instrument: &instrument,
            market_ready: true,
            build_ready: false,
        };
        let profitable = project_path_readiness(request(), vec![transfer.clone()]);
        assert_eq!(
            profitable.availability,
            OnchainPathAvailability::Replenishable
        );
        assert_eq!(profitable.transfer_cost_usd, Some(0.1));
        assert_eq!(profitable.post_transfer_net_profit_usd, Some(0.9));

        let unprofitable = OnchainCexComparison {
            net_spread_bps: 5.0,
            ..comparison.clone()
        };
        assert_eq!(
            project_path_readiness(
                PathReadinessRequest {
                    comparison: &unprofitable,
                    ..request()
                },
                vec![transfer.clone()],
            )
            .availability,
            OnchainPathAvailability::TransferUnprofitable
        );

        transfer.status = OnchainTransferStatus::Unknown;
        assert_eq!(
            project_path_readiness(request(), vec![transfer]).availability,
            OnchainPathAvailability::EvidencePending
        );
    }

    fn spot_instrument() -> VenueInstrument {
        VenueInstrument {
            venue: "binance".to_owned(),
            native_symbol: "SOLUSDC".to_owned(),
            canonical_symbol: "SOL".to_owned(),
            display_symbol: "SOL/USDC".to_owned(),
            asset_class: InstrumentAssetClass::Crypto,
            product_type: Some("spot".to_owned()),
            quote_asset: Some("USDC".to_owned()),
            settle_asset: None,
            margin_asset: None,
            contract_size: Some(1.0),
            execution_supported: true,
            price_tick: Some(0.001),
            qty_step: Some(0.001),
            min_qty: Some(0.001),
            min_notional: Some(5.0),
            listing_status: InstrumentListingStatus::Trading,
            funding_interval_ms: None,
            builder_dex: None,
            source: InstrumentMetadataSource::OfficialEndpoint,
            source_url: Some("/api/v3/exchangeInfo".to_owned()),
            checked_at_ms: 1,
            schema_version: Some("binance-spot-v1".to_owned()),
        }
    }

    fn spot_resolution(instrument: VenueInstrument) -> SpotInstrumentResolution {
        SpotInstrumentResolution {
            status: SpotInstrumentResolutionStatus::Ready,
            checked_at_ms: Some(instrument.checked_at_ms),
            instrument: Some(instrument),
            problem: None,
        }
    }
}
