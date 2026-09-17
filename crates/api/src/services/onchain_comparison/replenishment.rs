use crate::services::onchain_replenishment_plan_store::ReplenishmentAuthorizeError;
use crate::state::AppState;
use axum::http::StatusCode;
use common::AppError;
use exchange::{
    TransferDestinationEvidence, TransferDestinationRequest, TransferDestinationStatus,
    TransferDirection,
};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use shared_types::{
    OnchainComparisonDirection, OnchainComparisonSnapshot, OnchainDirectionReadiness,
    OnchainReplenishmentAuthorizeRequest, OnchainReplenishmentBuildRequest,
    OnchainReplenishmentDestination, OnchainReplenishmentDestinationStatus,
    OnchainReplenishmentLeg, OnchainReplenishmentLegEconomics, OnchainReplenishmentNetworkEvidence,
    OnchainReplenishmentPlanResponse, OnchainReplenishmentPlanStatus, OnchainReplenishmentRun,
    OnchainReplenishmentRunsResponse, OnchainTransferDirection, OnchainTransferEvidence,
    OnchainTransferStatus, ONCHAIN_REPLENISHMENT_AUTHORIZATION_PHRASE,
};

const PLAN_ID_KEY: &[u8] = b"crossline-onchain-replenishment-plan-v1";
const PLAN_VALIDITY_MS: i64 = 5_000;

pub(crate) async fn build(
    state: &AppState,
    request: &OnchainReplenishmentBuildRequest,
) -> Result<OnchainReplenishmentPlanResponse, AppError> {
    let snapshot = super::snapshot(state);
    validate_snapshot(&snapshot, request)?;
    let readiness = direction_readiness(&snapshot, request.direction)?;
    if readiness.path.replenishment.is_empty() {
        return Err(conflict(
            "ONCHAIN_REPLENISHMENT_NOT_REQUIRED",
            "当前方向没有缺口，不需要生成补仓计划",
        ));
    }

    let built_at_ms = common::time::now_ms();
    let valid_until_ms = plan_valid_until(&snapshot, request, built_at_ms);
    if valid_until_ms <= built_at_ms {
        return Err(conflict(
            "ONCHAIN_REPLENISHMENT_SNAPSHOT_STALE",
            "补仓计划依赖的链上或 CEX 行情已经过期，请刷新后重试",
        ));
    }
    let mut legs = Vec::with_capacity(readiness.path.replenishment.len());
    let comparison = snapshot
        .comparisons
        .iter()
        .find(|row| row.direction == request.direction);
    for evidence in &readiness.path.replenishment {
        let destination = fetch_destination(state, &snapshot, evidence).await;
        let mut leg = build_leg(&snapshot, evidence, destination.as_ref());
        leg.economics.estimated_cost_usd = comparison.and_then(|row| {
            super::execution_readiness::transfer_economics(
                &snapshot,
                row,
                std::slice::from_ref(evidence),
            )
            .transfer_cost_usd
        });
        legs.push(leg);
    }
    let economics = comparison
        .map(|row| {
            super::execution_readiness::transfer_economics(
                &snapshot,
                row,
                &readiness.path.replenishment,
            )
        })
        .unwrap_or_default();
    let mut blockers = collect_blockers(&legs, economics.post_transfer_net_profit_usd);
    collect_chain_deposit_blockers(state, &snapshot, &legs, &mut blockers);
    if let Err(problem) = state.onchain_replenishment_plans().readiness() {
        blockers.push(problem);
    }
    let status = plan_status(&legs, economics.post_transfer_net_profit_usd, &blockers);
    let submit_ready =
        status == OnchainReplenishmentPlanStatus::ReadyForAuthorization && blockers.is_empty();
    let plan_id = plan_id(request, &legs);
    let mut response = OnchainReplenishmentPlanResponse {
        plan_id,
        direction: request.direction,
        status,
        legs,
        transfer_cost_usd: economics.transfer_cost_usd,
        post_transfer_net_profit_usd: economics.post_transfer_net_profit_usd,
        built_at_ms,
        valid_until_ms,
        requires_live_authorization: true,
        submit_ready,
        blockers,
    };
    if let Err(problem) = state
        .onchain_replenishment_plans()
        .insert(response.clone(), built_at_ms)
    {
        response.status = OnchainReplenishmentPlanStatus::Blocked;
        response.submit_ready = false;
        response
            .blockers
            .push(format!("链上补仓计划持久化失败：{problem}"));
    }
    Ok(response)
}

