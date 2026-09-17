#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::too_many_lines
    )
)]
//! 实盘下单。M9 阶段实现具体内容。

pub mod error;
pub mod execution;
pub mod journal;
pub mod ledger;
pub mod mock;
pub mod reconciler;
pub mod risk;
pub mod sql_ledger;
pub mod state_machine;

pub use error::{TradingError, TradingResult};
pub use execution::ExecutionEngine;
pub use journal::{
    CreatedClaim, ExecutionLedgerStorageSnapshot, FillOrderIdentity, OrderJournal,
    OrderSnapshotStorageSnapshot,
};
pub use ledger::{
    ExecutionLedger, ExecutionLedgerOrderContext, ExecutionLedgerQuery, FillLedgerInput,
    FundingLedgerInput, OrderbookDepthLedgerInput, SlippageLedgerInput,
};
pub use mock::MockLiveAdapter;
pub use reconciler::{diff_orders, ReconcileDiff, ReconcileDiffKind};
pub use risk::{exchange_allowed, symbol_allowed, RiskConfig, RiskEngine};
pub use sql_ledger::{
    init_sql_ledger_store, run_sql_ledger_migration, sql_ledger_schema_hash, SqlBalanceLedgerEvent,
    SqlBalanceLedgerReplayEvent, SqlLedgerInit, SqlLedgerMigrationHealth, SqlLedgerPersistAck,
    SqlLedgerReplay, SqlLedgerReplayHealth, SqlLedgerStorageSnapshot, SqlProjectionJob,
    SqlProjectionJobAck, SqlRealizedWindow, SqlRunCostFact, SqlRunCostRebuildReport,
    SqlRunFinalityLedgerEvent, SqlRunFinalityReplayEvent, CLOSE_RUN_PROJECTOR,
    EXECUTION_RUN_PROJECTOR, RUN_COST_PROJECTOR, SQL_LEDGER_MIGRATION_ID,
    SQL_LEDGER_MIGRATION_PATH, SQL_LEDGER_MIGRATION_VERSION, SQL_LEDGER_SCHEMA_VERSION,
};
pub use state_machine::{transition, OrderTransitionError};
