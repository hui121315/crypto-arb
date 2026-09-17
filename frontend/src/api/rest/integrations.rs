use shared_types::{
    AutomatedArbitrageConfigPatch, AutomationControlRequest, AutomationRuntimeStatus,
    DeterministicExecutionArtifact, ExecutionArtifactBuildRequest,
    ExecutionArtifactValidationRequest, ExecutionArtifactValidationResponse,
    GateCrossExModeConfigPatch, GateCrossExModeSnapshot, GateCrossExRouteCatalogResponse,
    OnchainBatchRemoveRequest, OnchainBatchSnapshot, OnchainCexPairCatalog,
    OnchainComparisonConfigPatch, OnchainComparisonSnapshot, OnchainCrossChainAuthorizeRequest,
    OnchainCrossChainBuildRequest, OnchainCrossChainBuildResponse, OnchainCrossChainRecheckRequest, OnchainCrossChainRun,
    OnchainCrossChainRunsResponse, OnchainCrossChainSubmitRequest, OnchainExecutionBuildRequest,
    OnchainCrossChainRecoveryPreview, OnchainCrossChainRecoveryPreviewRequest,
    OnchainCrossChainRecoveryPlan, OnchainCrossChainRecoveryAuthorizeRequest, OnchainCrossChainRecoveryCancelRequest,
    OnchainExecutionBuildResponse, OnchainExecutionRunsResponse, OnchainExecutionSubmitRequest,
    OnchainExecutionSubmitResponse,
    OnchainProviderCredentialClearRequest, OnchainProviderCredentialMutationResponse,
    OnchainProviderCredentialUpdateRequest, OnchainProviderCredentialsResponse,
    OnchainReplenishmentAuthorizeRequest, OnchainReplenishmentBuildRequest,
    OnchainReplenishmentPlanResponse, OnchainReplenishmentRun, OnchainReplenishmentRunsResponse,
    OnchainReplenishmentSubmitRequest,
    OnchainTokenApprovalBuildRequest, OnchainTokenApprovalBuildResponse,
    OnchainTokenApprovalRunsResponse, OnchainTokenApprovalSubmitRequest,
    OnchainTokenApprovalSubmitResponse, OnchainTokenIdentityRequest, OnchainTokenResolution,
    VenueCredentialValue, WebhookConfigPatch, WebhookRuntimeStatus, WebhookTestRequest,
};

use super::{encoding::encode_query_component, ApiClient, ApiError, MutationRequestContext};

