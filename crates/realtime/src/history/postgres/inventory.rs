use super::{postgres_history_error, HistoryError, PostgresHistoryStore, HISTORY_RUNTIME_TABLES};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct HistorySchemaPresence {
    pub(super) migration_table: bool,
    pub(super) history_meta: bool,
    pub(super) runtime_tables: bool,
}

impl PostgresHistoryStore {
    pub(super) async fn load_schema_presence(&self) -> Result<HistorySchemaPresence, HistoryError> {
        let present = self.load_schema_tables().await?;
        Ok(HistorySchemaPresence {
            migration_table: present.contains("schema_migrations"),
            history_meta: present.contains("history_meta"),
            runtime_tables: HISTORY_RUNTIME_TABLES[2..]
                .iter()
                .any(|table| present.contains(*table)),
        })
    }

    pub(super) async fn validate_runtime_tables(&self) -> Result<(), HistoryError> {
        let present = self.load_schema_tables().await?;
        let missing = HISTORY_RUNTIME_TABLES
            .iter()
            .filter(|table| !present.contains(**table))
            .copied()
            .collect::<Vec<_>>();
        if missing.is_empty() {
            Ok(())
        } else {
            Err(HistoryError::SchemaDrift(format!(
                "history runtime tables are missing: {}",
                missing.join(", ")
            )))
        }
    }

    async fn load_schema_tables(&self) -> Result<HashSet<String>, HistoryError> {
        let rows = self
            .client
            .query(
                "SELECT table_name FROM information_schema.tables \
                 WHERE table_schema = current_schema()",
                &[],
            )
            .await
            .map_err(|error| postgres_history_error(&error))?;
        let mut present = HashSet::with_capacity(rows.len());
        for row in rows {
            let table: String = row.try_get("table_name").map_err(|error| {
                HistoryError::SchemaDrift(format!(
                    "history table inventory row is invalid: {error}"
                ))
            })?;
            present.insert(table);
        }
        Ok(present)
    }
}
