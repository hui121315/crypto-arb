use super::{cross_chain, lifi, wallet_inventory};
use crate::state::AppState;
use axum::http::StatusCode;
use common::AppError;
use rust_decimal::{prelude::ToPrimitive, Decimal};
use shared_types::{
    OnchainComparisonConfig as Config, OnchainCrossChainAssetChange as Asset,
    OnchainCrossChainDispositionAction as Action, OnchainCrossChainRecoveryPreview as Preview,
    OnchainCrossChainRecoveryPreviewRequest as Request, OnchainCrossChainRun as Run,
    OnchainCrossChainRunStatus as Status,
};

pub(crate) async fn preview(
    state: &AppState,
    request: &Request,
    actor: &str,
) -> Result<Preview, AppError> {
    tokio::time::timeout(
        std::time::Duration::from_secs(20),
        prepare(state, request, actor, &LiveIo(state)),
    )
    .await
    .map_err(|_| {
        error(
            "ONCHAIN_RECOVERY_PREVIEW_TIMEOUT",
            "余额与新报价核验超时，本次未签名或发送交易",
        )
    })?
}

#[async_trait::async_trait]
trait PreviewIo: Sync {
    async fn balance(
        &self,
        config: &Config,
        token: &str,
    ) -> Result<wallet_inventory::ExactAssetBalance, String>;
    async fn quote(
        &self,
        request: &lifi::RecoveryQuoteRequest,
    ) -> Result<lifi::RecoveryQuote, String>;
    async fn readiness(
        &self,
        config: &Config,
        token: &str,
        raw: &str,
        quote: &lifi::RecoveryQuote,
    ) -> Result<(), String>;
}

struct LiveIo<'a>(&'a AppState);
#[async_trait::async_trait]
impl PreviewIo for LiveIo<'_> {
    async fn balance(
        &self,
        config: &Config,
        token: &str,
    ) -> Result<wallet_inventory::ExactAssetBalance, String> {
        wallet_inventory::exact_asset_balance(self.0, config, token).await
    }
    async fn quote(
        &self,
        request: &lifi::RecoveryQuoteRequest,
    ) -> Result<lifi::RecoveryQuote, String> {
        lifi::fetch_recovery_quote(request).await
    }
    async fn readiness(
        &self,
        config: &Config,
        token: &str,
        raw: &str,
        quote: &lifi::RecoveryQuote,
    ) -> Result<(), String> {
        cross_chain::validate_current_execution_contract(
            self.0,
            config,
            "处置预检".into(),
            token,
            raw,
            &quote.transaction,
        )
        .await
    }
}

fn select(
    run: &Run,
    request: &Request,
    actor: &str,
) -> Result<(Asset, Asset, Action, String), AppError> {
    if run.authorization.actor != actor {
        return Err(AppError::domain(
            StatusCode::FORBIDDEN,
            "ONCHAIN_RECOVERY_ACTOR_MISMATCH",
            "仅原运行的账户可查看资金处置报价",
        ));
    }
    if run.updated_at_ms != request.expected_run_updated_at_ms
        || !matches!(
            run.status,
            Status::Paused | Status::Failed | Status::Compensating
        )
    {
        return Err(error(
            "ONCHAIN_RECOVERY_SOURCE_CHANGED",
            "原记录已变化或仍在执行，请刷新后重新预检",
        ));
    }
    let plan = run
        .accounting
        .as_ref()
        .and_then(|accounting| accounting.disposition.as_ref())
        .filter(|plan| plan.blockers.is_empty())
        .ok_or_else(|| {
            error(
                "ONCHAIN_RECOVERY_EVIDENCE_PENDING",
                "原交易的到账、扣款或费用尚未核齐",
            )
        })?;
    let selected = plan
        .remaining_assets
        .get(request.asset_index)
        .ok_or_else(|| error("ONCHAIN_RECOVERY_ASSET_MISSING", "本次剩余资产已变化"))?;
    let target = plan
        .original_capital
        .clone()
        .ok_or_else(|| error("ONCHAIN_RECOVERY_TARGET_MISSING", "原报价币及原钱包未核实"))?;
    if selected.action == Action::ReviewWallet {
        return Err(error(
            "ONCHAIN_RECOVERY_WALLET_REVIEW",
            "先核对不同钱包的归属；不自动选择转账路径",
        ));
    }
    let mut input = selected.change.clone();
    let maximum = to_raw(&input.amount_exact, input.asset.decimals)?;
    let requested = to_raw(request.amount_exact.trim(), input.asset.decimals)?;
    if requested > maximum {
        return Err(error(
            "ONCHAIN_RECOVERY_AMOUNT_EXCEEDED",
            "处置数量超过本次账面剩余，不能动用钱包其他资金",
        ));
    }
    input.amount_exact = Decimal::from_str_exact(request.amount_exact.trim())
        .unwrap()
        .normalize()
        .to_string();
    Ok((input, target, selected.action, requested.to_string()))
}

