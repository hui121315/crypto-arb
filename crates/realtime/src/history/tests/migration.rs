use super::*;

#[test]
fn history_migration_contains_runtime_history_tables() {
    let migration = include_str!("../../../migrations/20260701_history.sql");

    for table in ["funding_diffs", "index_compositions"] {
        assert!(
            migration.contains(&format!("CREATE TABLE IF NOT EXISTS {table}")),
            "migration missing runtime table {table}"
        );
        assert!(
            migration.contains(&format!("create_hypertable('{table}'")),
            "migration missing hypertable setup for {table}"
        );
    }
    assert!(
        migration.contains("CREATE TABLE IF NOT EXISTS history_meta"),
        "migration missing history_meta schema table"
    );
    assert!(
        migration.contains("schema_version"),
        "migration missing history schema version marker"
    );
    assert!(
        migration.contains(&format!(
            "VALUES ('schema_version', {HISTORY_SCHEMA_VERSION},"
        )),
        "migration schema_version marker does not match HISTORY_SCHEMA_VERSION"
    );
    assert_eq!(history_migration_checksum_hex().len(), 16);
}

#[test]
fn history_migration_includes_observability_tables() {
    let migration = include_str!("../../../migrations/20260701_history.sql");

    for table in ["api_health", "events"] {
        assert!(
            migration.contains(&format!("CREATE TABLE IF NOT EXISTS {table}")),
            "migration missing observability table {table}"
        );
        assert!(
            migration.contains(&format!("create_hypertable('{table}'")),
            "migration missing hypertable setup for {table}"
        );
    }
    for correlation in [
        "idx_events_request_id_time",
        "idx_events_run_id_time",
        "idx_events_ticket_id_time",
        "idx_events_client_order_id_time",
        "idx_events_exchange_order_id_time",
    ] {
        assert!(
            migration.contains(correlation),
            "migration missing correlation index {correlation}"
        );
    }
}

#[test]
fn history_migration_includes_execution_ledger_tables() {
    let migration = include_str!("../../../migrations/20260701_history.sql");

    for table in [
        "executions",
        "orders",
        "fills",
        "fees",
        "funding_payments",
        "balances",
    ] {
        assert!(
            migration.contains(&format!("CREATE TABLE IF NOT EXISTS {table}")),
            "migration missing execution ledger table {table}"
        );
        assert!(
            migration.contains(&format!("create_hypertable('{table}'")),
            "migration missing hypertable setup for {table}"
        );
    }
    for correlation in [
        "idx_orders_client_order_id_time",
        "idx_orders_exchange_order_id_time",
        "idx_orders_run_id_time",
        "idx_fills_run_id_time",
        "idx_executions_run_id_time",
        "idx_executions_ticket_id_time",
    ] {
        assert!(
            migration.contains(correlation),
            "migration missing execution ledger correlation index {correlation}"
        );
    }
}

#[test]
fn history_schema_version_is_observability_revision() {
    assert_eq!(HISTORY_SCHEMA_VERSION, 2);
}
