CREATE TABLE IF NOT EXISTS schema_migrations (
    migration_id   TEXT PRIMARY KEY,
    schema_name    TEXT NOT NULL,
    schema_version INTEGER NOT NULL,
    checksum       TEXT NOT NULL,
    applied_at_ms  BIGINT NOT NULL,
    applied_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS order_events (
    id BIGSERIAL PRIMARY KEY,
    event_id          TEXT NOT NULL UNIQUE,
    internal_order_id TEXT NOT NULL,
    client_order_id   TEXT NOT NULL,
    exchange_order_id TEXT,
    public_client_order_id TEXT NOT NULL,
    venue_client_order_id  TEXT,
    run_id            TEXT,
    ticket_id         TEXT,
    leg_role          TEXT,
    exchange          TEXT NOT NULL,
    symbol            TEXT NOT NULL,
    side              TEXT NOT NULL,
    order_ref         JSONB NOT NULL,
    event_type        TEXT NOT NULL,
    source            TEXT NOT NULL,
    state             TEXT,
    event             TEXT,
    payload           JSONB,
    payload_hash      TEXT NOT NULL,
    schema_version    INTEGER NOT NULL DEFAULT 1,
    occurred_at_ms    BIGINT NOT NULL,
    captured_at_ms    BIGINT NOT NULL,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_order_events_event_id
    ON order_events(event_id);

CREATE INDEX IF NOT EXISTS idx_order_events_internal_id
    ON order_events(internal_order_id);

CREATE INDEX IF NOT EXISTS idx_order_events_occurred_at
    ON order_events(occurred_at_ms DESC);

CREATE TABLE IF NOT EXISTS fills (
    event_id          TEXT PRIMARY KEY REFERENCES order_events(event_id) ON DELETE CASCADE,
    internal_order_id TEXT NOT NULL,
    exchange          TEXT NOT NULL,
    symbol            TEXT NOT NULL,
    side              TEXT NOT NULL,
    run_id            TEXT,
    ticket_id         TEXT,
    leg_role          TEXT,
    source            TEXT NOT NULL,
    fill_kind         TEXT NOT NULL,
    quantity          DOUBLE PRECISION NOT NULL,
    average_price     DOUBLE PRECISION NOT NULL,
    quote_value       DOUBLE PRECISION NOT NULL,
    quality           TEXT NOT NULL,
    fill_confidence   TEXT NOT NULL DEFAULT 'unknown',
    fill_confidence_score DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    fee_amount        DOUBLE PRECISION,
    fee_currency      TEXT,
    fee_quality       TEXT,
    payload_hash      TEXT NOT NULL,
    occurred_at_ms    BIGINT NOT NULL,
    captured_at_ms    BIGINT NOT NULL,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_fills_run_id
    ON fills(run_id);

CREATE INDEX IF NOT EXISTS idx_fills_symbol_time
    ON fills(exchange, symbol, occurred_at_ms DESC);

ALTER TABLE fills
    ADD COLUMN IF NOT EXISTS fill_confidence TEXT NOT NULL DEFAULT 'unknown';

ALTER TABLE fills
    ADD COLUMN IF NOT EXISTS fill_confidence_score DOUBLE PRECISION NOT NULL DEFAULT 0.0;

CREATE TABLE IF NOT EXISTS fees (
    event_id          TEXT PRIMARY KEY REFERENCES order_events(event_id) ON DELETE CASCADE,
    internal_order_id TEXT NOT NULL,
    exchange          TEXT NOT NULL,
    symbol            TEXT NOT NULL,
    side              TEXT NOT NULL,
    run_id            TEXT,
    ticket_id         TEXT,
    leg_role          TEXT,
    source            TEXT NOT NULL,
    fee_origin        TEXT NOT NULL,
    amount            DOUBLE PRECISION NOT NULL,
    currency          TEXT,
    quality           TEXT NOT NULL,
    payload_hash      TEXT NOT NULL,
    occurred_at_ms    BIGINT NOT NULL,
    captured_at_ms    BIGINT NOT NULL,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_fees_run_id
    ON fees(run_id);

CREATE INDEX IF NOT EXISTS idx_fees_symbol_time
    ON fees(exchange, symbol, occurred_at_ms DESC);

CREATE TABLE IF NOT EXISTS funding_payments (
    event_id          TEXT PRIMARY KEY REFERENCES order_events(event_id) ON DELETE CASCADE,
    internal_order_id TEXT NOT NULL,
    exchange          TEXT NOT NULL,
    symbol            TEXT NOT NULL,
    side              TEXT NOT NULL,
    run_id            TEXT,
    ticket_id         TEXT,
    leg_role          TEXT,
    source            TEXT NOT NULL,
    amount            DOUBLE PRECISION NOT NULL,
    currency          TEXT NOT NULL,
    funding_time_ms   BIGINT NOT NULL,
    quality           TEXT NOT NULL,
    payload_hash      TEXT NOT NULL,
    occurred_at_ms    BIGINT NOT NULL,
    captured_at_ms    BIGINT NOT NULL,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_funding_payments_run_id
    ON funding_payments(run_id);

CREATE INDEX IF NOT EXISTS idx_funding_payments_symbol_time
    ON funding_payments(exchange, symbol, funding_time_ms DESC);

CREATE TABLE IF NOT EXISTS slippage_events (
    event_id          TEXT PRIMARY KEY REFERENCES order_events(event_id) ON DELETE CASCADE,
    internal_order_id TEXT NOT NULL,
    exchange          TEXT NOT NULL,
    symbol            TEXT NOT NULL,
    side              TEXT NOT NULL,
    run_id            TEXT,
    ticket_id         TEXT,
    leg_role          TEXT,
    source            TEXT NOT NULL,
    amount_usd        DOUBLE PRECISION NOT NULL,
    reference_price   DOUBLE PRECISION NOT NULL,
    fill_price        DOUBLE PRECISION NOT NULL,
    quantity          DOUBLE PRECISION NOT NULL,
    quality           TEXT NOT NULL,
    payload_hash      TEXT NOT NULL,
    occurred_at_ms    BIGINT NOT NULL,
    captured_at_ms    BIGINT NOT NULL,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_slippage_events_run_id
    ON slippage_events(run_id);

CREATE INDEX IF NOT EXISTS idx_slippage_events_symbol_time
    ON slippage_events(exchange, symbol, occurred_at_ms DESC);

CREATE TABLE IF NOT EXISTS orderbook_evidence (
    event_id          TEXT PRIMARY KEY REFERENCES order_events(event_id) ON DELETE CASCADE,
    internal_order_id TEXT NOT NULL,
    exchange          TEXT NOT NULL,
    symbol            TEXT NOT NULL,
    side              TEXT NOT NULL,
    run_id            TEXT,
    ticket_id         TEXT,
    leg_role          TEXT,
    source            TEXT NOT NULL,
    reference_price   DOUBLE PRECISION,
    bid               DOUBLE PRECISION,
    ask               DOUBLE PRECISION,
    mid               DOUBLE PRECISION,
    open_vwap_price   DOUBLE PRECISION,
    open_slippage_bps DOUBLE PRECISION,
    close_vwap_price  DOUBLE PRECISION,
    close_slippage_bps DOUBLE PRECISION,
    depth_usd_5bps    DOUBLE PRECISION,
    depth_usd_10bps   DOUBLE PRECISION,
    depth_usd_20bps   DOUBLE PRECISION,
    max_notional_usd  DOUBLE PRECISION,
    market_timestamp_ms BIGINT,
    health            JSONB,
    reason            TEXT,
    evidence_quality  TEXT NOT NULL,
    payload_hash      TEXT NOT NULL,
    occurred_at_ms    BIGINT NOT NULL,
    captured_at_ms    BIGINT NOT NULL,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_orderbook_evidence_run_id
    ON orderbook_evidence(run_id);

CREATE INDEX IF NOT EXISTS idx_orderbook_evidence_symbol_time
    ON orderbook_evidence(exchange, symbol, occurred_at_ms DESC);

CREATE TABLE IF NOT EXISTS balance_events (
    event_id       TEXT PRIMARY KEY,
    exchange       TEXT NOT NULL,
    asset          TEXT NOT NULL,
    balance_kind   TEXT NOT NULL,
    payload        JSONB NOT NULL,
    payload_hash   TEXT NOT NULL,
    observed_at_ms BIGINT NOT NULL,
    captured_at_ms BIGINT NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_balance_events_exchange_asset_time
    ON balance_events(exchange, asset, observed_at_ms DESC);

CREATE TABLE IF NOT EXISTS order_snapshots (
    internal_order_id TEXT PRIMARY KEY,
    public_client_order_id TEXT NOT NULL,
    venue_client_order_id  TEXT,
    exchange_order_id TEXT,
    state             TEXT NOT NULL,
    last_update_source TEXT NOT NULL,
    exchange          TEXT NOT NULL,
    symbol            TEXT NOT NULL,
    side              TEXT NOT NULL,
    record            JSONB NOT NULL,
    updated_at_ms     BIGINT NOT NULL,
    record_hash       TEXT NOT NULL,
    schema_version    INTEGER NOT NULL DEFAULT 1,
    captured_at_ms    BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS run_finality_events (
    id BIGSERIAL PRIMARY KEY,
    event_id              TEXT NOT NULL UNIQUE,
    run_kind              TEXT NOT NULL,
    run_id                TEXT NOT NULL,
    source_event_id       TEXT,
    source_order_event_id TEXT,
    source                TEXT NOT NULL,
    state                 TEXT NOT NULL,
    payload               JSONB NOT NULL,
    payload_hash          TEXT NOT NULL,
    schema_version        INTEGER NOT NULL DEFAULT 1,
    occurred_at_ms        BIGINT NOT NULL,
    captured_at_ms        BIGINT NOT NULL,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_run_finality_events_run
    ON run_finality_events(run_kind, run_id, occurred_at_ms DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_run_finality_events_source_event
    ON run_finality_events(source_event_id);