fn to_raw(value: &str, decimals: u8) -> Result<u128, AppError> {
    let invalid = || {
        error(
            "ONCHAIN_RECOVERY_AMOUNT_INVALID",
            "输入正数金额，不能超过代币精度或精确核算范围",
        )
    };
    if decimals > 28 || value.is_empty() || !value.bytes().all(|c| c.is_ascii_digit() || c == b'.')
    {
        return Err(invalid());
    }
    let amount = Decimal::from_str_exact(value).map_err(|_| invalid())?;
    if amount.normalize().scale() > u32::from(decimals) {
        return Err(invalid());
    }
    let scale = Decimal::from(10u128.pow(decimals as u32));
    let raw = amount
        .checked_mul(scale)
        .filter(|raw| raw.fract().is_zero())
        .ok_or_else(invalid)?;
    raw.to_u128().filter(|raw| *raw > 0).ok_or_else(invalid)
}

fn config(state: &AppState, input: &Asset) -> Result<Config, AppError> {
    let chain = lifi::chain_id(&input.chain)
        .ok_or_else(|| error("ONCHAIN_RECOVERY_CHAIN_UNKNOWN", "来源链未登记"))?;
    let active = state.onchain_monitor().snapshot().config.clone();
    std::iter::once(active)
        .chain(
            state
                .onchain_monitor()
                .batch()
                .configs()
                .into_iter()
                .map(|(_, config)| config),
        )
        .find(|config| {
            config.chain == input.chain
                && lifi::address_matches(chain, &config.wallet_address, &input.wallet)
        })
        .ok_or_else(|| {
            error(
                "ONCHAIN_RECOVERY_CONFIG_MISSING",
                "找不到对应链及钱包的 RPC 配置；请恢复该钱包配置后预检",
            )
        })
}

