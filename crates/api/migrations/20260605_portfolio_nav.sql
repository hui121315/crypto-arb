CREATE TABLE IF NOT EXISTS portfolio_nav_samples (
    occurred_at_ms INTEGER PRIMARY KEY,
    nav_usd        REAL NOT NULL,
    status         TEXT NOT NULL DEFAULT 'ok',
    source         TEXT NOT NULL DEFAULT 'position_margin',
    problem        TEXT
);

CREATE INDEX IF NOT EXISTS idx_portfolio_nav_samples_time
    ON portfolio_nav_samples (occurred_at_ms DESC);

CREATE TABLE IF NOT EXISTS portfolio_nav_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

PRAGMA user_version = 1;

INSERT OR REPLACE INTO portfolio_nav_meta (key, value)
VALUES ('schema_version', '1');
