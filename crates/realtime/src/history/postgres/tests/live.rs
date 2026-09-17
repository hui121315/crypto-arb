use super::*;

#[tokio::test]
#[ignore = "requires CROSSLINE_TEST_HISTORY_POSTGRES_URL"]
async fn history_postgres_restart_rejects_migration_and_table_drift() {
    let database_url = std::env::var("CROSSLINE_TEST_HISTORY_POSTGRES_URL")
        .expect("CROSSLINE_TEST_HISTORY_POSTGRES_URL");
    let (client, connection) = tokio_postgres::connect(&database_url, NoTls)
        .await
        .expect("connect drift fixture");
    let connection_task = tokio::spawn(async move {
        let _ = connection.await;
    });

    client
        .batch_execute("CREATE TABLE funding_rates (orphan_id BIGINT)")
        .await
        .expect("create orphan history table");
    assert_history_connect_drift(&database_url, "schema_migrations authority").await;
    let history_meta: Option<String> = client
        .query_one(
            "SELECT to_regclass(current_schema() || '.history_meta')::text AS table_name",
            &[],
        )
        .await
        .expect("probe history metadata after rejected orphan table")
        .try_get("table_name")
        .expect("history metadata probe");
    assert_eq!(history_meta, None, "startup must not adopt an orphan table");
    client
        .batch_execute("DROP TABLE funding_rates")
        .await
        .expect("drop orphan history table");

    let (first, second) = tokio::join!(
        PostgresHistoryStore::connect(&database_url),
        PostgresHistoryStore::connect(&database_url)
    );
    drop(first.expect("initialize history schema from first instance"));
    drop(second.expect("initialize history schema from second instance"));

    let checksum = history_migration_checksum_hex();

    client
        .execute(
            "UPDATE schema_migrations SET checksum = 'tampered' WHERE migration_id = $1",
            &[&HISTORY_MIGRATION_ID],
        )
        .await
        .expect("tamper checksum");
    assert_history_connect_drift(&database_url, "checksum").await;
    let stored_checksum: String = client
        .query_one(
            "SELECT checksum FROM schema_migrations WHERE migration_id = $1",
            &[&HISTORY_MIGRATION_ID],
        )
        .await
        .expect("read tampered checksum")
        .try_get("checksum")
        .expect("checksum column");
    assert_eq!(stored_checksum, "tampered");

    client
        .execute(
            "UPDATE schema_migrations SET checksum = $1 WHERE migration_id = $2",
            &[&checksum, &HISTORY_MIGRATION_ID],
        )
        .await
        .expect("restore checksum");
    client
        .execute(
            "DELETE FROM schema_migrations WHERE migration_id = $1",
            &[&HISTORY_MIGRATION_ID],
        )
        .await
        .expect("remove applied migration row");
    assert_history_connect_drift(&database_url, "applied migration row").await;
    let remaining_rows: i64 = client
        .query_one("SELECT COUNT(*) AS count FROM schema_migrations", &[])
        .await
        .expect("count migration rows")
        .try_get("count")
        .expect("count column");
    assert_eq!(
        remaining_rows, 0,
        "startup must not recreate missing history"
    );

    let schema_version = HISTORY_SCHEMA_VERSION as i32;
    let applied_at_ms = 1_700_000_000_000_i64;
    client
        .execute(
            INSERT_HISTORY_MIGRATION_SQL,
            &[
                &HISTORY_MIGRATION_ID,
                &HISTORY_SCHEMA_NAME,
                &schema_version,
                &checksum,
                &applied_at_ms,
            ],
        )
        .await
        .expect("restore applied migration row");
    client
        .execute(
            "UPDATE history_meta SET value = 99 WHERE key = 'schema_version'",
            &[],
        )
        .await
        .expect("tamper schema version");
    assert_history_connect_drift(&database_url, "schema version mismatch").await;
    let stored_version: i64 = client
        .query_one(
            "SELECT value FROM history_meta WHERE key = 'schema_version'",
            &[],
        )
        .await
        .expect("read tampered schema version")
        .try_get("value")
        .expect("schema version column");
    assert_eq!(stored_version, 99);

    client
        .execute(
            "UPDATE history_meta SET value = $1 WHERE key = 'schema_version'",
            &[&i64::from(HISTORY_SCHEMA_VERSION)],
        )
        .await
        .expect("restore schema version");
    client
        .batch_execute("DROP TABLE funding_rates")
        .await
        .expect("drop required history table");
    assert_history_connect_drift(&database_url, "funding_rates").await;

    connection_task.abort();
}

async fn assert_history_connect_drift(database_url: &str, marker: &str) {
    let error = PostgresHistoryStore::connect(database_url)
        .await
        .expect_err("history schema drift must fail closed");
    assert!(
        matches!(error, HistoryError::SchemaDrift(_)),
        "expected schema drift, got {error}"
    );
    assert!(
        error.to_string().contains(marker),
        "missing marker {marker}: {error}"
    );
}
