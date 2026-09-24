//! API router registry.

use std::collections::BTreeSet;

use crate::routers;
use crate::state::AppState;
use axum::Router;
use common::config::ApiSurfaceConfig;
use shared_types::ActionRunKind;

pub(crate) type RouterFactory = fn() -> Router<AppState>;
type SeededEndpointSummary = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    Option<ActionRunKind>,
);

#[cfg(not(feature = "legacy-chat"))]
const CHAT_REQUESTED_BUT_UNCOMPILED: &str = "api_surface.chat:requested_but_uncompiled";
#[cfg(not(feature = "legacy-chat"))]
const LLM_REQUESTED_BUT_UNCOMPILED: &str = "api_surface.llm_diagnostics:requested_but_uncompiled";
#[cfg(not(feature = "legacy-options"))]
const OPTIONS_REQUESTED_BUT_UNCOMPILED: &str = "api_surface.options:requested_but_uncompiled";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RouteGate {
    Core,
    #[cfg(feature = "legacy-chat")]
    Chat,
    #[cfg(feature = "legacy-chat")]
    LlmDiagnostics,
    #[cfg(feature = "legacy-options")]
    Options,
    WatchlistAlerts,
    SpotV1,
    StrategyV1,
}

#[derive(Clone, Copy)]
pub(crate) struct RouteSpec {
    router_key: &'static str,
    gate: RouteGate,
    build: RouterFactory,
    endpoints: &'static [RouteEndpointSpec],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RouteEndpointSpec {
    path: &'static str,
    methods: &'static str,
    class: &'static str,
    default_exposure: &'static str,
    risk: &'static str,
    auth_policy: &'static str,
    audit_policy: &'static str,
    action_run_kind: Option<ActionRunKind>,
}

impl RouteSpec {
    const fn with_endpoints(
        router_key: &'static str,
        gate: RouteGate,
        build: RouterFactory,
        endpoints: &'static [RouteEndpointSpec],
    ) -> Self {
        Self {
            router_key,
            gate,
            build,
            endpoints,
        }
    }

    pub(crate) const fn router_key(self) -> &'static str {
        self.router_key
    }

    pub(crate) const fn feature_flag(self) -> &'static str {
        self.gate.feature_flag()
    }

    pub(crate) const fn endpoints(self) -> &'static [RouteEndpointSpec] {
        self.endpoints
    }

    fn enabled(self, surface: &ApiSurfaceConfig) -> bool {
        self.gate.enabled(surface)
    }

    fn router(self) -> Router<AppState> {
        (self.build)()
    }
}

impl RouteEndpointSpec {
    const fn post(
        path: &'static str,
        class: &'static str,
        default_exposure: &'static str,
        risk: &'static str,
        auth_policy: &'static str,
        audit_policy: &'static str,
    ) -> Self {
        Self {
            path,
            methods: "POST",
            class,
            default_exposure,
            risk,
            auth_policy,
            audit_policy,
            action_run_kind: None,
        }
    }

    const fn get(
        path: &'static str,
        class: &'static str,
        default_exposure: &'static str,
        risk: &'static str,
        auth_policy: &'static str,
        audit_policy: &'static str,
    ) -> Self {
        Self {
            path,
            methods: "GET",
            class,
            default_exposure,
            risk,
            auth_policy,
            audit_policy,
            action_run_kind: None,
        }
    }

    const fn delete(
        path: &'static str,
        class: &'static str,
        default_exposure: &'static str,
        risk: &'static str,
        auth_policy: &'static str,
        audit_policy: &'static str,
    ) -> Self {
        Self {
            path,
            methods: "DELETE",
            class,
            default_exposure,
            risk,
            auth_policy,
            audit_policy,
            action_run_kind: None,
        }
    }

    const fn post_action_run(
        path: &'static str,
        class: &'static str,
        default_exposure: &'static str,
        risk: &'static str,
        audit_policy: &'static str,
        action_run_kind: ActionRunKind,
    ) -> Self {
        Self {
            path,
            methods: "POST",
            class,
            default_exposure,
            risk,
            auth_policy: "bearer",
            audit_policy,
            action_run_kind: Some(action_run_kind),
        }
    }

    const fn patch_action_run(
        path: &'static str,
        class: &'static str,
        default_exposure: &'static str,
        risk: &'static str,
        audit_policy: &'static str,
        action_run_kind: ActionRunKind,
    ) -> Self {
        Self {
            path,
            methods: "PATCH",
            class,
            default_exposure,
            risk,
            auth_policy: "bearer",
            audit_policy,
            action_run_kind: Some(action_run_kind),
        }
    }

    pub(crate) const fn path(self) -> &'static str {
        self.path
    }

    pub(crate) const fn methods(self) -> &'static str {
        self.methods
    }

    pub(crate) const fn class(self) -> &'static str {
        self.class
    }

    pub(crate) const fn default_exposure(self) -> &'static str {
        self.default_exposure
    }

    pub(crate) const fn risk(self) -> &'static str {
        self.risk
    }

    pub(crate) const fn auth_policy(self) -> &'static str {
        self.auth_policy
    }

    pub(crate) const fn audit_policy(self) -> &'static str {
        self.audit_policy
    }

    pub(crate) const fn action_run_kind(self) -> Option<ActionRunKind> {
        self.action_run_kind
    }
}