pub(crate) fn recent_plans(
    state: &AppState,
    limit: usize,
) -> shared_types::OnchainReplenishmentPlansResponse {
    state
        .onchain_replenishment_plans()
        .plans(limit, common::time::now_ms())
}

pub(crate) fn authorize(
    state: &AppState,
    request: &OnchainReplenishmentAuthorizeRequest,
    actor: &str,
) -> Result<OnchainReplenishmentRun, AppError> {
    if request.confirmation != ONCHAIN_REPLENISHMENT_AUTHORIZATION_PHRASE {
        return Err(AppError::domain(
            StatusCode::BAD_REQUEST,
            "ONCHAIN_REPLENISHMENT_CONFIRMATION_REQUIRED",
            format!("必须完整输入授权短语：{ONCHAIN_REPLENISHMENT_AUTHORIZATION_PHRASE}"),
        ));
    }
    let plan_id = request.plan_id.trim();
    let idempotency_key = request.idempotency_key.trim();
    if plan_id.is_empty() || idempotency_key.is_empty() {
        return Err(AppError::domain(
            StatusCode::BAD_REQUEST,
            "ONCHAIN_REPLENISHMENT_AUTHORIZATION_INVALID",
            "planId 与 idempotencyKey 均不能为空",
        ));
    }
    state
        .onchain_replenishment_plans()
        .readiness()
        .map_err(|problem| conflict("ONCHAIN_REPLENISHMENT_RECOVERY_UNAVAILABLE", problem))?;
    state
        .onchain_replenishment_plans()
        .authorize(plan_id, idempotency_key, actor, common::time::now_ms())
        .map(|outcome| outcome.run)
        .map_err(map_authorize_error)
}

pub(crate) fn recent_runs(state: &AppState, limit: usize) -> OnchainReplenishmentRunsResponse {
    state
        .onchain_replenishment_plans()
        .runs(limit, common::time::now_ms())
}

fn map_authorize_error(error: ReplenishmentAuthorizeError) -> AppError {
    match error {
        ReplenishmentAuthorizeError::Missing => AppError::domain(
            StatusCode::NOT_FOUND,
            "ONCHAIN_REPLENISHMENT_PLAN_MISSING",
            "补仓计划不存在或已被清理，请重新生成",
        ),
        ReplenishmentAuthorizeError::Expired => conflict(
            "ONCHAIN_REPLENISHMENT_PLAN_EXPIRED",
            "补仓计划已过期，请基于最新行情和网络证据重新生成",
        ),
        ReplenishmentAuthorizeError::NotReady(problem) => {
            conflict("ONCHAIN_REPLENISHMENT_PLAN_NOT_READY", problem)
        }
        ReplenishmentAuthorizeError::IdempotencyConflict => conflict(
            "ONCHAIN_REPLENISHMENT_IDEMPOTENCY_CONFLICT",
            "该幂等键已绑定另一条补仓运行记录",
        ),
        ReplenishmentAuthorizeError::Persistence(problem) => conflict(
            "ONCHAIN_REPLENISHMENT_AUTHORIZATION_NOT_DURABLE",
            format!("补仓授权未能持久化，未创建运行记录：{problem}"),
        ),
    }
}

