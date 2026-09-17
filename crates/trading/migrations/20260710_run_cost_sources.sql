ALTER TABLE run_cost_facts
    ADD COLUMN IF NOT EXISTS source_run_finality_event_id TEXT;

UPDATE run_cost_facts
SET source_order_event_id = event_id
WHERE source_order_event_id IS NULL
  AND source_run_finality_event_id IS NULL;

ALTER TABLE run_cost_facts
    DROP CONSTRAINT IF EXISTS run_cost_facts_event_id_fkey;

ALTER TABLE run_cost_facts
    ADD CONSTRAINT run_cost_facts_source_order_event_fkey
    FOREIGN KEY (source_order_event_id)
    REFERENCES order_events(event_id)
    ON DELETE RESTRICT;

ALTER TABLE run_cost_facts
    ADD CONSTRAINT run_cost_facts_source_run_finality_event_fkey
    FOREIGN KEY (source_run_finality_event_id)
    REFERENCES run_finality_events(event_id)
    ON DELETE RESTRICT;

ALTER TABLE run_cost_facts
    ADD CONSTRAINT run_cost_facts_exactly_one_source_check
    CHECK (
        (source_order_event_id IS NOT NULL AND source_run_finality_event_id IS NULL)
        OR (source_order_event_id IS NULL AND source_run_finality_event_id IS NOT NULL)
    );

CREATE INDEX IF NOT EXISTS idx_run_cost_facts_source_order_event
    ON run_cost_facts(source_order_event_id)
    WHERE source_order_event_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_run_cost_facts_source_run_finality_event
    ON run_cost_facts(source_run_finality_event_id)
    WHERE source_run_finality_event_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS run_finality_source_links (
    run_finality_event_id TEXT NOT NULL
        REFERENCES run_finality_events(event_id) ON DELETE RESTRICT,
    status                TEXT NOT NULL,
    source_event_id       TEXT,
    source_order_event_id TEXT,
    candidate_count       INTEGER NOT NULL,
    linked_at_ms          BIGINT NOT NULL,
    PRIMARY KEY (run_finality_event_id),
    CONSTRAINT run_finality_source_links_status_check CHECK (
        status IN ('intrinsic', 'unique', 'missing', 'unlinked', 'ambiguous')
    ),
    CONSTRAINT run_finality_source_links_resolution_check CHECK (
        (status = 'intrinsic'
            AND candidate_count = 1
            AND (source_event_id IS NOT NULL OR source_order_event_id IS NOT NULL))
        OR (status = 'unique'
            AND candidate_count = 1
            AND source_event_id IS NOT NULL
            AND source_order_event_id IS NOT NULL)
        OR (status IN ('missing', 'unlinked')
            AND candidate_count = 0
            AND source_event_id IS NULL
            AND source_order_event_id IS NULL)
        OR (status = 'ambiguous'
            AND candidate_count > 1
            AND source_event_id IS NULL
            AND source_order_event_id IS NULL)
    )
);

INSERT INTO run_finality_source_links (
    run_finality_event_id,
    status,
    source_event_id,
    source_order_event_id,
    candidate_count,
    linked_at_ms
)
SELECT
    finality.event_id,
    'intrinsic',
    finality.source_event_id,
    finality.source_order_event_id,
    1,
    finality.captured_at_ms
FROM run_finality_events AS finality
WHERE finality.source_event_id IS NOT NULL
   OR finality.source_order_event_id IS NOT NULL
ON CONFLICT (run_finality_event_id) DO NOTHING;

CREATE INDEX IF NOT EXISTS idx_run_finality_source_links_order_event
    ON run_finality_source_links(source_event_id)
    WHERE source_event_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS run_cost_rebuild_receipts (
    projector            TEXT PRIMARY KEY,
    order_cursor         BIGINT NOT NULL DEFAULT 0,
    finality_cursor      BIGINT NOT NULL DEFAULT 0,
    order_high_water     BIGINT NOT NULL DEFAULT 0,
    finality_high_water  BIGINT NOT NULL DEFAULT 0,
    facts_written        BIGINT NOT NULL DEFAULT 0,
    legacy_links_written BIGINT NOT NULL DEFAULT 0,
    updated_at_ms        BIGINT NOT NULL,
    CONSTRAINT run_cost_rebuild_receipts_order_cursor_check CHECK (
        order_cursor >= 0
        AND order_high_water >= 0
        AND order_cursor <= order_high_water
    ),
    CONSTRAINT run_cost_rebuild_receipts_finality_cursor_check CHECK (
        finality_cursor >= 0
        AND finality_high_water >= 0
        AND finality_cursor <= finality_high_water
    ),
    CONSTRAINT run_cost_rebuild_receipts_counts_check CHECK (
        facts_written >= 0 AND legacy_links_written >= 0
    )
);