impl ApiClient {
    pub async fn build_stock_exchange_conversion(&self,r:&shared_types::stocks::StockExchangeConversionRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError>{
        super::timeout::with_mutation_timeout_ms("生成 Backpack 账户兑换计划",self.post_json("/api/stocks/funding/exchange-conversions",r),24_000).await
    }
    pub async fn cancel_stock_exchange_conversion(&self,r:&shared_types::stocks::StockPlanRevisionRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError>{
        super::with_mutation_timeout("取消账户兑换预留",self.post_json("/api/stocks/funding/exchange-conversions/cancel",r)).await
    }
    pub async fn submit_stock_exchange_conversion(&self,r:&shared_types::stocks::StockStablecoinSubmitRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError>{
        super::timeout::with_mutation_timeout_ms("提交原账户兑换",self.post_json("/api/stocks/funding/exchange-conversions/submit",r),30_000).await
    }
    pub async fn recheck_stock_exchange_conversion(&self,r:&shared_types::stocks::StockPlanCancelRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError>{
        super::timeout::with_mutation_timeout_ms("核对原账户兑换",self.post_json("/api/stocks/funding/exchange-conversions/recheck",r),30_000).await
    }
    pub async fn build_stock_plan(&self,request:&shared_types::stocks::StockPlanBuildRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError>{
        super::timeout::with_mutation_timeout_ms("构建并预留股票计划",self.post_json("/api/stocks/plans/build",request),40_000).await
    }
    pub async fn prepare_stock_recovery(&self,request:&shared_types::stocks::StockRecoveryBuildRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError>{
        super::timeout::with_mutation_timeout_ms("试算股票补偿",self.post_json("/api/stocks/plans/recovery",request),44_000).await
    }
    pub async fn cancel_stock_recovery(&self,request:&shared_types::stocks::StockRecoveryActionRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError>{
        super::with_mutation_timeout("取消未提交补偿",self.post_json("/api/stocks/plans/recovery/cancel",request)).await
    }
    pub async fn recheck_stock_recovery(&self,request:&shared_types::stocks::StockRecoveryActionRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError>{
        super::with_mutation_timeout("核对原补偿",self.post_json("/api/stocks/plans/recovery/recheck",request)).await
    }
    pub async fn execute_stock_plan(&self, request:&shared_types::stocks::StockPlanExecutionRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError>{
        super::timeout::with_mutation_timeout_ms("提交股票原计划",self.post_json("/api/stocks/plans/execute",request),30_000).await
    }
    pub async fn reserve_stock_plan(&self,request:&shared_types::stocks::StockPlanRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError>{
        super::with_mutation_timeout("保存股票预留计划",self.post_json("/api/stocks/plans",request)).await
    }
    pub async fn cancel_stock_plan(&self,id:&str)->Result<shared_types::stocks::StockMarketSnapshot,ApiError>{
        super::with_mutation_timeout("取消股票本地预留",self.post_json("/api/stocks/plans/cancel",&shared_types::stocks::StockPlanCancelRequest{plan_id:id.into()})).await
    }
    pub async fn recheck_stock_order(&self,id:&str)->Result<shared_types::stocks::StockMarketSnapshot,ApiError>{
        super::timeout::with_mutation_timeout_ms("核对股票两腿回执",self.post_json("/api/stocks/plans/recheck",&shared_types::stocks::StockPlanCancelRequest{plan_id:id.into()}),40_000).await
    }
    pub async fn settle_stock_plan(&self, request:&shared_types::stocks::StockPlanRevisionRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError>{
        super::with_mutation_timeout("结束股票计划",self.post_json("/api/stocks/plans/settle",request)).await
    }
    pub async fn prepare_stock_topup(&self, request:&shared_types::stocks::StockPlanRevisionRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError>{
        super::timeout::with_mutation_timeout_ms("试算 SOL 补回",self.post_json("/api/stocks/plans/native-topup",request),34_000).await
    }
    pub async fn recheck_stock_topup(&self, request:&shared_types::stocks::StockTopupRecheckRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError>{
        super::with_mutation_timeout("核对 SOL 补回",self.post_json("/api/stocks/plans/native-topup/recheck",request)).await
    }
    pub async fn preflight_stock(&self,request:&shared_types::stocks::StockPreflightRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError>{
        super::with_mutation_timeout("股票库存与成本预检",self.post_json("/api/stocks/preflight",request)).await
    }
    pub async fn stock_deposit_address(&self,request:&shared_types::stocks::StockDepositAddressRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError>{
        super::with_mutation_timeout("读取 Backpack 充值地址",self.post_json("/api/stocks/funding/address",request)).await
    }
    pub async fn preview_stock_stablecoin(&self,request:&shared_types::stocks::StockStablecoinRequest)->Result<shared_types::stocks::StockStablecoinPreview,ApiError>{
        super::timeout::with_mutation_timeout_ms("试算 USDT 补充 USDC",self.post_json("/api/stocks/funding/stablecoin-preview",request),34_000).await
    }
    pub async fn build_stock_stablecoin_plan(&self,request:&shared_types::stocks::StockStablecoinPlanRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError>{
        super::with_mutation_timeout("保存稳定币兑换计划",self.post_json("/api/stocks/funding/stablecoin-plans",request)).await
    }
    pub async fn stock_stablecoin_plans(&self)->Result<shared_types::stocks::StockMarketSnapshot,ApiError>{
        self.get_json("/api/stocks/funding/stablecoin-plans").await
    }
    pub async fn cancel_stock_stablecoin_plan(&self,request:&shared_types::stocks::StockPlanRevisionRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError>{
        super::with_mutation_timeout("取消稳定币兑换预留",self.post_json("/api/stocks/funding/stablecoin-plans/cancel",request)).await
    }
    pub async fn submit_stock_stablecoin(&self,request:&shared_types::stocks::StockStablecoinSubmitRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError>{
        super::timeout::with_mutation_timeout_ms("提交原稳定币兑换计划",self.post_json("/api/stocks/funding/stablecoin-plans/submit",request),30_000).await
    }
    pub async fn recheck_stock_stablecoin(&self,request:&shared_types::stocks::StockPlanCancelRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError>{
        super::timeout::with_mutation_timeout_ms("核对原稳定币兑换回执",self.post_json("/api/stocks/funding/stablecoin-plans/recheck",request),22_000).await
    }
    pub async fn prepare_stock_stablecoin_topup(&self,request:&shared_types::stocks::StockPlanRevisionRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError>{
        super::timeout::with_mutation_timeout_ms("试算兑换后的 SOL 补回",self.post_json("/api/stocks/funding/stablecoin-plans/native-topup",request),34_000).await
    }
    pub async fn submit_stock_stablecoin_topup(&self,request:&shared_types::stocks::StockStablecoinTopupSubmitRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError>{
        super::timeout::with_mutation_timeout_ms("提交原 SOL 补回计划",self.post_json("/api/stocks/funding/stablecoin-plans/native-topup/submit",request),30_000).await
    }
    pub async fn cancel_stock_stablecoin_topup(&self,request:&shared_types::stocks::StockTopupRecheckRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError>{
        super::with_mutation_timeout("取消 SOL 补回预留",self.post_json("/api/stocks/funding/stablecoin-plans/native-topup/cancel",request)).await
    }
    pub async fn recheck_stock_stablecoin_topup(&self,request:&shared_types::stocks::StockTopupRecheckRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError>{
        super::timeout::with_mutation_timeout_ms("核对原 SOL 补回回执",self.post_json("/api/stocks/funding/stablecoin-plans/native-topup/recheck",request),22_000).await
    }
    pub async fn build_stock_funding_plan(&self,request:&shared_types::stocks::StockFundingPlanRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError>{
        super::timeout::with_mutation_timeout_ms("保存股票补库计划",self.post_json("/api/stocks/funding/plans",request),34_000).await
    }
    pub async fn cancel_stock_funding_plan(&self,request:&shared_types::stocks::StockPlanRevisionRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError>{
        super::with_mutation_timeout("取消股票补库预留",self.post_json("/api/stocks/funding/plans/cancel",request)).await
    }
    pub async fn submit_stock_funding(&self,request:&shared_types::stocks::StockFundingSubmitRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError>{
        super::timeout::with_mutation_timeout_ms("提交股票补库",self.post_json("/api/stocks/funding/plans/submit",request),50_000).await
    }
    pub async fn prepare_stock_funding_transfer(&self,request:&shared_types::stocks::StockPlanRevisionRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError>{
        super::timeout::with_mutation_timeout_ms("核算链上补库转账",self.post_json("/api/stocks/funding/plans/prepare-transfer",request),34_000).await
    }
    pub async fn recheck_stock_funding(&self,request:&shared_types::stocks::StockPlanCancelRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError>{
        super::timeout::with_mutation_timeout_ms("核对原股票补库",self.post_json("/api/stocks/funding/plans/recheck",request),32_000).await
    }
    pub async fn stock_chain_cost(&self, request: &shared_types::stocks::StockChainCostRequest) -> Result<shared_types::stocks::StockMarketSnapshot, ApiError> {
        super::with_mutation_timeout("股票链上费用试算", self.post_json("/api/stocks/chain-cost", request)).await
    }
    pub async fn request_stock_rfq(&self,request:&shared_types::stocks::StockRfqRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError>{
        super::with_mutation_timeout("股票 RFQ 询价",self.post_json("/api/stocks/rfq",request)).await
    }
    pub async fn recheck_stock_rfq(&self,id:&str)->Result<shared_types::stocks::StockMarketSnapshot,ApiError>{
        super::with_mutation_timeout("核对股票 RFQ",self.post_json("/api/stocks/rfq/recheck",&shared_types::stocks::StockRfqActionRequest{request_id:id.into()})).await
    }
    pub async fn finish_unsent_stock_rfq(&self,request:&shared_types::stocks::StockRfqRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError>{
        super::with_mutation_timeout("结束未发送询价",self.post_json("/api/stocks/rfq/finish-unsent",request)).await
    }
    pub async fn cancel_stock_rfq(&self,id:&str)->Result<shared_types::stocks::StockMarketSnapshot,ApiError>{
        super::with_mutation_timeout("取消股票 RFQ",self.post_json("/api/stocks/rfq/cancel",&shared_types::stocks::StockRfqActionRequest{request_id:id.into()})).await
    }
    pub async fn stock_catalog(&self) -> Result<shared_types::stocks::StockCatalog, ApiError> {
        super::with_mutation_timeout("读取股票目录", self.get_json("/api/stocks/catalog")).await
    }

    pub async fn stock_peer_markets(&self, request: &shared_types::stocks::StockPeerCatalogRequest) -> Result<shared_types::stocks::StockPeerCatalog,ApiError> {
        self.get_json(&format!("/api/stocks/peer-markets?venue={}&product={}&search={}",
            encode_query_component(&request.venue),request.product.as_str(),encode_query_component(&request.search))).await
    }

    pub async fn watch_stock_peer(&self, request: &shared_types::stocks::StockPeerWatchRequest) -> Result<shared_types::stocks::StockMarketSnapshot,ApiError> {
        super::with_mutation_timeout("选择股票对比市场",self.post_json("/api/stocks/peer",request)).await
    }

    pub async fn stock_peer_preflight(&self, request: &shared_types::stocks::StockPeerPreflightRequest) -> Result<shared_types::stocks::StockMarketSnapshot,ApiError> {
        // Account (12s) and optional wallet (8s) reads need response overhead.
        super::timeout::with_mutation_timeout_ms("检查股票账户与费用",self.post_json("/api/stocks/peer/preflight",request),24_000).await
    }

    pub async fn stock_peer_funding(&self, request: &shared_types::stocks::StockPeerFundingRequest) -> Result<shared_types::stocks::StockMarketSnapshot,ApiError> {
        super::timeout::with_mutation_timeout_ms("检查股票充提",self.post_json("/api/stocks/peer/funding",request),22_000).await
    }

    pub async fn stock_peer_order_check(&self, request: &shared_types::stocks::StockPeerOrderCheckRequest) -> Result<shared_types::stocks::StockMarketSnapshot,ApiError> {
        super::timeout::with_mutation_timeout_ms("验证股票订单（不成交）",self.post_json("/api/stocks/peer/order-check",request),14_000).await
    }

    pub async fn build_stock_peer_plan(&self, request: &shared_types::stocks::StockPeerPlanRequest) -> Result<shared_types::stocks::StockMarketSnapshot,ApiError> {
        super::timeout::with_mutation_timeout_ms("保存股票双边计划",self.post_json("/api/stocks/peer/plans",request),44_000).await
    }

    pub async fn stock_peer_plans(&self) -> Result<shared_types::stocks::StockMarketSnapshot,ApiError> {
        self.get_json("/api/stocks/peer/plans").await
    }

    pub async fn cancel_stock_peer_plan(&self, request: &shared_types::stocks::StockPlanRevisionRequest) -> Result<shared_types::stocks::StockMarketSnapshot,ApiError> {
        super::with_mutation_timeout("取消股票双边预留",self.post_json("/api/stocks/peer/plans/cancel",request)).await
    }
    pub async fn execute_stock_peer_plan(&self, request: &shared_types::stocks::StockPeerExecutionRequest) -> Result<shared_types::stocks::StockMarketSnapshot,ApiError> {
        super::timeout::with_mutation_timeout_ms("提交股票双边计划",self.post_json("/api/stocks/peer/plans/execute",request),29_000).await
    }
    pub async fn recheck_stock_peer_plan(&self, request: &shared_types::stocks::StockPlanRevisionRequest) -> Result<shared_types::stocks::StockMarketSnapshot,ApiError> {
        super::timeout::with_mutation_timeout_ms("核对原双边交易",self.post_json("/api/stocks/peer/plans/recheck",request),24_000).await
    }
    pub async fn prepare_stock_peer_recovery(&self,request:&shared_types::stocks::StockPeerRecoveryRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError> {
        super::timeout::with_mutation_timeout_ms("编制股票补偿",self.post_json("/api/stocks/peer/plans/recovery",request),44_000).await
    }
    pub async fn cancel_stock_peer_recovery(&self,request:&shared_types::stocks::StockRecoveryActionRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError> {
        super::with_mutation_timeout("取消补偿预留",self.post_json("/api/stocks/peer/plans/recovery/cancel",request)).await
    }
    pub async fn submit_stock_peer_recovery(&self,request:&shared_types::stocks::StockPeerRecoverySubmitRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError> {
        super::timeout::with_mutation_timeout_ms("提交股票补偿",self.post_json("/api/stocks/peer/plans/recovery/submit",request),29_000).await
    }
    pub async fn recheck_stock_peer_recovery(&self,request:&shared_types::stocks::StockRecoveryActionRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError> {
        super::timeout::with_mutation_timeout_ms("核对原补偿交易",self.post_json("/api/stocks/peer/plans/recovery/recheck",request),24_000).await
    }

    pub async fn prepare_stock_peer_conversion(&self,request:&shared_types::stocks::StockPeerConversionRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError> {
        super::timeout::with_mutation_timeout_ms("编制原币换汇",self.post_json("/api/stocks/peer/plans/conversion",request),24_000).await
    }
    pub async fn prepare_stock_peer_inventory(&self,r:&shared_types::stocks::StockPeerInventoryRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError> {
        super::timeout::with_mutation_timeout_ms("编制股票库存恢复",self.post_json("/api/stocks/peer/plans/inventory",r),24_000).await
    }
    pub async fn cancel_stock_peer_inventory(&self,r:&shared_types::stocks::StockRecoveryActionRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError> {
        super::with_mutation_timeout("取消库存恢复报价",self.post_json("/api/stocks/peer/plans/inventory/cancel",r)).await
    }
    pub async fn submit_stock_peer_inventory(&self,r:&shared_types::stocks::StockPeerRecoverySubmitRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError> {
        super::timeout::with_mutation_timeout_ms("提交股票库存恢复",self.post_json("/api/stocks/peer/plans/inventory/submit",r),24_000).await
    }
    pub async fn recheck_stock_peer_inventory(&self,r:&shared_types::stocks::StockRecoveryActionRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError> {
        super::timeout::with_mutation_timeout_ms("核对原库存恢复订单",self.post_json("/api/stocks/peer/plans/inventory/recheck",r),22_000).await
    }
    pub async fn cancel_stock_peer_conversion(&self,request:&shared_types::stocks::StockRecoveryActionRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError> {
        super::with_mutation_timeout("取消换汇报价",self.post_json("/api/stocks/peer/plans/conversion/cancel",request)).await
    }
    pub async fn submit_stock_peer_conversion(&self,request:&shared_types::stocks::StockPeerRecoverySubmitRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError> {
        super::timeout::with_mutation_timeout_ms("提交原币换汇",self.post_json("/api/stocks/peer/plans/conversion/submit",request),24_000).await
    }
    pub async fn recheck_stock_peer_conversion(&self,request:&shared_types::stocks::StockRecoveryActionRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError> {
        super::timeout::with_mutation_timeout_ms("核对原换汇订单",self.post_json("/api/stocks/peer/plans/conversion/recheck",request),22_000).await
    }

    pub async fn prepare_stock_peer_native_topup(&self,r:&shared_types::stocks::StockPeerNativeTopupRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError> {
        super::timeout::with_mutation_timeout_ms("编制 SOL 补回",self.post_json("/api/stocks/peer/plans/native-topup",r),44_000).await
    }
    pub async fn cancel_stock_peer_native_topup(&self,r:&shared_types::stocks::StockRecoveryActionRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError> {
        super::with_mutation_timeout("取消 SOL 补回",self.post_json("/api/stocks/peer/plans/native-topup/cancel",r)).await
    }
    pub async fn submit_stock_peer_native_topup(&self,r:&shared_types::stocks::StockPeerRecoverySubmitRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError> {
        super::timeout::with_mutation_timeout_ms("提交 SOL 补回",self.post_json("/api/stocks/peer/plans/native-topup/submit",r),29_000).await
    }
    pub async fn recheck_stock_peer_native_topup(&self,r:&shared_types::stocks::StockRecoveryActionRequest)->Result<shared_types::stocks::StockMarketSnapshot,ApiError> {
        super::timeout::with_mutation_timeout_ms("核对原 SOL 补回",self.post_json("/api/stocks/peer/plans/native-topup/recheck",r),24_000).await
    }

    pub async fn quote_stock(&self, request: &shared_types::stocks::StockQuoteRequest) -> Result<shared_types::stocks::StockMarketSnapshot, ApiError> {
        super::with_mutation_timeout("股票询价", self.post_json("/api/stocks/quote", request)).await
    }

    pub async fn monitor_stock(
        &self,
        request: &shared_types::stocks::StockMonitorRequest,
    ) -> Result<shared_types::stocks::StockMarketSnapshot, ApiError> {
        super::with_mutation_timeout("股票监控", self.post_json("/api/stocks/monitor", request)).await
    }

    pub async fn watch_stock(&self, asset: Option<String>) -> Result<shared_types::stocks::StockMarketSnapshot, ApiError> {
        super::with_mutation_timeout("选择股票", self.post_json("/api/stocks/watch", &shared_types::stocks::StockWatchRequest { asset })).await
    }

    pub async fn automation_status(&self) -> Result<AutomationRuntimeStatus, ApiError> {
        self.get_json("/api/automation/status").await
    }

    pub async fn gate_crossex_mode(&self) -> Result<GateCrossExModeSnapshot, ApiError> {
        self.get_json("/api/system/gate-crossex").await
    }

    pub async fn gate_crossex_routes(
        &self,
        search: &str,
    ) -> Result<GateCrossExRouteCatalogResponse, ApiError> {
        let search = encode_query_component(search);
        self.get_json(&format!(
            "/api/system/gate-crossex/routes?search={search}&limit=80"
        ))
        .await
    }

    pub async fn update_gate_crossex_mode(
        &self,
        patch: &GateCrossExModeConfigPatch,
    ) -> Result<GateCrossExModeSnapshot, ApiError> {
        let context =
            MutationRequestContext::new_idempotent_attempt("gate-crossex-mode-update".to_owned());
        self.patch_json_with_context("/api/system/gate-crossex/config", patch, &context)
            .await
    }

    pub async fn update_automation_config(
        &self,
        patch: &AutomatedArbitrageConfigPatch,
    ) -> Result<AutomationRuntimeStatus, ApiError> {
        self.patch_json("/api/automation/config", patch).await
    }

    pub async fn control_automation(
        &self,
        request: &AutomationControlRequest,
    ) -> Result<AutomationRuntimeStatus, ApiError> {
        self.post_json("/api/automation/control", request).await
    }

    pub async fn build_execution_artifact(
        &self,
        request: &ExecutionArtifactBuildRequest,
    ) -> Result<DeterministicExecutionArtifact, ApiError> {
        self.post_json("/api/automation/execution-artifacts/build", request)
            .await
    }

    pub async fn validate_execution_artifact(
        &self,
        request: &ExecutionArtifactValidationRequest,
    ) -> Result<ExecutionArtifactValidationResponse, ApiError> {
        self.post_json("/api/automation/execution-artifacts/validate", request)
            .await
    }

    pub async fn onchain_comparison(&self) -> Result<OnchainComparisonSnapshot, ApiError> {
        self.get_json("/api/onchain/comparison").await
    }

    pub async fn update_onchain_comparison(
        &self,
        patch: &OnchainComparisonConfigPatch,
    ) -> Result<OnchainComparisonSnapshot, ApiError> {
        self.patch_json("/api/onchain/comparison/config", patch)
            .await
    }

    pub async fn refresh_onchain_comparison(&self) -> Result<OnchainComparisonSnapshot, ApiError> {
        self.post_json("/api/onchain/comparison/refresh", &serde_json::json!({}))
            .await
    }

    pub async fn refresh_onchain_transfer_networks(
        &self,
    ) -> Result<OnchainComparisonSnapshot, ApiError> {
        self.post_json(
            "/api/onchain/transfer-networks/refresh",
            &serde_json::json!({}),
        )
        .await
    }

    pub async fn build_onchain_execution(
        &self,
        request: &OnchainExecutionBuildRequest,
    ) -> Result<OnchainExecutionBuildResponse, ApiError> {
        self.post_json("/api/onchain/execution/build", request)
            .await
    }

    pub async fn build_onchain_cross_chain_preview(
        &self,
        request: &OnchainCrossChainBuildRequest,
    ) -> Result<OnchainCrossChainBuildResponse, ApiError> {
        self.post_json("/api/onchain/cross-chain/build", request)
            .await
    }

    pub async fn authorize_onchain_cross_chain(
        &self,
        request: &OnchainCrossChainAuthorizeRequest,
    ) -> Result<OnchainCrossChainRun, ApiError> {
        self.post_json("/api/onchain/cross-chain/authorize", request)
            .await
    }

    pub async fn preview_onchain_cross_chain_recovery(
        &self, request: &OnchainCrossChainRecoveryPreviewRequest,
    ) -> Result<OnchainCrossChainRecoveryPreview, ApiError> {
        self.post_json("/api/onchain/cross-chain/recovery/preview", request).await
    }

    pub async fn reserve_onchain_cross_chain_recovery(&self, request: &OnchainCrossChainRecoveryAuthorizeRequest) -> Result<OnchainCrossChainRecoveryPlan, ApiError> {
        self.post_json("/api/onchain/cross-chain/recovery/reserve", request).await
    }

    pub async fn cancel_onchain_cross_chain_recovery(&self, plan_id: &str) -> Result<OnchainCrossChainRecoveryPlan, ApiError> {
        self.post_json("/api/onchain/cross-chain/recovery/cancel", &OnchainCrossChainRecoveryCancelRequest { plan_id: plan_id.into() }).await
    }

    pub async fn submit_onchain_cross_chain(
        &self,
        request: &OnchainCrossChainSubmitRequest,
    ) -> Result<OnchainCrossChainRun, ApiError> {
        self.post_json("/api/onchain/cross-chain/submit", request)
            .await
    }

    pub async fn onchain_cross_chain_runs(
        &self,
        limit: usize,
    ) -> Result<OnchainCrossChainRunsResponse, ApiError> {
        self.get_json(&format!(
            "/api/onchain/cross-chain/runs?limit={}",
            limit.clamp(1, 100)
        ))
        .await
    }

    pub async fn recheck_onchain_cross_chain(
        &self,
        request: &OnchainCrossChainRecheckRequest,
    ) -> Result<OnchainCrossChainRun, ApiError> {
        self.post_json("/api/onchain/cross-chain/recheck", request).await
    }

    pub async fn submit_onchain_execution(
        &self,
        request: &OnchainExecutionSubmitRequest,
    ) -> Result<OnchainExecutionSubmitResponse, ApiError> {
        self.post_json("/api/onchain/execution/submit", request)
            .await
    }

    pub async fn onchain_execution_runs(
        &self,
        limit: usize,
    ) -> Result<OnchainExecutionRunsResponse, ApiError> {
        self.get_json(&format!(
            "/api/onchain/execution/runs?limit={}",
            limit.clamp(1, 100)
        ))
        .await
    }

    pub async fn build_onchain_replenishment(
        &self,
        request: &OnchainReplenishmentBuildRequest,
    ) -> Result<OnchainReplenishmentPlanResponse, ApiError> {
        self.post_json("/api/onchain/replenishment/build", request)
            .await
    }

    pub async fn authorize_onchain_replenishment(
        &self,
        request: &OnchainReplenishmentAuthorizeRequest,
    ) -> Result<OnchainReplenishmentRun, ApiError> {
        self.post_json("/api/onchain/replenishment/authorize", request)
            .await
    }

    pub async fn submit_onchain_replenishment(
        &self,
        request: &OnchainReplenishmentSubmitRequest,
    ) -> Result<OnchainReplenishmentRun, ApiError> {
        self.post_json("/api/onchain/replenishment/submit", request)
            .await
    }

    pub async fn recheck_onchain_replenishment(
        &self,
        request: &shared_types::OnchainReplenishmentRecheckRequest,
    ) -> Result<shared_types::OnchainReplenishmentRun, super::ApiError> {
        self.post_json("/api/onchain/replenishment/recheck", request).await
    }

    pub async fn onchain_replenishment_runs(
        &self,
        limit: usize,
    ) -> Result<OnchainReplenishmentRunsResponse, ApiError> {
        self.get_json(&format!(
            "/api/onchain/replenishment/runs?limit={}",
            limit.clamp(1, 100)
        ))
        .await
    }

    pub async fn build_onchain_token_approval(
        &self,
        request: &OnchainTokenApprovalBuildRequest,
    ) -> Result<OnchainTokenApprovalBuildResponse, ApiError> {
        self.post_json("/api/onchain/token-approval/build", request)
            .await
    }

    pub async fn submit_onchain_token_approval(
        &self,
        request: &OnchainTokenApprovalSubmitRequest,
    ) -> Result<OnchainTokenApprovalSubmitResponse, ApiError> {
        self.post_json("/api/onchain/token-approval/submit", request)
            .await
    }

    pub async fn onchain_token_approval_runs(
        &self,
        limit: usize,
    ) -> Result<OnchainTokenApprovalRunsResponse, ApiError> {
        self.get_json(&format!(
            "/api/onchain/token-approval/runs?limit={}",
            limit.clamp(1, 100)
        ))
        .await
    }

    pub async fn onchain_batch(&self) -> Result<OnchainBatchSnapshot, ApiError> {
        self.get_json("/api/onchain/comparison/batch").await
    }

    pub async fn add_onchain_batch(
        &self,
        patch: &OnchainComparisonConfigPatch,
    ) -> Result<OnchainBatchSnapshot, ApiError> {
        self.post_json("/api/onchain/comparison/batch", patch).await
    }

    pub async fn remove_onchain_batch(
        &self,
        item_id: String,
    ) -> Result<OnchainBatchSnapshot, ApiError> {
        self.post_json(
            "/api/onchain/comparison/batch/remove",
            &OnchainBatchRemoveRequest { item_id },
        )
        .await
    }

    pub async fn resolve_onchain_token(
        &self,
        request: &OnchainTokenIdentityRequest,
    ) -> Result<OnchainTokenResolution, ApiError> {
        self.post_json_quiet("/api/onchain/token/resolve", request)
            .await
    }

    pub async fn onchain_cex_pairs(
        &self,
        venue: &str,
        base_token: &str,
    ) -> Result<OnchainCexPairCatalog, ApiError> {
        let venue = encode_query_component(venue);
        let base_token = encode_query_component(base_token);
        self.get_json(&format!(
            "/api/onchain/cex-pairs?venue={venue}&baseToken={base_token}"
        ))
        .await
    }

    pub async fn onchain_provider_credentials(
        &self,
    ) -> Result<OnchainProviderCredentialsResponse, ApiError> {
        self.get_json("/api/onchain/credentials").await
    }

    pub async fn save_onchain_provider_credentials(
        &self,
        provider: &str,
        fields: Vec<VenueCredentialValue>,
    ) -> Result<OnchainProviderCredentialMutationResponse, ApiError> {
        let request = OnchainProviderCredentialUpdateRequest {
            provider: provider.to_owned(),
            fields,
        };
        let context = MutationRequestContext::new_idempotent_attempt(format!(
            "onchain-provider-credentials-save-{provider}"
        ));
        self.post_json_with_context("/api/onchain/credentials", &request, &context)
            .await
    }

    pub async fn clear_onchain_provider_credentials(
        &self,
        provider: &str,
        fields: Vec<String>,
    ) -> Result<OnchainProviderCredentialMutationResponse, ApiError> {
        let request = OnchainProviderCredentialClearRequest {
            provider: provider.to_owned(),
            fields,
        };
        let context = MutationRequestContext::new_idempotent_attempt(format!(
            "onchain-provider-credentials-clear-{provider}"
        ));
        self.post_json_with_context("/api/onchain/credentials/clear", &request, &context)
            .await
    }

    pub async fn webhook_status(&self) -> Result<WebhookRuntimeStatus, ApiError> {
        self.get_json("/api/webhook/status").await
    }

    pub async fn update_webhook_config(
        &self,
        patch: &WebhookConfigPatch,
    ) -> Result<WebhookRuntimeStatus, ApiError> {
        self.patch_json("/api/webhook/config", patch).await
    }

    pub async fn test_webhook(&self, request: &WebhookTestRequest) -> Result<(), ApiError> {
        self.post_json::<_, serde_json::Value>("/api/webhook/test", request)
            .await
            .map(|_| ())
    }
}