fn validate_snapshot(
    snapshot: &OnchainComparisonSnapshot,
    request: &OnchainReplenishmentBuildRequest,
) -> Result<(), AppError> {
    if snapshot.quote_observed_at_ms != Some(request.expected_quote_observed_at_ms)
        || snapshot.cex_observed_at_ms != Some(request.expected_cex_observed_at_ms)
    {
        return Err(conflict(
            "ONCHAIN_REPLENISHMENT_SNAPSHOT_CHANGED",
            "链上或 CEX 快照已变化，请基于最新数据重新生成补仓计划",
        ));
    }
    Ok(())
}

fn direction_readiness(
    snapshot: &OnchainComparisonSnapshot,
    direction: OnchainComparisonDirection,
) -> Result<&OnchainDirectionReadiness, AppError> {
    snapshot
        .execution_readiness
        .directions
        .iter()
        .find(|row| row.direction == direction)
        .ok_or_else(|| {
            conflict(
                "ONCHAIN_REPLENISHMENT_DIRECTION_MISSING",
                "当前快照没有该方向的补仓证据",
            )
        })
}

fn build_leg(
    snapshot: &OnchainComparisonSnapshot,
    evidence: &OnchainTransferEvidence,
    official_destination: Option<&TransferDestinationEvidence>,
) -> OnchainReplenishmentLeg {
    let destination = destination(snapshot, evidence, official_destination);
    let (asset_address, asset_decimals) = asset_identity(snapshot, &evidence.asset);
    let transfer_amount_exact = exact_transfer_amount(evidence, asset_decimals);
    let source_debit_upper_bound_exact =
        exact_source_debit(evidence, transfer_amount_exact.as_deref());
    let source_debit_upper_bound = source_debit_upper_bound_exact
        .as_deref()
        .and_then(|value| value.parse::<Decimal>().ok())
        .and_then(|value| value.to_f64());
    let blocker = leg_blocker(evidence, &destination, transfer_amount_exact.as_deref())
        .or_else(|| {
            (asset_address.is_none() || asset_decimals.is_none())
                .then(|| "补仓资产缺少链上合约/Mint 或精度，无法核验最终到账".to_owned())
        })
        .or_else(|| {
            source_debit_upper_bound_exact
                .is_none()
                .then(|| "精确手续费或最高扣款不一致，请重新生成补仓计划".to_owned())
        });
    OnchainReplenishmentLeg {
        direction: evidence.direction,
        venue: evidence.venue.clone(),
        asset: evidence.asset.clone(),
        chain: evidence.chain.clone(),
        source_address: (evidence.direction == OnchainTransferDirection::DepositToCex)
            .then(|| snapshot.config.wallet_address.trim().to_owned())
            .filter(|address| !address.is_empty()),
        asset_address,
        asset_decimals,
        transfer_amount: evidence.amount,
        transfer_amount_exact,
        economics: OnchainReplenishmentLegEconomics {
            estimated_cost_usd: None,
            estimated_network_cost_usd: (evidence.direction
                == OnchainTransferDirection::DepositToCex)
                .then_some(snapshot.config.gas_usd)
                .filter(|fee| fee.is_finite() && *fee >= 0.0),
            reconciled_cost_usd: None,
            fee_amount: evidence.fee,
            fee_amount_exact: evidence.fee_exact.clone(),
            source_debit_upper_bound,
            source_debit_upper_bound_exact,
        },
        network_evidence: OnchainReplenishmentNetworkEvidence {
            network: evidence.network.clone(),
            minimum_amount: evidence.minimum,
            minimum_amount_exact: evidence.minimum_exact.clone(),
            amount_step: evidence.amount_step.clone(),
            credit_confirmations: evidence.credit_confirmations,
            unlock_confirmations: evidence.unlock_confirmations,
            transfer_status: evidence.status,
            evidence_source: evidence.source.clone(),
            evidence_observed_at_ms: evidence.observed_at_ms,
        },
        destination,
        blocker,
    }
}

