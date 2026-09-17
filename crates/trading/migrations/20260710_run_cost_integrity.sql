CREATE TABLE IF NOT EXISTS ledger_projection_jobs (
    event_id        TEXT NOT NULL REFERENCES order_events(event_id) ON DELETE CASCADE,
    projector       TEXT NOT NULL,
    payload_hash    TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'pending',
    attempt_count   INTEGER NOT NULL DEFAULT 0,
    available_at_ms BIGINT NOT NULL,
    claimed_at_ms   BIGINT,
    completed_at_ms BIGINT,
    last_error      TEXT,
    created_at_ms   BIGINT NOT NULL,
    updated_at_ms   BIGINT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (event_id, projector)
);

CREATE INDEX IF NOT EXISTS idx_ledger_projection_jobs_pending_order
    ON ledger_projection_jobs(available_at_ms, event_id, projector)
    WHERE status = 'pending';

CREATE TABLE IF NOT EXISTS run_cost_facts (
    run_kind              TEXT NOT NULL,
    run_id                TEXT NOT NULL,
    scope                 TEXT NOT NULL,
    component             TEXT NOT NULL,
    event_id              TEXT NOT NULL REFERENCES order_events(event_id) ON DELETE RESTRICT,
    source_order_event_id TEXT,
    ticket_id             TEXT,
    internal_order_id     TEXT,
    exchange              TEXT,
    symbol                TEXT,
    leg_role              TEXT,
    amount                DOUBLE PRECISION NOT NULL,
    currency              TEXT,
    amount_usd            DOUBLE PRECISION,
    quality               TEXT NOT NULL,
    payload               JSONB NOT NULL,
    payload_hash          TEXT NOT NULL,
    occurred_at_ms        BIGINT NOT NULL,
    captured_at_ms        BIGINT NOT NULL,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (run_kind, run_id, scope, component, event_id)
);

CREATE INDEX IF NOT EXISTS idx_run_cost_facts_run_component_time
    ON run_cost_facts(run_kind, run_id, scope, component, occurred_at_ms, event_id);

CREATE INDEX IF NOT EXISTS idx_run_cost_facts_event
    ON run_cost_facts(event_id);
