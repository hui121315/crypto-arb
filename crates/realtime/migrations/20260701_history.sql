CREATE EXTENSION IF NOT EXISTS timescaledb;

CREATE TABLE IF NOT EXISTS history_meta (
    key           TEXT PRIMARY KEY,
    value         BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL
);

INSERT INTO history_meta (key, value, updated_at_ms)
VALUES ('schema_version', 2, (EXTRACT(EPOCH FROM NOW()) * 1000)::BIGINT)
ON CONFLICT (key) DO UPDATE SET
    value = EXCLUDED.value,
    updated_at_ms = EXCLUDED.updated_at_ms;

CREATE TABLE IF NOT EXISTS schema_migrations (
    migration_id    TEXT PRIMARY KEY,
    schema_name     TEXT NOT NULL,
    schema_version  INTEGER NOT NULL,
    checksum        TEXT NOT NULL,
    applied_at_ms   BIGINT NOT NULL,
    applied_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS funding_rates (
    occurred_at_ms   BIGINT NOT NULL,
    exchange         TEXT NOT NULL,
    symbol           TEXT NOT NULL,
    rate             DOUBLE PRECISION NOT NULL,
    interval_hours   INTEGER NOT NULL,
    next_funding_ms  BIGINT,
    volume_24h       DOUBLE PRECISION
);

SELECT create_hypertable('funding_rates', 'occurred_at_ms', chunk_time_interval => 86400000, if_not_exists => TRUE);

CREATE INDEX IF NOT EXISTS idx_funding_rates_exchange_symbol_time
    ON funding_rates (exchange, symbol, occurred_at_ms DESC);

CREATE TABLE IF NOT EXISTS funding_diffs (
    occurred_at_ms             BIGINT NOT NULL,
    symbol                     TEXT NOT NULL,
    long_exchange              TEXT NOT NULL,
    short_exchange             TEXT NOT NULL,
    long_rate_8h               DOUBLE PRECISION NOT NULL,
    short_rate_8h              DOUBLE PRECISION NOT NULL,
    gross_diff_bps             DOUBLE PRECISION NOT NULL,
    long_next_funding_ms       BIGINT,
    short_next_funding_ms      BIGINT,
    window_alignment_minutes   INTEGER,
    long_interval_hours        INTEGER,
    short_interval_hours       INTEGER,
    min_volume_24h             DOUBLE PRECISION
);

SELECT create_hypertable('funding_diffs', 'occurred_at_ms', chunk_time_interval => 86400000, if_not_exists => TRUE);

CREATE INDEX IF NOT EXISTS idx_funding_diffs_pair_time
    ON funding_diffs (symbol, long_exchange, short_exchange, occurred_at_ms DESC);

CREATE TABLE IF NOT EXISTS opportunities (
    occurred_at_ms BIGINT NOT NULL,
    id             TEXT NOT NULL,
    symbol         TEXT NOT NULL,
    long_exchange  TEXT NOT NULL,
    short_exchange TEXT NOT NULL,
    spread_8h      DOUBLE PRECISION,
    net_yield      DOUBLE PRECISION,
    volume_24h_min DOUBLE PRECISION,
    payload        JSONB NOT NULL
);

SELECT create_hypertable('opportunities', 'occurred_at_ms', chunk_time_interval => 86400000, if_not_exists => TRUE);

CREATE INDEX IF NOT EXISTS idx_opportunities_symbol_time
    ON opportunities (symbol, occurred_at_ms DESC);

CREATE TABLE IF NOT EXISTS index_compositions (
    occurred_at_ms BIGINT NOT NULL,
    venue          TEXT NOT NULL,
    symbol         TEXT NOT NULL,
    index_id       TEXT NOT NULL,
    quality        TEXT NOT NULL,
    component_count INTEGER NOT NULL,
    source         TEXT NOT NULL,
    payload        JSONB NOT NULL
);

SELECT create_hypertable('index_compositions', 'occurred_at_ms', chunk_time_interval => 86400000, if_not_exists => TRUE);

CREATE INDEX IF NOT EXISTS idx_index_compositions_venue_symbol_time
    ON index_compositions (venue, symbol, occurred_at_ms DESC);

CREATE TABLE IF NOT EXISTS watchlist (
    id BIGSERIAL PRIMARY KEY,
    symbol TEXT NOT NULL,
    venue_long TEXT,
    venue_short TEXT,
    min_net_yield DOUBLE PRECISION,
    min_volume_24h DOUBLE PRECISION,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS alert_rules (
    id BIGSERIAL PRIMARY KEY,
    watchlist_id BIGINT NOT NULL REFERENCES watchlist(id) ON DELETE CASCADE,
    channel JSONB NOT NULL,
    cooldown_secs BIGINT NOT NULL DEFAULT 300,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Observability ledger: per venue/endpoint API health samples so Prometheus and
-- the settings page can localise which venue + endpoint is slow / erroring /
-- rate-limited (status / error / latency / retry_after / circuit).
CREATE TABLE IF NOT EXISTS api_health (
    occurred_at_ms BIGINT NOT NULL,
    exchange       TEXT NOT NULL,
    endpoint       TEXT NOT NULL,
    method         TEXT,
    outcome        TEXT NOT NULL,
    status_code    INTEGER,
    latency_ms     DOUBLE PRECISION,
    retry_after_ms BIGINT,
    circuit_state  TEXT,
    error_code     TEXT,
    payload        JSONB NOT NULL
);

SELECT create_hypertable('api_health', 'occurred_at_ms', chunk_time_interval => 86400000, if_not_exists => TRUE);

CREATE INDEX IF NOT EXISTS idx_api_health_exchange_endpoint_time
    ON api_health (exchange, endpoint, occurred_at_ms DESC);

CREATE INDEX IF NOT EXISTS idx_api_health_outcome_time
    ON api_health (outcome, occurred_at_ms DESC);

-- Unified event/audit ledger: every high-risk operation and notable lifecycle
-- event leaves a durable record threaded with the full correlation chain
-- (request_id / run_id / ticket_id / client_order_id / exchange_order_id) so
-- events can be replayed and joined across the platform.
CREATE TABLE IF NOT EXISTS events (
    occurred_at_ms    BIGINT NOT NULL,
    event_id          TEXT NOT NULL,
    category          TEXT NOT NULL,
    action            TEXT NOT NULL,
    actor             TEXT,
    resource          TEXT,
    outcome           TEXT NOT NULL,
    severity          TEXT,
    request_id        TEXT,
    run_id            TEXT,
    ticket_id         TEXT,
    client_order_id   TEXT,
    exchange_order_id TEXT,
    payload           JSONB NOT NULL
);

SELECT create_hypertable('events', 'occurred_at_ms', chunk_time_interval => 86400000, if_not_exists => TRUE);

CREATE INDEX IF NOT EXISTS idx_events_category_time
    ON events (category, occurred_at_ms DESC);

CREATE INDEX IF NOT EXISTS idx_events_request_id_time
    ON events (request_id, occurred_at_ms DESC);

CREATE INDEX IF NOT EXISTS idx_events_run_id_time
    ON events (run_id, occurred_at_ms DESC);

CREATE INDEX IF NOT EXISTS idx_events_ticket_id_time
    ON events (ticket_id, occurred_at_ms DESC);

CREATE INDEX IF NOT EXISTS idx_events_client_order_id_time
    ON events (client_order_id, occurred_at_ms DESC);

CREATE INDEX IF NOT EXISTS idx_events_exchange_order_id_time
    ON events (exchange_order_id, occurred_at_ms DESC);

-- Execution ledger foundation (PR-W provides the durable schema; append-only
-- writes land via PR-AB / PR-H). Every row is threaded with the correlation
-- chain so an execution run can be replayed across orders, fills, fees and
-- funding payments.

-- Execution run lifecycle facts (one row per run-level state transition).
CREATE TABLE IF NOT EXISTS executions (
    occurred_at_ms BIGINT NOT NULL,
    run_id         TEXT NOT NULL,
    request_id     TEXT,
    ticket_id      TEXT,
    strategy       TEXT,
    status         TEXT NOT NULL,
    leg_count      INTEGER,
    notional_usd   DOUBLE PRECISION,
    payload        JSONB NOT NULL
);

SELECT create_hypertable('executions', 'occurred_at_ms', chunk_time_interval => 86400000, if_not_exists => TRUE);

CREATE INDEX IF NOT EXISTS idx_executions_run_id_time
    ON executions (run_id, occurred_at_ms DESC);

CREATE INDEX IF NOT EXISTS idx_executions_ticket_id_time
    ON executions (ticket_id, occurred_at_ms DESC);

-- Order state records (one row per venue order state transition).
CREATE TABLE IF NOT EXISTS orders (
    occurred_at_ms    BIGINT NOT NULL,
    exchange          TEXT NOT NULL,
    symbol            TEXT NOT NULL,
    side              TEXT NOT NULL,
    state             TEXT NOT NULL,
    client_order_id   TEXT,
    exchange_order_id TEXT,
    run_id            TEXT,
    ticket_id         TEXT,
    request_id        TEXT,
    leg_role          TEXT,
    source            TEXT,
    payload           JSONB NOT NULL
);

SELECT create_hypertable('orders', 'occurred_at_ms', chunk_time_interval => 86400000, if_not_exists => TRUE);

CREATE INDEX IF NOT EXISTS idx_orders_exchange_symbol_time
    ON orders (exchange, symbol, occurred_at_ms DESC);

CREATE INDEX IF NOT EXISTS idx_orders_client_order_id_time
    ON orders (client_order_id, occurred_at_ms DESC);

CREATE INDEX IF NOT EXISTS idx_orders_exchange_order_id_time
    ON orders (exchange_order_id, occurred_at_ms DESC);

CREATE INDEX IF NOT EXISTS idx_orders_run_id_time
    ON orders (run_id, occurred_at_ms DESC);

-- Fill events (one row per executed quantity, grounded in FillLedgerSnapshot).
CREATE TABLE IF NOT EXISTS fills (
    occurred_at_ms    BIGINT NOT NULL,
    exchange          TEXT NOT NULL,
    symbol            TEXT NOT NULL,
    side              TEXT NOT NULL,
    quantity          DOUBLE PRECISION NOT NULL,
    average_price     DOUBLE PRECISION NOT NULL,
    quote_value       DOUBLE PRECISION NOT NULL,
    quality           TEXT NOT NULL,
    confidence        TEXT,
    client_order_id   TEXT,
    exchange_order_id TEXT,
    run_id            TEXT,
    ticket_id         TEXT,
    payload           JSONB NOT NULL
);

SELECT create_hypertable('fills', 'occurred_at_ms', chunk_time_interval => 86400000, if_not_exists => TRUE);

CREATE INDEX IF NOT EXISTS idx_fills_exchange_symbol_time
    ON fills (exchange, symbol, occurred_at_ms DESC);

CREATE INDEX IF NOT EXISTS idx_fills_client_order_id_time
    ON fills (client_order_id, occurred_at_ms DESC);

CREATE INDEX IF NOT EXISTS idx_fills_exchange_order_id_time
    ON fills (exchange_order_id, occurred_at_ms DESC);

CREATE INDEX IF NOT EXISTS idx_fills_run_id_time
    ON fills (run_id, occurred_at_ms DESC);

-- Fee events (one row per realised fee, grounded in FeeLedgerSnapshot).
CREATE TABLE IF NOT EXISTS fees (
    occurred_at_ms    BIGINT NOT NULL,
    exchange          TEXT NOT NULL,
    symbol            TEXT,
    amount            DOUBLE PRECISION NOT NULL,
    currency          TEXT,
    quality           TEXT NOT NULL,
    client_order_id   TEXT,
    exchange_order_id TEXT,
    run_id            TEXT,
    ticket_id         TEXT,
    payload           JSONB NOT NULL
);

SELECT create_hypertable('fees', 'occurred_at_ms', chunk_time_interval => 86400000, if_not_exists => TRUE);

CREATE INDEX IF NOT EXISTS idx_fees_exchange_symbol_time
    ON fees (exchange, symbol, occurred_at_ms DESC);

CREATE INDEX IF NOT EXISTS idx_fees_run_id_time
    ON fees (run_id, occurred_at_ms DESC);

-- Funding payment events (grounded in FundingPaymentLedgerRecord).
CREATE TABLE IF NOT EXISTS funding_payments (
    occurred_at_ms  BIGINT NOT NULL,
    exchange        TEXT NOT NULL,
    symbol          TEXT NOT NULL,
    amount          DOUBLE PRECISION NOT NULL,
    currency        TEXT NOT NULL,
    funding_time_ms BIGINT NOT NULL,
    quality         TEXT NOT NULL,
    run_id          TEXT,
    ticket_id       TEXT,
    payload         JSONB NOT NULL
);

SELECT create_hypertable('funding_payments', 'occurred_at_ms', chunk_time_interval => 86400000, if_not_exists => TRUE);

CREATE INDEX IF NOT EXISTS idx_funding_payments_exchange_symbol_time
    ON funding_payments (exchange, symbol, occurred_at_ms DESC);

CREATE INDEX IF NOT EXISTS idx_funding_payments_run_id_time
    ON funding_payments (run_id, occurred_at_ms DESC);

-- Balance snapshots (one row per venue/asset balance observation).
CREATE TABLE IF NOT EXISTS balances (
    occurred_at_ms BIGINT NOT NULL,
    exchange       TEXT NOT NULL,
    asset          TEXT NOT NULL,
    total          DOUBLE PRECISION NOT NULL,
    available      DOUBLE PRECISION,
    locked         DOUBLE PRECISION,
    run_id         TEXT,
    request_id     TEXT,
    payload        JSONB NOT NULL
);

SELECT create_hypertable('balances', 'occurred_at_ms', chunk_time_interval => 86400000, if_not_exists => TRUE);

CREATE INDEX IF NOT EXISTS idx_balances_exchange_asset_time
    ON balances (exchange, asset, occurred_at_ms DESC);

CREATE INDEX IF NOT EXISTS idx_balances_run_id_time
    ON balances (run_id, occurred_at_ms DESC);