fn asset_identity(
    snapshot: &OnchainComparisonSnapshot,
    asset: &str,
) -> (Option<String>, Option<u8>) {
    if asset.eq_ignore_ascii_case(&snapshot.config.base_token) {
        return (
            non_empty(&snapshot.config.base_mint),
            Some(snapshot.config.base_decimals),
        );
    }
    if asset.eq_ignore_ascii_case(&snapshot.config.quote_token) {
        return (
            non_empty(&snapshot.config.quote_mint),
            Some(snapshot.config.quote_decimals),
        );
    }
    (None, None)
}

fn destination(
    snapshot: &OnchainComparisonSnapshot,
    evidence: &OnchainTransferEvidence,
    official: Option<&TransferDestinationEvidence>,
) -> OnchainReplenishmentDestination {
    if let Some(official) = official {
        return OnchainReplenishmentDestination {
            address: official.address.clone(),
            tag: official.tag.clone(),
            status: match official.status {
                TransferDestinationStatus::Verified => {
                    OnchainReplenishmentDestinationStatus::Verified
                }
                TransferDestinationStatus::Missing => {
                    OnchainReplenishmentDestinationStatus::Missing
                }
                TransferDestinationStatus::Unverified => {
                    OnchainReplenishmentDestinationStatus::ConfiguredUnverified
                }
            },
            source: Some(official.source_url.clone()),
            observed_at_ms: Some(official.checked_at_ms),
            problem: official.problem.clone(),
        };
    }
    match evidence.direction {
        OnchainTransferDirection::WithdrawToChain => {
            let address = non_empty(&snapshot.config.wallet_address);
            let (status, problem) = if address.is_some() {
                (
                    OnchainReplenishmentDestinationStatus::ConfiguredUnverified,
                    Some("链上钱包已配置，但尚未通过交易所官方提币地址/白名单接口核验".to_owned()),
                )
            } else {
                (
                    OnchainReplenishmentDestinationStatus::Missing,
                    Some("尚未配置目标链钱包地址".to_owned()),
                )
            };
            OnchainReplenishmentDestination {
                address,
                tag: None,
                status,
                source: None,
                observed_at_ms: None,
                problem,
            }
        }
        OnchainTransferDirection::DepositToCex => OnchainReplenishmentDestination {
            address: None,
            tag: None,
            status: OnchainReplenishmentDestinationStatus::Missing,
            source: None,
            observed_at_ms: None,
            problem: Some("尚未通过交易所官方接口取得并核验充值地址".to_owned()),
        },
    }
}

async fn fetch_destination(
    state: &AppState,
    snapshot: &OnchainComparisonSnapshot,
    evidence: &OnchainTransferEvidence,
) -> Option<TransferDestinationEvidence> {
    let adapter = state.aggregator().get(&evidence.venue)?;
    let request = TransferDestinationRequest {
        currency: evidence.asset.clone(),
        network: evidence.network.clone()?,
        direction: match evidence.direction {
            OnchainTransferDirection::WithdrawToChain => TransferDirection::WithdrawToChain,
            OnchainTransferDirection::DepositToCex => TransferDirection::DepositToVenue,
        },
        expected_address: (evidence.direction == OnchainTransferDirection::WithdrawToChain)
            .then(|| snapshot.config.wallet_address.trim().to_owned())
            .filter(|address| !address.is_empty()),
        expected_tag: None,
        amount: evidence
            .amount_exact
            .as_deref()
            .and_then(|amount| Decimal::from_str_exact(amount).ok()),
    };
    match adapter.fetch_transfer_destination(&request).await {
        Ok(destination) => Some(destination),
        Err(error) => {
            tracing::warn!(
                venue = %evidence.venue,
                asset = %evidence.asset,
                network = %request.network,
                %error,
                "on-chain replenishment destination evidence unavailable"
            );
            None
        }
    }
}

