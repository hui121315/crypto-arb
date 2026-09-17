use crate::api::rest::MutationRequestContext;
use crate::panels::modules::positions::data::actions::{
    close_all_request_context, close_request_context, close_request_context_with_attempt,
    portfolio_scope_evidence, position_scope_evidence,
};
use crate::panels::modules::positions::data::requests::close_execution_scope;
use crate::state::load_state::LoadState;
use leptos::prelude::{Owner, RwSignal};
use shared_types::{
    ActionRunKind, ExecutionEnvironment, PositionSide, TradingRiskStatus, TradingStatusResponse,
    TradingWsChannels,
};

use super::super::support::{paired_row, portfolio_snapshot, row};

#[test]
fn pair_pending_evidence_keeps_action_and_both_legs() {
    let context = MutationRequestContext::with_idempotency_key("pair-idem");
    let row = paired_row(
        "binance",
        "BTCUSDT",
        PositionSide::Long,
        Some("okx@BTC-USDT-SWAP"),
    );

    let evidence = position_scope_evidence(&context, ActionRunKind::PortfolioClosePair, &row);

    assert_eq!(
        evidence.action_kind,
        Some(ActionRunKind::PortfolioClosePair)
    );
    assert_eq!(evidence.venues, ["binance", "okx"]);
    assert_eq!(evidence.symbols, ["BTCUSDT", "BTC-USDT-SWAP"]);
    assert_eq!(evidence.idempotency_key.as_deref(), Some("pair-idem"));
}

#[test]
fn close_all_pending_evidence_deduplicates_snapshot_scope() {
    Owner::new().with(|| {
        let context = MutationRequestContext::with_idempotency_key("all-idem");
        let mut snapshot = portfolio_snapshot("positions-v1");
        snapshot.positions = vec![
            row("binance", "BTCUSDT", PositionSide::Long, None),
            row("okx", "BTC-USDT-SWAP", PositionSide::Short, None),
            row("binance", "BTCUSDT", PositionSide::Long, None),
        ];
        let snapshot_state = RwSignal::new(LoadState::Ready(snapshot));

        let evidence = portfolio_scope_evidence(&context, snapshot_state);

        assert_eq!(evidence.action_kind, Some(ActionRunKind::PortfolioCloseAll));
        assert_eq!(evidence.venues, ["binance", "okx"]);
        assert_eq!(evidence.symbols, ["BTCUSDT", "BTC-USDT-SWAP"]);
        assert_eq!(evidence.idempotency_key.as_deref(), Some("all-idem"));
    });
}

#[test]
fn close_all_confirmation_correction_uses_a_new_idempotency_scope() {
    let unconfirmed =
        close_all_request_context(Some("positions-v1"), "", "adapter=mock:environment=paper");
    let confirmed = close_all_request_context(
        Some("positions-v1"),
        shared_types::CLOSE_ALL_POSITIONS_CONFIRMATION_PHRASE,
        "adapter=mock:environment=paper",
    );

    assert_ne!(unconfirmed.idempotency_key(), confirmed.idempotency_key());
    assert!(unconfirmed
        .idempotency_key()
        .is_some_and(|key| key.contains("portfolio:unconfirmed:adapter=mock:environment=paper")));
    assert!(confirmed
        .idempotency_key()
        .is_some_and(|key| key.contains("portfolio:confirmed:adapter=mock:environment=paper")));
}

#[test]
fn definitive_failed_retry_uses_a_new_idempotency_scope() {
    let first = close_request_context(
        "single",
        "binance:SOL:Long:adapter=live:environment=live",
        Some("positions-v1"),
    );
    let retry = close_request_context_with_attempt(
        "single",
        "binance:SOL:Long:adapter=live:environment=live",
        Some("positions-v1"),
        Some("close-failed-1"),
    );

    assert_ne!(first.idempotency_key(), retry.idempotency_key());
    assert!(retry
        .idempotency_key()
        .is_some_and(|key| key.ends_with(":after=close-failed-1")));
}

#[test]
fn completed_close_advances_identical_reopened_position_scope() {
    let first = close_request_context(
        "single",
        "binance:SOL:Long:adapter=live:environment=live",
        Some("positions-v1"),
    );
    let reopened = close_request_context_with_attempt(
        "single",
        "binance:SOL:Long:adapter=live:environment=live",
        Some("positions-v1"),
        Some("close-succeeded-1"),
    );

    assert_ne!(first.idempotency_key(), reopened.idempotency_key());
    assert!(reopened
        .idempotency_key()
        .is_some_and(|key| key.ends_with(":after=close-succeeded-1")));
}

#[test]
fn account_private_close_is_blocked_in_paper_mode() {
    Owner::new().with(|| {
        let status = RwSignal::new(LoadState::Ready(trading_status(
            "mock",
            ExecutionEnvironment::Paper,
            false,
        )));

        let result = close_execution_scope(status, true);
        assert!(result.is_err(), "paper close must block");
        let Some(problem) = result.err() else {
            return;
        };

        assert_eq!(problem.code, shared_types::problem::codes::RISK_BLOCKED);
        assert_eq!(problem.source.as_deref(), Some("positions.close_mode"));
        assert!(problem.message.contains("两步启用实盘"));
    });
}

#[test]
fn live_account_close_scope_cannot_replay_a_paper_action() {
    Owner::new().with(|| {
        let paper = RwSignal::new(LoadState::Ready(trading_status(
            "mock",
            ExecutionEnvironment::Paper,
            false,
        )));
        let live = RwSignal::new(LoadState::Ready(trading_status(
            "live",
            ExecutionEnvironment::Live,
            true,
        )));

        let paper_scope = close_execution_scope(paper, false);
        let live_scope = close_execution_scope(live, true);
        assert!(paper_scope.is_ok(), "paper ledger close should be allowed");
        assert!(live_scope.is_ok(), "live account close should be allowed");
        let (Ok(paper_scope), Ok(live_scope)) = (paper_scope, live_scope) else {
            return;
        };

        assert_ne!(paper_scope, live_scope);
        assert_eq!(live_scope, "adapter=live:environment=live");
    });
}

fn trading_status(
    adapter: &str,
    environment: ExecutionEnvironment,
    live_trading_enabled: bool,
) -> TradingStatusResponse {
    TradingStatusResponse {
        adapter: adapter.to_owned(),
        environment,
        open_order_count: 0,
        risk: TradingRiskStatus {
            live_trading_enabled,
            kill_switch_active: false,
            max_order_notional: 1_000.0,
            max_open_orders: 4,
            max_hedge_imbalance_pct: 0.05,
            liquidation_warn_pct: 0.2,
            liquidation_danger_pct: 0.1,
            allowed_exchanges: Vec::new(),
            allowed_symbols: Vec::new(),
            protected_positions: Vec::new(),
            auto_profit_close: Default::default(),
        },
        ws_channels: TradingWsChannels {
            orders: "orders".to_owned(),
            execution: "execution".to_owned(),
            risk_alerts: "risk_alerts".to_owned(),
        },
        action_run_id: None,
        request_id: None,
        idempotency_key: None,
        mutation: None,
    }
}