impl RouteGate {
    const fn feature_flag(self) -> &'static str {
        match self {
            Self::Core => "core",
            #[cfg(feature = "legacy-chat")]
            Self::Chat => "api_surface.chat",
            #[cfg(feature = "legacy-chat")]
            Self::LlmDiagnostics => "api_surface.llm_diagnostics",
            #[cfg(feature = "legacy-options")]
            Self::Options => "api_surface.options",
            Self::WatchlistAlerts => "api_surface.watchlist_alerts",
            Self::SpotV1 => "api_surface.spot_v1",
            Self::StrategyV1 => "api_surface.strategy_v1",
        }
    }

    fn enabled(self, surface: &ApiSurfaceConfig) -> bool {
        match self {
            Self::Core => true,
            #[cfg(feature = "legacy-chat")]
            Self::Chat => surface.chat,
            #[cfg(feature = "legacy-chat")]
            Self::LlmDiagnostics => surface.llm_diagnostics,
            #[cfg(feature = "legacy-options")]
            Self::Options => surface.options,
            Self::WatchlistAlerts => surface.watchlist_alerts,
            Self::SpotV1 => surface.spot_v1,
            Self::StrategyV1 => surface.strategy_v1,
        }
    }
}

const HEALTH_ENDPOINTS: &[RouteEndpointSpec] = &[
    RouteEndpointSpec::get(
        "/health",
        "liveness",
        "always",
        "low",
        "public_liveness",
        "read",
    ),
    RouteEndpointSpec::get(
        "/health/ready",
        "readiness",
        "always",
        "medium",
        "bearer",
        "read",
    ),
];

const METRICS_ENDPOINTS: &[RouteEndpointSpec] = &[RouteEndpointSpec::get(
    "/metrics",
    "diagnostic",
    "always",
    "medium",
    "scrape_bearer",
    "metrics",
)];