fn leg_blocker(
    evidence: &OnchainTransferEvidence,
    destination: &OnchainReplenishmentDestination,
    transfer_amount_exact: Option<&str>,
) -> Option<String> {
    if evidence.status != OnchainTransferStatus::Ready {
        return Some(
            evidence
                .problem
                .clone()
                .unwrap_or_else(|| "充提网络证据未就绪".to_owned()),
        );
    }
    if !evidence.contract_verified {
        return Some("链、网络与合约身份尚未核验".to_owned());
    }
    if evidence.fee.is_none_or(|fee| !fee.is_finite() || fee < 0.0) {
        return Some("提币或链上转账费用尚未取得".to_owned());
    }
    if evidence.direction == OnchainTransferDirection::WithdrawToChain
        && evidence.fee_exact.is_none()
    {
        return Some("交易所提币手续费缺少精确十进制证据".to_owned());
    }
    if transfer_amount_exact.is_none() {
        return Some(match evidence.direction {
            OnchainTransferDirection::WithdrawToChain => {
                "提币精确数量与官方步长、费用尚未一致，请重新生成补仓计划".to_owned()
            }
            OnchainTransferDirection::DepositToCex => {
                "链上转账精确数量与代币精度尚未一致，请重新生成补仓计划".to_owned()
            }
        });
    }
    if evidence
        .minimum
        .is_some_and(|minimum| evidence.amount + f64::EPSILON < minimum)
    {
        return Some("计划数量低于交易所最小充提数量".to_owned());
    }
    if evidence.requires_tag && destination.tag.is_none() {
        return Some("该网络需要 memo/tag，但目的地标签尚未取得".to_owned());
    }
    destination.problem.clone()
}

fn exact_transfer_amount(
    evidence: &OnchainTransferEvidence,
    asset_decimals: Option<u8>,
) -> Option<String> {
    // Do not round a second time: network fees and balance checks used amount_exact.
    let amount = evidence.amount_exact.as_deref()?.parse::<Decimal>().ok()?;
    let atomic_step = Decimal::try_new(1, u32::from(asset_decimals?)).ok()?;
    if amount <= Decimal::ZERO
        || !amount.checked_rem(atomic_step)?.is_zero()
        || amount.to_f64()? != evidence.amount
        || amount < evidence.minimum_exact.as_deref()?.parse::<Decimal>().ok()?
    {
        return None;
    }
    if evidence.direction == OnchainTransferDirection::WithdrawToChain {
        let step = evidence.amount_step.as_deref()?.parse::<Decimal>().ok()?;
        if step <= Decimal::ZERO || !amount.checked_rem(step)?.is_zero() {
            return None;
        }
    }
    Some(amount.normalize().to_string())
}

fn exact_source_debit(
    evidence: &OnchainTransferEvidence,
    transfer_amount_exact: Option<&str>,
) -> Option<String> {
    let amount = transfer_amount_exact?.parse::<Decimal>().ok()?;
    let debit = match evidence.direction {
        OnchainTransferDirection::WithdrawToChain => {
            let fee = evidence.fee_exact.as_deref()?.parse::<Decimal>().ok()?;
            if fee < Decimal::ZERO || fee.to_f64() != evidence.fee {
                return None;
            }
            amount.checked_add(fee)?
        }
        OnchainTransferDirection::DepositToCex => amount,
    };
    Some(debit.normalize().to_string())
}

fn collect_blockers(
    legs: &[OnchainReplenishmentLeg],
    post_transfer_net_profit_usd: Option<f64>,
) -> Vec<String> {
    let mut blockers = legs
        .iter()
        .filter_map(|leg| {
            leg.blocker
                .as_ref()
                .map(|problem| format!("{} {}: {problem}", leg.venue, leg.asset))
        })
        .collect::<Vec<_>>();
    match post_transfer_net_profit_usd.filter(|profit| profit.is_finite()) {
        Some(profit) if profit <= 0.0 => {
            blockers.push(format!("计入搬运费后预计净收益 ${profit:.4}，不允许补仓"))
        }
        None => blockers.push("搬运费或搬运后净收益尚未能可靠估值".to_owned()),
        Some(_) => {}
    }
    blockers
}

