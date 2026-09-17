use super::postgres_history_error;
use crate::history::HistoryError;
use tokio_postgres::NoTls;

pub(super) const INDEX_LOCK_TIMEOUT: &str = "2s";
pub(super) const INDEX_STATEMENT_TIMEOUT: &str = "30s";
pub(super) const QUERY_EXPRESSION_INDEXES: &[(&str, &str)] = &[
    (
        "funding_rates",
        "CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_funding_rates_lower_filter \
         ON funding_rates (lower(exchange), lower(symbol), occurred_at_ms DESC);",
    ),
    (
        "funding_diffs",
        "CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_funding_diffs_lower_symbol \
         ON funding_diffs (lower(symbol), occurred_at_ms DESC);",
    ),
    (
        "opportunities",
        "CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_opportunities_lower_symbol \
         ON opportunities (lower(symbol), occurred_at_ms DESC);",
    ),
    (
        "index_compositions",
        "CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_index_compositions_lower_filter \
         ON index_compositions (lower(venue), lower(symbol), occurred_at_ms DESC);",
    ),
    (
        "api_health",
        "CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_api_health_lower_filter \
         ON api_health (lower(exchange), lower(endpoint), occurred_at_ms DESC);",
    ),
];

pub(super) fn spawn_query_expression_index_setup(database_url: String) {
    tokio::spawn(async move {
        if let Err(error) = setup_query_expression_indexes(&database_url).await {
            tracing::warn!(%error, "history query expression index background setup failed");
        }
    });
}

async fn setup_query_expression_indexes(database_url: &str) -> Result<(), HistoryError> {
    let (client, connection) = tokio_postgres::connect(database_url, NoTls)
        .await
        .map_err(|error| HistoryError::Unavailable(error.to_string()))?;
    tokio::spawn(async move {
        if let Err(error) = connection.await {
            tracing::warn!(%error, "postgres history index connection closed");
        }
    });
    client
        .batch_execute(&format!(
            "SET lock_timeout = '{INDEX_LOCK_TIMEOUT}'; \
             SET statement_timeout = '{INDEX_STATEMENT_TIMEOUT}';"
        ))
        .await
        .map_err(|error| postgres_history_error(&error))?;
    for (table, sql) in QUERY_EXPRESSION_INDEXES {
        if let Err(error) = client.batch_execute(sql).await {
            tracing::warn!(
                %error,
                table,
                "history query expression index setup skipped for one table"
            );
        }
    }
    Ok(())
}
