CREATE TABLE IF NOT EXISTS watchlist_alert_snapshots (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    schema_version INTEGER NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    payload_json TEXT NOT NULL,
    payload_hash TEXT NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS watchlist_alert_meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