fn collect_chain_deposit_blockers(
    state: &AppState,
    snapshot: &OnchainComparisonSnapshot,
    legs: &[OnchainReplenishmentLeg],
    blockers: &mut Vec<String>,
) {
    for leg in legs
        .iter()
        .filter(|leg| leg.direction == OnchainTransferDirection::DepositToCex)
    {
        if !state.trading_service().deposit_status_supported(&leg.venue) {
            blockers.push(format!(
                "{} 尚未接入官方充值历史终态核验",
                leg.venue.to_ascii_uppercase()
            ));
        }
    }
    if !legs
        .iter()
        .any(|leg| leg.direction == OnchainTransferDirection::DepositToCex)
    {
        return;
    }
    if let Err(problem) = crate::services::onchain_signer::readiness(
        &snapshot.config.chain,
        &snapshot.config.wallet_address,
    ) {
        blockers.push(problem);
    }
    if let Err(problem) = super::execution_submit::submission_rpc(state, &snapshot.config) {
        blockers.push(problem);
    }
}

fn plan_status(
    legs: &[OnchainReplenishmentLeg],
    post_transfer_net_profit_usd: Option<f64>,
    blockers: &[String],
) -> OnchainReplenishmentPlanStatus {
    if post_transfer_net_profit_usd.is_some_and(|profit| profit <= 0.0) {
        return OnchainReplenishmentPlanStatus::Unprofitable;
    }
    if legs.iter().any(|leg| {
        matches!(
            leg.network_evidence.transfer_status,
            OnchainTransferStatus::Blocked | OnchainTransferStatus::Unsupported
        )
    }) {
        return OnchainReplenishmentPlanStatus::Blocked;
    }
    if blockers.is_empty() {
        OnchainReplenishmentPlanStatus::ReadyForAuthorization
    } else {
        OnchainReplenishmentPlanStatus::EvidencePending
    }
}

fn plan_valid_until(
    snapshot: &OnchainComparisonSnapshot,
    request: &OnchainReplenishmentBuildRequest,
    built_at_ms: i64,
) -> i64 {
    let max_age_ms = snapshot.config.max_age_ms.max(1_000);
    built_at_ms
        .saturating_add(PLAN_VALIDITY_MS)
        .min(
            request
                .expected_quote_observed_at_ms
                .saturating_add(max_age_ms),
        )
        .min(
            request
                .expected_cex_observed_at_ms
                .saturating_add(max_age_ms),
        )
}

fn plan_id(request: &OnchainReplenishmentBuildRequest, legs: &[OnchainReplenishmentLeg]) -> String {
    let legs = serde_json::to_string(legs).unwrap_or_default();
    let canonical = format!(
        "{:?}|{}|{}|{}",
        request.direction,
        request.expected_quote_observed_at_ms,
        request.expected_cex_observed_at_ms,
        legs
    );
    let digest = common::signing::hmac_sha256_hex(PLAN_ID_KEY, canonical.as_bytes());
    format!("onchain-replenishment-{}", &digest[..24])
}

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn conflict(code: &'static str, message: impl Into<String>) -> AppError {
    AppError::domain(StatusCode::CONFLICT, code, message.into())
}

#[cfg(test)]
mod tests {
    use shared_types::{OnchainComparisonConfig, OnchainTransferStatus};

    use super::*;