const ONCHAIN_ENDPOINTS: &[RouteEndpointSpec] = &[
    RouteEndpointSpec::get(
        "/api/onchain/credentials",
        "main_p0",
        "always",
        "medium",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::post_action_run(
        "/api/onchain/credentials",
        "main_p0",
        "always",
        "high",
        "secret_mutation",
        ActionRunKind::OnchainProviderCredentialsUpdate,
    ),
    RouteEndpointSpec::post_action_run(
        "/api/onchain/credentials/clear",
        "main_p0",
        "always",
        "high",
        "secret_mutation",
        ActionRunKind::OnchainProviderCredentialsClear,
    ),
    RouteEndpointSpec::get(
        "/api/onchain/comparison",
        "main_p0",
        "always",
        "low",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::get(
        "/api/onchain/comparison/batch",
        "main_p0",
        "always",
        "low",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::post(
        "/api/onchain/comparison/batch",
        "main_p0",
        "always",
        "medium",
        "bearer",
        "required",
    ),
    RouteEndpointSpec::post(
        "/api/onchain/comparison/batch/remove",
        "main_p0",
        "always",
        "medium",
        "bearer",
        "required",
    ),
    RouteEndpointSpec::get(
        "/api/onchain/cex-pairs",
        "main_p0",
        "always",
        "low",
        "bearer",
        "read",
    ),
    RouteEndpointSpec {
        path: "/api/onchain/comparison/config",
        methods: "PATCH",
        class: "main_p0",
        default_exposure: "always",
        risk: "medium",
        auth_policy: "bearer",
        audit_policy: "required",
        action_run_kind: None,
    },
    RouteEndpointSpec::post(
        "/api/onchain/comparison/refresh",
        "main_p0",
        "always",
        "low",
        "bearer",
        "required",
    ),
    RouteEndpointSpec::post(
        "/api/onchain/transfer-networks/refresh",
        "main_p0",
        "always",
        "low",
        "bearer",
        "required",
    ),
    RouteEndpointSpec::post(
        "/api/onchain/cross-chain/build",
        "main_p0",
        "always",
        "medium",
        "bearer",
        "required",
    ),
    RouteEndpointSpec::post(
        "/api/onchain/cross-chain/authorize",
        "main_p0",
        "always",
        "high",
        "bearer",
        "required",
    ),
    RouteEndpointSpec::get(
        "/api/onchain/cross-chain/runs",
        "main_p0",
        "always",
        "medium",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::post(
        "/api/onchain/cross-chain/submit",
        "main_p0",
        "always",
        "high",
        "bearer",
        "required",
    ),
    RouteEndpointSpec::post(
        "/api/onchain/cross-chain/recheck",
        "main_p0",
        "always",
        "low",
        "bearer",
        "required",
    ),
    RouteEndpointSpec::post(
        "/api/onchain/execution/build",
        "main_p0",
        "always",
        "high",
        "bearer",
        "required",
    ),
    RouteEndpointSpec::post(
        "/api/onchain/execution/submit",
        "main_p0",
        "always",
        "high",
        "bearer",
        "required",
    ),
    RouteEndpointSpec::get(
        "/api/onchain/execution/runs",
        "main_p0",
        "always",
        "medium",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::post(
        "/api/onchain/replenishment/build",
        "main_p0",
        "always",
        "high",
        "bearer",
        "required",
    ),
    RouteEndpointSpec::post(
        "/api/onchain/replenishment/authorize",
        "main_p0",
        "always",
        "high",
        "bearer",
        "required",
    ),
    RouteEndpointSpec::post(
        "/api/onchain/replenishment/submit",
        "main_p0",
        "always",
        "high",
        "bearer",
        "required",
    ),
    RouteEndpointSpec::get(
        "/api/onchain/replenishment/plans",
        "main_p0",
        "always",
        "medium",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::get(
        "/api/onchain/replenishment/runs",
        "main_p0",
        "always",
        "medium",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::post(
        "/api/onchain/replenishment/recheck",
        "main_p0",
        "always",
        "low",
        "bearer",
        "required",
    ),
    RouteEndpointSpec::post(
        "/api/onchain/token-approval/build",
        "main_p0",
        "always",
        "high",
        "bearer",
        "required",
    ),
    RouteEndpointSpec::post(
        "/api/onchain/token-approval/submit",
        "main_p0",
        "always",
        "high",
        "bearer",
        "required",
    ),
    RouteEndpointSpec::get(
        "/api/onchain/token-approval/runs",
        "main_p0",
        "always",
        "medium",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::post(
        "/api/onchain/token/resolve",
        "main_p0",
        "always",
        "low",
        "bearer",
        "required",
    ),
];

const STOCK_ENDPOINTS: &[RouteEndpointSpec] = &[
    RouteEndpointSpec::get("/api/stocks/catalog", "main_p0", "always", "low", "bearer", "read"),
    RouteEndpointSpec::get("/api/stocks/peer-markets", "main_p0", "always", "low", "bearer", "read"),
    RouteEndpointSpec::post("/api/stocks/peer", "main_p0", "always", "low", "bearer", "required"),
    RouteEndpointSpec::post("/api/stocks/peer/preflight", "main_p0", "always", "low", "bearer", "read"),
    RouteEndpointSpec::post("/api/stocks/peer/funding", "main_p0", "always", "low", "bearer", "read"),
    RouteEndpointSpec::post("/api/stocks/peer/order-check", "main_p0", "always", "low", "bearer", "read"),
    RouteEndpointSpec::get("/api/stocks/peer/plans", "main_p0", "always", "low", "bearer", "read"),
    RouteEndpointSpec::post("/api/stocks/peer/plans", "main_p0", "always", "low", "bearer", "required"),
    RouteEndpointSpec::post("/api/stocks/peer/plans/cancel", "main_p0", "always", "low", "bearer", "required"),
    RouteEndpointSpec::post("/api/stocks/peer/plans/execute", "main_p0", "always", "low", "bearer", "required"),
    RouteEndpointSpec::post("/api/stocks/peer/plans/recheck", "main_p0", "always", "low", "bearer", "required"),
    RouteEndpointSpec::post("/api/stocks/watch", "main_p0", "always", "low", "bearer", "required"),
    RouteEndpointSpec::post("/api/stocks/quote", "main_p0", "always", "low", "bearer", "read"),
    RouteEndpointSpec::post("/api/stocks/monitor", "main_p0", "always", "low", "bearer", "required"),
    RouteEndpointSpec::post("/api/stocks/rfq", "main_p0", "always", "medium", "bearer", "required"),
    RouteEndpointSpec::post("/api/stocks/rfq/recheck", "main_p0", "always", "low", "bearer", "required"),
    RouteEndpointSpec::post("/api/stocks/rfq/finish-unsent", "main_p0", "always", "low", "bearer", "required"),
    RouteEndpointSpec::post("/api/stocks/rfq/cancel", "main_p0", "always", "medium", "bearer", "required"),
    RouteEndpointSpec::post("/api/stocks/preflight", "main_p0", "always", "low", "bearer", "required"),
    RouteEndpointSpec::post("/api/stocks/funding/address", "main_p0", "always", "low", "bearer", "required"),
    RouteEndpointSpec::post("/api/stocks/funding/plans", "main_p0", "always", "low", "bearer", "required"),
    RouteEndpointSpec::post("/api/stocks/funding/plans/cancel", "main_p0", "always", "low", "bearer", "required"),
    RouteEndpointSpec::post("/api/stocks/funding/plans/prepare-transfer", "main_p0", "always", "low", "bearer", "required"),
    RouteEndpointSpec::post("/api/stocks/funding/plans/submit", "main_p0", "always", "low", "bearer", "required"),
    RouteEndpointSpec::post("/api/stocks/funding/plans/recheck", "main_p0", "always", "low", "bearer", "required"),
    RouteEndpointSpec::post("/api/stocks/chain-cost", "main_p0", "always", "low", "bearer", "required"),
    RouteEndpointSpec::post("/api/stocks/plans", "main_p0", "always", "low", "bearer", "required"),
    RouteEndpointSpec::post("/api/stocks/plans/execute", "main_p0", "always", "low", "bearer", "required"),
    RouteEndpointSpec::post("/api/stocks/plans/cancel", "main_p0", "always", "low", "bearer", "required"),
    RouteEndpointSpec::post("/api/stocks/plans/recheck", "main_p0", "always", "low", "bearer", "required"),
    RouteEndpointSpec::post("/api/stocks/plans/settle", "main_p0", "always", "low", "bearer", "required"),
    RouteEndpointSpec::post("/api/stocks/plans/native-topup", "main_p0", "always", "low", "bearer", "required"),
    RouteEndpointSpec::post("/api/stocks/plans/native-topup/recheck", "main_p0", "always", "low", "bearer", "required"),
    RouteEndpointSpec::post("/api/stocks/peer/plans/native-topup", "main_p0", "always", "low", "bearer", "required"),
    RouteEndpointSpec::post("/api/stocks/peer/plans/inventory", "main_p0", "always", "low", "bearer", "required"),
    RouteEndpointSpec::post("/api/stocks/peer/plans/inventory/cancel", "main_p0", "always", "low", "bearer", "required"),
    RouteEndpointSpec::post("/api/stocks/peer/plans/inventory/submit", "main_p0", "always", "high", "bearer", "required"),
    RouteEndpointSpec::post("/api/stocks/peer/plans/inventory/recheck", "main_p0", "always", "low", "bearer", "required"),
    RouteEndpointSpec::post("/api/stocks/peer/plans/native-topup/cancel", "main_p0", "always", "low", "bearer", "required"),
    RouteEndpointSpec::post("/api/stocks/peer/plans/native-topup/submit", "main_p0", "always", "low", "bearer", "required"),
    RouteEndpointSpec::post("/api/stocks/peer/plans/native-topup/recheck", "main_p0", "always", "low", "bearer", "required"),
    RouteEndpointSpec::post("/api/stocks/plans/recovery", "main_p0", "always", "low", "bearer", "required"),
    RouteEndpointSpec::post("/api/stocks/plans/recovery/cancel", "main_p0", "always", "low", "bearer", "required"),
    RouteEndpointSpec::post("/api/stocks/plans/recovery/recheck", "main_p0", "always", "low", "bearer", "required"),
];

const AUTH_ENDPOINTS: &[RouteEndpointSpec] = &[RouteEndpointSpec::post(
    "/api/auth/ws-ticket",
    "main_p0",
    "always",
    "medium",
    "bearer",
    "read",
)];

const ARBITRAGE_ENDPOINTS: &[RouteEndpointSpec] = &[
    RouteEndpointSpec::get(
        "/api/arbitrage/funding-rates",
        "main_p0",
        "always",
        "low",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::post_action_run(
        "/api/arbitrage/opportunities/:id/confirm",
        "main_p0",
        "always",
        "high",
        "action_run",
        ActionRunKind::HedgeConfirm,
    ),
    RouteEndpointSpec::post(
        "/api/arbitrage/opportunities/:id/preview",
        "main_p0",
        "always",
        "medium",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::get(
        "/api/v3/arbitrage/opportunities",
        "legacy",
        "always",
        "low",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::get(
        "/api/v3/arbitrage/opportunities/:id",
        "main_p0",
        "always",
        "low",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::get(
        "/api/v3/arbitrage/opportunities/:id/detail",
        "main_p0",
        "always",
        "low",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::get(
        "/api/v3/arbitrage/opportunities/list",
        "main_p0",
        "always",
        "low",
        "bearer",
        "read",
    ),
];

const AUTOMATION_ENDPOINTS: &[RouteEndpointSpec] = &[
    RouteEndpointSpec::get(
        "/api/automation/execution-runs/:run_id",
        "main_p0",
        "always",
        "medium",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::get(
        "/api/automation/status",
        "main_p0",
        "always",
        "medium",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::patch_action_run(
        "/api/automation/config",
        "main_p0",
        "always",
        "high",
        "action_run",
        ActionRunKind::AutomationConfigUpdate,
    ),
    RouteEndpointSpec::post_action_run(
        "/api/automation/control",
        "main_p0",
        "always",
        "high",
        "action_run",
        ActionRunKind::AutomationControl,
    ),
    RouteEndpointSpec::post(
        "/api/automation/execution-artifacts/build",
        "main_p0",
        "always",
        "medium",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::post(
        "/api/automation/execution-artifacts/validate",
        "main_p0",
        "always",
        "low",
        "bearer",
        "read",
    ),
];

const EXCHANGES_ENDPOINTS: &[RouteEndpointSpec] = &[
    RouteEndpointSpec::get(
        "/api/exchanges/:venue/orderbook",
        "main_p0",
        "always",
        "medium",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::get(
        "/api/exchanges/credentials",
        "main_p0",
        "always",
        "medium",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::post_action_run(
        "/api/exchanges/credentials",
        "main_p0",
        "always",
        "high",
        "secret_mutation",
        ActionRunKind::VenueCredentialsUpdate,
    ),
    RouteEndpointSpec::post_action_run(
        "/api/exchanges/credentials/clear",
        "main_p0",
        "always",
        "high",
        "secret_mutation",
        ActionRunKind::VenueCredentialsClear,
    ),
    RouteEndpointSpec::post_action_run(
        "/api/exchanges/credentials/migrate",
        "main_p0",
        "always",
        "high",
        "secret_mutation",
        ActionRunKind::VenueCredentialsMigrate,
    ),
];

const HISTORY_ENDPOINTS: &[RouteEndpointSpec] = &[
    RouteEndpointSpec::get(
        "/api/history/funding",
        "main_p0",
        "always",
        "low",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::get(
        "/api/history/funding-diff-stats",
        "main_p0",
        "always",
        "low",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::get(
        "/api/history/funding-diffs",
        "main_p0",
        "always",
        "low",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::get(
        "/api/history/index-compositions",
        "main_p0",
        "always",
        "low",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::get(
        "/api/history/opportunities",
        "main_p0",
        "always",
        "low",
        "bearer",
        "read",
    ),
];

const SYSTEM_ENDPOINTS: &[RouteEndpointSpec] = &[
    RouteEndpointSpec::get(
        "/api/system/health",
        "main_p0",
        "always",
        "low",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::get(
        "/api/system/market-data-diagnostics",
        "diagnostic",
        "always",
        "low",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::get(
        "/api/system/market-subscriptions",
        "main_p0",
        "always",
        "low",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::patch_action_run(
        "/api/system/market-subscriptions/config",
        "main_p0",
        "always",
        "medium",
        "required",
        ActionRunKind::MarketSubscriptionsUpdate,
    ),
    RouteEndpointSpec::get(
        "/api/system/gate-crossex",
        "main_p0",
        "always",
        "low",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::get(
        "/api/system/gate-crossex/routes",
        "main_p0",
        "always",
        "low",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::patch_action_run(
        "/api/system/gate-crossex/config",
        "main_p0",
        "always",
        "medium",
        "required",
        ActionRunKind::GateCrossExModeUpdate,
    ),
    RouteEndpointSpec::get(
        "/api/system/venue-operation-health",
        "diagnostic",
        "always",
        "low",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::get(
        "/api/system/venue-runtime-health",
        "diagnostic",
        "always",
        "low",
        "bearer",
        "read",
    ),
];

const WEBSOCKET_ENDPOINTS: &[RouteEndpointSpec] = &[RouteEndpointSpec::get(
    "/ws",
    "main_p0",
    "always",
    "medium",
    "bearer_ws",
    "read",
)];

const WEBHOOK_ENDPOINTS: &[RouteEndpointSpec] = &[
    RouteEndpointSpec::get(
        "/api/webhook/status",
        "main_p0",
        "always",
        "low",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::patch_action_run(
        "/api/webhook/config",
        "main_p0",
        "always",
        "high",
        "secret_mutation",
        ActionRunKind::WebhookConfigUpdate,
    ),
    RouteEndpointSpec::post(
        "/api/webhook/test",
        "main_p0",
        "always",
        "medium",
        "bearer",
        "required",
    ),
];

const PORTFOLIO_ENDPOINTS: &[RouteEndpointSpec] = &[
    RouteEndpointSpec::post_action_run(
        "/api/trading/portfolio/close-all",
        "main_p0",
        "always",
        "high",
        "action_run",
        ActionRunKind::PortfolioCloseAll,
    ),
    RouteEndpointSpec::post_action_run(
        "/api/trading/portfolio/close-runs/:close_run_id/compensation-orders",
        "main_p0",
        "always",
        "high",
        "action_run",
        ActionRunKind::PortfolioCloseCompensation,
    ),
    RouteEndpointSpec::post_action_run(
        "/api/trading/portfolio/close-runs/:close_run_id/manual-terminal",
        "main_p0",
        "always",
        "high",
        "action_run",
        ActionRunKind::PortfolioCloseManualTerminal,
    ),
    RouteEndpointSpec::post_action_run(
        "/api/trading/portfolio/positions/:venue/:symbol/close",
        "main_p0",
        "always",
        "high",
        "action_run",
        ActionRunKind::PortfolioClosePosition,
    ),
    RouteEndpointSpec::post_action_run(
        "/api/trading/portfolio/positions/:venue/:symbol/close-pair",
        "main_p0",
        "always",
        "high",
        "action_run",
        ActionRunKind::PortfolioClosePair,
    ),
    RouteEndpointSpec::get(
        "/api/trading/portfolio/nav-history",
        "main_p0",
        "always",
        "low",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::get(
        "/api/trading/portfolio/snapshot",
        "main_p0",
        "always",
        "medium",
        "bearer",
        "read",
    ),
];

const REVIEW_ENDPOINTS: &[RouteEndpointSpec] = &[
    RouteEndpointSpec::get(
        "/api/review/executed",
        "main_p0",
        "always",
        "low",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::get(
        "/api/review/missed",
        "main_p0",
        "always",
        "low",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::get(
        "/api/review/strategy-performance",
        "main_p0",
        "always",
        "low",
        "bearer",
        "read",
    ),
];

const TRADING_ENDPOINTS: &[RouteEndpointSpec] = &[
    RouteEndpointSpec::get(
        "/api/trading/action-runs",
        "main_p0",
        "always",
        "low",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::get(
        "/api/trading/action-runs/:id",
        "main_p0",
        "always",
        "low",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::get(
        "/api/trading/adapters",
        "main_p0",
        "always",
        "low",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::post_action_run(
        "/api/trading/adapters/select",
        "main_p0",
        "always",
        "high",
        "action_run",
        ActionRunKind::TradingAdapterSelect,
    ),
    RouteEndpointSpec::get(
        "/api/trading/account-state",
        "main_p0",
        "always",
        "medium",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::get(
        "/api/trading/balances",
        "main_p0",
        "always",
        "medium",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::get(
        "/api/trading/credentials/env-template",
        "main_p0",
        "always",
        "medium",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::get(
        "/api/trading/execution-ledger",
        "main_p0",
        "always",
        "low",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::get(
        "/api/trading/execution-runs",
        "main_p0",
        "always",
        "low",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::post_action_run(
        "/api/trading/fee-snapshots",
        "main_p0",
        "always",
        "high",
        "action_run",
        ActionRunKind::TradingFeeSnapshotUpsert,
    ),
    RouteEndpointSpec::get(
        "/api/trading/fee-schedules",
        "main_p0",
        "always",
        "low",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::post_action_run(
        "/api/trading/kill-switch",
        "main_p0",
        "always",
        "high",
        "action_run",
        ActionRunKind::TradingKillSwitch,
    ),
    RouteEndpointSpec::get(
        "/api/trading/orders",
        "main_p0",
        "always",
        "medium",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::post_action_run(
        "/api/trading/orders",
        "main_p0",
        "always",
        "high",
        "action_run",
        ActionRunKind::TradingOrderSubmit,
    ),
    RouteEndpointSpec::get(
        "/api/trading/orders/:id",
        "main_p0",
        "always",
        "medium",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::post_action_run(
        "/api/trading/orders/:id/cancel",
        "main_p0",
        "always",
        "high",
        "action_run",
        ActionRunKind::TradingOrderCancel,
    ),
    RouteEndpointSpec::post_action_run(
        "/api/trading/orders/reconcile",
        "main_p0",
        "always",
        "high",
        "action_run",
        ActionRunKind::TradingOrderReconcile,
    ),
    RouteEndpointSpec::get(
        "/api/trading/positions",
        "main_p0",
        "always",
        "medium",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::patch_action_run(
        "/api/trading/risk-config",
        "main_p0",
        "always",
        "high",
        "action_run",
        ActionRunKind::TradingRiskConfigUpdate,
    ),
    RouteEndpointSpec::get(
        "/api/trading/status",
        "main_p0",
        "always",
        "low",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::get(
        "/api/trading/ws/venues",
        "main_p0",
        "always",
        "low",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::get(
        "/api/trading/ws/operations",
        "main_p0",
        "always",
        "low",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::get(
        "/api/trading/rest/endpoints",
        "main_p0",
        "always",
        "low",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::get(
        "/api/trading/transport/registry",
        "main_p0",
        "always",
        "low",
        "bearer",
        "read",
    ),
];

const VENUES_ENDPOINTS: &[RouteEndpointSpec] = &[
    RouteEndpointSpec::get(
        "/api/venues/instrument-coverage",
        "main_p0",
        "always",
        "low",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::get(
        "/api/trading/venues/quality",
        "main_p0",
        "always",
        "low",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::get(
        "/api/venues/index-compositions",
        "diagnostic",
        "always",
        "medium",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::post(
        "/api/venues/index-compositions",
        "diagnostic",
        "always",
        "high",
        "bearer",
        "required",
    ),
    RouteEndpointSpec::get(
        "/api/venues/index-compositions/envelope",
        "diagnostic",
        "always",
        "low",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::get(
        "/api/venues/index-compositions/fetch",
        "diagnostic",
        "always",
        "medium",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::get(
        "/api/venues/quality",
        "main_p0",
        "always",
        "low",
        "bearer",
        "read",
    ),
];

#[cfg(feature = "legacy-chat")]
const CHAT_ENDPOINTS: &[RouteEndpointSpec] = &[
    RouteEndpointSpec::post(
        "/api/chat",
        "non_main",
        "default_off",
        "medium",
        "bearer",
        "external_payload",
    ),
    RouteEndpointSpec::get(
        "/api/chat/providers",
        "non_main",
        "default_off",
        "low",
        "bearer",
        "read",
    ),
];

#[cfg(feature = "legacy-chat")]
const LLM_DIAGNOSTIC_ENDPOINTS: &[RouteEndpointSpec] = &[
    RouteEndpointSpec::post(
        "/api/llm/daily-brief",
        "diagnostic",
        "default_off",
        "medium",
        "bearer",
        "external_payload",
    ),
    RouteEndpointSpec::post(
        "/api/llm/diagnose-failure",
        "diagnostic",
        "default_off",
        "medium",
        "bearer",
        "external_payload",
    ),
    RouteEndpointSpec::post(
        "/api/llm/explain-opportunity",
        "diagnostic",
        "default_off",
        "medium",
        "bearer",
        "external_payload",
    ),
];

#[cfg(feature = "legacy-options")]
const OPTIONS_ENDPOINTS: &[RouteEndpointSpec] = &[
    RouteEndpointSpec::post(
        "/api/options/greeks",
        "non_main",
        "default_off",
        "medium",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::post(
        "/api/options/iv",
        "non_main",
        "default_off",
        "medium",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::get(
        "/api/options/positions",
        "non_main",
        "default_off",
        "low",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::post(
        "/api/options/price",
        "non_main",
        "default_off",
        "medium",
        "bearer",
        "read",
    ),
];

const WATCHLIST_ENDPOINTS: &[RouteEndpointSpec] = &[
    RouteEndpointSpec::get(
        "/api/watchlist",
        "product_support",
        "default_off",
        "medium",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::post(
        "/api/watchlist",
        "product_support",
        "default_off",
        "high",
        "bearer",
        "required",
    ),
    RouteEndpointSpec::delete(
        "/api/watchlist/:id",
        "product_support",
        "default_off",
        "medium",
        "bearer",
        "required",
    ),
];

const ALERTS_ENDPOINTS: &[RouteEndpointSpec] = &[
    RouteEndpointSpec::get(
        "/api/alerts/rules",
        "product_support",
        "default_off",
        "medium",
        "bearer",
        "read",
    ),
    RouteEndpointSpec::post(
        "/api/alerts/rules",
        "product_support",
        "default_off",
        "high",
        "bearer",
        "required",
    ),
    RouteEndpointSpec::delete(
        "/api/alerts/rules/:id",
        "product_support",
        "default_off",
        "high",
        "bearer",
        "required",
    ),
];

const SPOT_ENDPOINTS: &[RouteEndpointSpec] = &[RouteEndpointSpec::get(
    "/api/v1/spot/ticks",
    "diagnostic",
    "default_off",
    "low",
    "bearer",
    "read",
)];

const STRATEGY_MAIN_ENDPOINTS: &[RouteEndpointSpec] = &[RouteEndpointSpec::get(
    "/api/strategy/main-kinds",
    "main_p0",
    "always",
    "low",
    "bearer",
    "read",
)];

const STRATEGY_V1_ENDPOINTS: &[RouteEndpointSpec] = &[RouteEndpointSpec::get(
    "/api/v1/strategy/kinds",
    "diagnostic",
    "default_off",
    "low",
    "bearer",
    "read",
)];

pub(crate) const ROUTE_SPECS: &[RouteSpec] = &[
    RouteSpec::with_endpoints(
        "health",
        RouteGate::Core,
        routers::health::router,
        HEALTH_ENDPOINTS,
    ),
    RouteSpec::with_endpoints(
        "metrics",
        RouteGate::Core,
        routers::metrics::router,
        METRICS_ENDPOINTS,
    ),
    RouteSpec::with_endpoints(
        "stocks",
        RouteGate::Core,
        routers::stocks::router,
        STOCK_ENDPOINTS,
    ),
    RouteSpec::with_endpoints(
        "onchain",
        RouteGate::Core,
        routers::onchain::router,
        ONCHAIN_ENDPOINTS,
    ),
    RouteSpec::with_endpoints(
        "auth",
        RouteGate::Core,
        routers::auth::router,
        AUTH_ENDPOINTS,
    ),
    RouteSpec::with_endpoints(
        "arbitrage",
        RouteGate::Core,
        routers::arbitrage::router,
        ARBITRAGE_ENDPOINTS,
    ),
    RouteSpec::with_endpoints(
        "automation",
        RouteGate::Core,
        routers::automation::router,
        AUTOMATION_ENDPOINTS,
    ),
    RouteSpec::with_endpoints(
        "exchanges",
        RouteGate::Core,
        routers::exchanges::router,
        EXCHANGES_ENDPOINTS,
    ),
    RouteSpec::with_endpoints(
        "history",
        RouteGate::Core,
        routers::history::router,
        HISTORY_ENDPOINTS,
    ),
    RouteSpec::with_endpoints(
        "portfolio",
        RouteGate::Core,
        routers::portfolio::router,
        PORTFOLIO_ENDPOINTS,
    ),
    RouteSpec::with_endpoints(
        "system",
        RouteGate::Core,
        routers::system::router,
        SYSTEM_ENDPOINTS,
    ),
    RouteSpec::with_endpoints(
        "venues",
        RouteGate::Core,
        routers::venues::router,
        VENUES_ENDPOINTS,
    ),
    RouteSpec::with_endpoints(
        "review",
        RouteGate::Core,
        routers::review::router,
        REVIEW_ENDPOINTS,
    ),
    RouteSpec::with_endpoints(
        "trading",
        RouteGate::Core,
        routers::trading::router,
        TRADING_ENDPOINTS,
    ),
    RouteSpec::with_endpoints(
        "websocket",
        RouteGate::Core,
        routers::websocket::router,
        WEBSOCKET_ENDPOINTS,
    ),
    RouteSpec::with_endpoints(
        "webhook",
        RouteGate::Core,
        routers::webhook::router,
        WEBHOOK_ENDPOINTS,
    ),
    #[cfg(feature = "legacy-chat")]
    RouteSpec::with_endpoints(
        "chat",
        RouteGate::Chat,
        routers::chat::router,
        CHAT_ENDPOINTS,
    ),
    #[cfg(feature = "legacy-chat")]
    RouteSpec::with_endpoints(
        "chat",
        RouteGate::LlmDiagnostics,
        routers::chat::llm_router,
        LLM_DIAGNOSTIC_ENDPOINTS,
    ),
    #[cfg(feature = "legacy-options")]
    RouteSpec::with_endpoints(
        "options",
        RouteGate::Options,
        routers::options::router,
        OPTIONS_ENDPOINTS,
    ),
    RouteSpec::with_endpoints(
        "watchlist",
        RouteGate::WatchlistAlerts,
        routers::watchlist::router,
        WATCHLIST_ENDPOINTS,
    ),
    RouteSpec::with_endpoints(
        "alerts",
        RouteGate::WatchlistAlerts,
        routers::alerts::router,
        ALERTS_ENDPOINTS,
    ),
    RouteSpec::with_endpoints(
        "spot",
        RouteGate::SpotV1,
        routers::spot::router,
        SPOT_ENDPOINTS,
    ),
    RouteSpec::with_endpoints(
        "strategy",
        RouteGate::Core,
        routers::strategy::main_router,
        STRATEGY_MAIN_ENDPOINTS,
    ),
    RouteSpec::with_endpoints(
        "strategy_v1",
        RouteGate::StrategyV1,
        routers::strategy::v1_router,
        STRATEGY_V1_ENDPOINTS,
    ),
];

pub(crate) const fn route_specs() -> &'static [RouteSpec] {
    ROUTE_SPECS
}

pub(crate) fn enabled_route_spec_count(surface: &ApiSurfaceConfig) -> usize {
    ROUTE_SPECS
        .iter()
        .copied()
        .filter(|spec| spec.enabled(surface))
        .count()
}

pub(crate) fn enabled_route_registry_summary(surface: &ApiSurfaceConfig) -> Vec<&'static str> {
    ROUTE_SPECS
        .iter()
        .copied()
        .filter(|spec| spec.enabled(surface))
        .map(|spec| spec.router_key())
        .collect()
}

pub(crate) fn disabled_route_registry_summary(surface: &ApiSurfaceConfig) -> Vec<&'static str> {
    let mut disabled = ROUTE_SPECS
        .iter()
        .copied()
        .filter(|spec| !spec.enabled(surface))
        .map(|spec| spec.feature_flag())
        .collect::<BTreeSet<_>>();
    disabled.extend(requested_but_uncompiled_route_features(surface));
    disabled.into_iter().collect()
}

#[cfg(not(all(feature = "legacy-chat", feature = "legacy-options")))]
fn requested_but_uncompiled_route_features(surface: &ApiSurfaceConfig) -> Vec<&'static str> {
    let mut unavailable = Vec::new();
    #[cfg(not(feature = "legacy-chat"))]
    {
        if surface.chat {
            unavailable.push(CHAT_REQUESTED_BUT_UNCOMPILED);
        }
        if surface.llm_diagnostics {
            unavailable.push(LLM_REQUESTED_BUT_UNCOMPILED);
        }
    }
    #[cfg(not(feature = "legacy-options"))]
    if surface.options {
        unavailable.push(OPTIONS_REQUESTED_BUT_UNCOMPILED);
    }
    unavailable
}

#[cfg(all(feature = "legacy-chat", feature = "legacy-options"))]
fn requested_but_uncompiled_route_features(_surface: &ApiSurfaceConfig) -> Vec<&'static str> {
    Vec::new()
}

pub(crate) fn seeded_endpoint_registry_summary(
    surface: &ApiSurfaceConfig,
) -> Vec<SeededEndpointSummary> {
    ROUTE_SPECS
        .iter()
        .copied()
        .filter(|spec| spec.enabled(surface))
        .flat_map(|spec| spec.endpoints())
        .map(|endpoint| {
            (
                endpoint.methods(),
                endpoint.path(),
                endpoint.class(),
                endpoint.default_exposure(),
                endpoint.risk(),
                endpoint.auth_policy(),
                endpoint.audit_policy(),
                endpoint.action_run_kind(),
            )
        })
        .collect()
}

pub(crate) fn seeded_endpoint_spec_count(surface: &ApiSurfaceConfig) -> usize {
    ROUTE_SPECS
        .iter()
        .copied()
        .filter(|spec| spec.enabled(surface))
        .map(|spec| spec.endpoints().len())
        .sum()
}

pub(crate) fn build_surface_router(surface: &ApiSurfaceConfig) -> Router<AppState> {
    ROUTE_SPECS
        .iter()
        .copied()
        .filter(|spec| spec.enabled(surface))
        .fold(Router::new(), |router, spec| router.merge(spec.router()))
}

#[cfg(test)]
#[path = "route_specs/tests.rs"]
mod tests;