async fn prepare(
    state: &AppState,
    request: &Request,
    actor: &str,
    io: &impl PreviewIo,
) -> Result<Preview, AppError> {
    let now = common::time::now_ms();
    state
        .onchain_cross_chain_runs()
        .readiness()
        .map_err(|p| error("ONCHAIN_RECOVERY_LEDGER_UNAVAILABLE", p))?;
    let run = state
        .onchain_cross_chain_runs()
        .run(&request.run_id, now)
        .ok_or_else(|| error("ONCHAIN_RECOVERY_RUN_MISSING", "原运行记录不存在"))?;
    let (input, target, action, raw) = select(&run, request, actor)?;
    let config = config(state, &input)?;
    let mut result = Preview {
        plan_id: None,
        source_run_id: run.run_id.clone(),
        source_run_updated_at_ms: run.updated_at_ms,
        asset_index: request.asset_index,
        input,
        target,
        input_amount_raw: raw,
        balance_amount_raw: None,
        balance_source: None,
        balance_checked_at_ms: None,
        route_id: None,
        provider: "lifi".into(),
        expected_output_amount_raw: None,
        minimum_output_amount_raw: None,
        fee_usd: None,
        gas_usd: None,
        estimated_duration_seconds: None,
        quote_observed_at_ms: None,
        valid_until_ms: None,
        blockers: vec![],
        quote_ready: false,
        submit_ready: false,
        requires_live_authorization: true,
        official_docs_url: lifi::RECOVERY_QUOTE_DOCS.into(),
    };
    let mut transaction = None;
    match io.balance(&config, &result.input.asset.address).await {
        Ok(balance) => {
            result.balance_checked_at_ms = Some(now);
            result.balance_amount_raw = Some(balance.amount_raw.to_string());
            result.balance_source = Some(balance.source.into());
            if balance.amount_raw < result.input_amount_raw.parse::<u128>().unwrap() {
                result
                    .blockers
                    .push("当前链上余额低于所选处置金额，请核对其他支出或减少本次金额".into());
            }
        }
        Err(problem) => result.blockers.push(format!("当前余额无法核验：{problem}")),
    }
    if result.blockers.is_empty() && action == Action::Keep {
        result.provider = "none".into();
        result
            .blockers
            .push("资产已在原钱包且为原报价币，无需换币或转账".into());
    }
    if result.blockers.is_empty() {
        let request = lifi::RecoveryQuoteRequest {
            from_chain: result.input.chain.clone(),
            to_chain: result.target.chain.clone(),
            from_token: result.input.asset.address.clone(),
            to_token: result.target.asset.address.clone(),
            from_wallet: result.input.wallet.clone(),
            to_wallet: result.target.wallet.clone(),
            amount_raw: result.input_amount_raw.clone(),
            slippage_bps: config.slippage_bps,
        };
        match io.quote(&request).await {
            Ok(quote) => {
                transaction = Some(quote.transaction.clone());
                result.route_id = Some(quote.route_id.clone());
                result.expected_output_amount_raw = Some(quote.output_raw.clone());
                result.minimum_output_amount_raw = Some(quote.minimum_raw.clone());
                result.fee_usd = quote.fee_usd;
                result.gas_usd = quote.gas_usd;
                result.estimated_duration_seconds = quote.duration_seconds;
                result.quote_observed_at_ms = Some(quote.observed_at_ms);
                result.valid_until_ms = Some(
                    quote.valid_until_ms.min(
                        result
                            .balance_checked_at_ms
                            .unwrap()
                            .saturating_add(wallet_inventory::WALLET_INVENTORY_MAX_AGE_MS),
                    ),
                );
                if quote.fee_usd.is_none() || quote.gas_usd.is_none() {
                    result
                        .blockers
                        .push("报价费用或 Gas 美元估值不完整，不能按零成本规划".into());
                }
                if let Err(problem) = io
                    .readiness(
                        &config,
                        &result.input.asset.address,
                        &result.input_amount_raw,
                        &quote,
                    )
                    .await
                {
                    result.blockers.push(problem);
                }
            }
            Err(problem) => result.blockers.push(format!("重新询价失败：{problem}")),
        }
    }
    // A read-only quote never reserves funds. Discard even a good quote if its source changed in flight.
    state
        .onchain_cross_chain_runs()
        .readiness()
        .map_err(|p| error("ONCHAIN_RECOVERY_LEDGER_UNAVAILABLE", p))?;
    if state
        .onchain_cross_chain_runs()
        .run(&run.run_id, common::time::now_ms())
        .as_ref()
        != Some(&run)
        || self::config(state, &result.input)? != config
    {
        return Err(error(
            "ONCHAIN_RECOVERY_SOURCE_CHANGED",
            "预检期间原记录或钱包配置已变化，已丢弃本次报价",
        ));
    }
    if result
        .valid_until_ms
        .is_some_and(|deadline| deadline <= common::time::now_ms())
    {
        result
            .blockers
            .push("余额或报价在预检结束前已过期，请重新预检".into());
    }
    result.quote_ready = result.route_id.is_some() && result.blockers.is_empty();
    if result.quote_ready {
        let plan = state.onchain_cross_chain_runs().save_recovery_plan(result.clone(),
            transaction.ok_or_else(|| error("ONCHAIN_RECOVERY_CONTRACT_MISSING", "处置交易合同缺失"))?, common::time::now_ms())
            .map_err(|problem| error("ONCHAIN_RECOVERY_PLAN_SAVE_FAILED", problem))?;
        result.plan_id = Some(plan.plan_id);
    }
    Ok(result)
}

fn error(code: &'static str, message: impl Into<String>) -> AppError {
    AppError::domain(StatusCode::CONFLICT, code, message)
}

#[cfg(test)]
mod tests;