    #[test]
    fn withdrawal_plan_keeps_configured_wallet_unverified_until_provider_proof() {
        let mut snapshot = OnchainComparisonSnapshot::default();
        snapshot.config = OnchainComparisonConfig {
            wallet_address: "wallet-1".to_owned(),
            ..OnchainComparisonConfig::default()
        };
        let evidence = OnchainTransferEvidence {
            direction: OnchainTransferDirection::WithdrawToChain,
            venue: "binance".to_owned(),
            asset: "USDC".to_owned(),
            chain: "solana".to_owned(),
            network: Some("SOL".to_owned()),
            amount: 100.0,
            amount_exact: Some("100".to_owned()),
            status: OnchainTransferStatus::Ready,
            fee: Some(1.0),
            fee_exact: Some("1".to_owned()),
            minimum: Some(10.0),
            minimum_exact: Some("10".to_owned()),
            amount_step: Some("0.01".to_owned()),
            requires_tag: false,
            contract_verified: true,
            credit_confirmations: Some(1),
            unlock_confirmations: Some(2),
            network_status: Some("online".to_owned()),
            source: Some("official".to_owned()),
            observed_at_ms: Some(10),
            problem: None,
        };
        let leg = build_leg(&snapshot, &evidence, None);
        assert_eq!(leg.transfer_amount_exact.as_deref(), Some("100"));
        assert_eq!(
            leg.economics.source_debit_upper_bound_exact.as_deref(),
            Some("101")
        );
        assert_eq!(leg.economics.source_debit_upper_bound, Some(101.0));
        assert_eq!(
            leg.destination.status,
            OnchainReplenishmentDestinationStatus::ConfiguredUnverified
        );
        assert!(leg
            .blocker
            .as_deref()
            .is_some_and(|problem| problem.contains("白名单")));
    }

    #[test]
    fn withdrawal_plan_preserves_compiled_amount_and_debit() {
        let mut snapshot = OnchainComparisonSnapshot::default();
        snapshot.config.wallet_address = "wallet-1".to_owned();
        let evidence = OnchainTransferEvidence {
            direction: OnchainTransferDirection::WithdrawToChain,
            venue: "binance".to_owned(),
            asset: "USDC".to_owned(),
            chain: "solana".to_owned(),
            network: Some("SOL".to_owned()),
            amount: 100.01,
            amount_exact: Some("100.01".to_owned()),
            status: OnchainTransferStatus::Ready,
            fee: Some(1.0),
            fee_exact: Some("1".to_owned()),
            minimum: Some(10.0),
            minimum_exact: Some("10".to_owned()),
            amount_step: Some("0.01".to_owned()),
            requires_tag: false,
            contract_verified: true,
            credit_confirmations: Some(1),
            unlock_confirmations: Some(2),
            network_status: Some("online".to_owned()),
            source: Some("official".to_owned()),
            observed_at_ms: Some(10),
            problem: None,
        };

        let leg = build_leg(&snapshot, &evidence, None);

        assert_eq!(leg.transfer_amount_exact.as_deref(), Some("100.01"));
        assert_eq!(leg.transfer_amount, 100.01);
        assert_eq!(leg.economics.source_debit_upper_bound, Some(101.01));
        assert_eq!(
            leg.economics.source_debit_upper_bound_exact.as_deref(),
            Some("101.01")
        );
        assert_eq!(leg.network_evidence.amount_step.as_deref(), Some("0.01"));

        for (amount, exact, fee) in [
            (100.001, Some("100.001"), "1"),
            (100.01, Some("100.02"), "1"),
            (100.01, None, "1"),
            (100.01, Some("100.01"), "-1"),
            (100.01, Some("100.01"), "2"),
        ] {
            let mut invalid = evidence.clone();
            invalid.amount = amount;
            invalid.amount_exact = exact.map(str::to_owned);
            invalid.fee_exact = Some(fee.to_owned());
            let leg = build_leg(&snapshot, &invalid, None);
            assert!(leg.blocker.is_some());
            assert!(leg.economics.source_debit_upper_bound_exact.is_none());
        }
    }

