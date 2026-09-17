ALTER TABLE ledger_projection_jobs
    ADD COLUMN IF NOT EXISTS claim_token TEXT;

ALTER TABLE ledger_projection_jobs
    ADD CONSTRAINT ledger_projection_jobs_status_check
    CHECK (status IN ('pending', 'processing', 'completed'));

ALTER TABLE ledger_projection_jobs
    ADD CONSTRAINT ledger_projection_jobs_attempt_count_check
    CHECK (attempt_count >= 0);

ALTER TABLE ledger_projection_jobs
    ADD CONSTRAINT ledger_projection_jobs_claim_state_check
    CHECK (
        (status = 'pending'
            AND claim_token IS NULL
            AND claimed_at_ms IS NULL
            AND completed_at_ms IS NULL)
        OR (status = 'processing'
            AND claim_token IS NOT NULL
            AND claimed_at_ms IS NOT NULL
            AND completed_at_ms IS NULL)
        OR (status = 'completed'
            AND claim_token IS NOT NULL
            AND claimed_at_ms IS NOT NULL
            AND completed_at_ms IS NOT NULL)
    );

CREATE INDEX IF NOT EXISTS idx_ledger_projection_jobs_claim_pending
    ON ledger_projection_jobs(projector, available_at_ms, event_id)
    WHERE status = 'pending';

CREATE INDEX IF NOT EXISTS idx_ledger_projection_jobs_claim_expired
    ON ledger_projection_jobs(projector, claimed_at_ms, event_id)
    WHERE status = 'processing';