    #[test]
    fn replenishment_registry_to_plan_preserves_rounding_and_rejects_consumed_profit() {
        use crate::services::instrument_registry::InstrumentRegistry;
        let registry = InstrumentRegistry::default();
        registry.replace_transfer_venue(
            "binance",
            vec![exchange::CurrencyTransferNetwork {
                venue: "binance".to_owned(),
                currency: "USDC".to_owned(),
                network: "BASE".to_owned(),
                canonical_network: "base".to_owned(),
                contract_address: Some("0xabc".to_owned()),
                deposit_enabled: true,
                withdraw_enabled: true,
                withdrawal_fee: Some(Decimal::new(1, 1)),
                withdrawal_fee_rate: Some(Decimal::new(1, 2)),
                withdrawal_step: Some(Decimal::new(1, 2)),
                min_withdraw: Some(Decimal::new(10003, 3)),
                min_deposit: Some(Decimal::ZERO),
                requires_tag: false,
                credit_confirmations: Some(1),
                unlock_confirmations: Some(2),
                network_status: None,
                checked_at_ms: 1_000,
                source_url: "fixture".to_owned(),
            }],
        );
        let evidence = registry.onchain_transfer_evidence(
            "binance",
            "USDC",
            "base",
            Some("0xabc"),
            OnchainTransferDirection::WithdrawToChain,
            Decimal::new(3001, 4),
            6,
            1_100,
        );
        let mut snapshot = OnchainComparisonSnapshot::default();
        snapshot.config.wallet_address = "0xwallet".to_owned();
        snapshot.quote_usd_valuation = Some(super::super::usd_valuation::fixture("USDC", 1.0, 0));
        snapshot.config.quote_token = "USDC".to_owned();
        snapshot.config.quote_mint = "0xabc".to_owned();
        let destination = TransferDestinationEvidence {
            venue: "binance".to_owned(),
            currency: "USDC".to_owned(),
            network: "BASE".to_owned(),
            direction: TransferDirection::WithdrawToChain,
            address: Some("0xwallet".to_owned()),
            tag: None,
            status: TransferDestinationStatus::Verified,
            allowlisted: Some(true),
            checked_at_ms: 1_100,
            source_url: "fixture".to_owned(),
            problem: None,
        };
        let leg = build_leg(&snapshot, &evidence, Some(&destination));
        assert!(leg.blocker.is_none(), "{:?}", leg.blocker);
        assert_eq!(leg.transfer_amount, 10.01);
        assert_eq!(leg.transfer_amount_exact.as_deref(), Some("10.01"));
        assert_eq!(leg.economics.fee_amount_exact.as_deref(), Some("0.2001"));
        assert_eq!(leg.economics.source_debit_upper_bound, Some(10.2101));
        assert_eq!(
            leg.economics.source_debit_upper_bound_exact.as_deref(),
            Some("10.2101")
        );

        let comparison = shared_types::OnchainCexComparison {
            direction: OnchainComparisonDirection::BuyOnchainSellCex,
            onchain_price: 1.0,
            cex_price: 1.01,
            gross_spread_bps: 100.0,
            cex_fee_bps: 0.0,
            quote_conversion_fee_bps: 0.0,
            slippage_bps: 0.0,
            gas_usd: 0.0,
            gas_bps: 0.0,
            total_cost_bps: 0.0,
            net_spread_bps: 20.0,
            observable_notional_usd: 100.0,
            executable: true,
        };
        let economics = super::super::execution_readiness::transfer_economics(
            &snapshot,
            &comparison,
            &[evidence],
        );
        assert_eq!(economics.transfer_cost_usd, Some(0.2001));
        let profit = economics.post_transfer_net_profit_usd;
        assert!(profit.unwrap() < 0.0);
        let blockers = collect_blockers(&[leg.clone()], profit);
        assert!(!blockers.is_empty());
        assert_eq!(
            plan_status(&[leg], profit, &blockers),
            OnchainReplenishmentPlanStatus::Unprofitable
        );
    }
}
