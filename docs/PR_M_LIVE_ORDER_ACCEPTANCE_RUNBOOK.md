# PR-M / PR-FR Live Runtime Acceptance Runbook

Status: 🟡 readiness only. This runbook does not provide a real exchange sample. It is the single procedure for PR-M live place/cancel proof and PR-FR private order-stream proof.

Create the evidence directory before capturing the first sample:

```bash
mkdir -p docs/live_samples
```

## ✅ Local Verifier Contract

Use the verifier before adding any `live-sample-acceptance:<venue>` row:

```bash
python3 scripts/check_live_order_runtime_acceptance.py \
  docs/live_samples/<venue>-venue-operation-health.json \
  --venue <venue> \
  --require-hashed-identity
```

For PR-FR, validate the same redacted capture with:

```bash
python3 scripts/check_private_order_stream_live_sample_acceptance.py \
  docs/live_samples/<venue>-venue-operation-health.json \
  --venue <venue> \
  --require-hashed-identity
```

The PR-FR capture must include a clean same-venue `private_ws_order_stream` companion row with `status=ok`, `source=private_ws_runtime`, `supported=true`, `configured=true`, at least one row, fresh timing, official WS evidence, and no problem/error/retry residue. Its evidence row is named `private-order-stream-live-sample-acceptance:<venue>`.

The captured JSON must be a `/api/system/venue-operation-health` response with
one matching `order_write` row for the target venue plus at least one same-venue
captured companion row such as `order_permission`, `order_finality`, or
`private_ws_order_stream`.

The top-level response envelope must be internally consistent:

- only `rows`, `generatedAtMs`, `rowCount`, `attentionCount`, and optional
  `retryAfterMs` are allowed at the top level
- `rowCount` equals the number of `rows`
- `attentionCount` equals the number of rows whose `status` is not `ok`
- optional `retryAfterMs` is a non-negative integer

The row must prove all of these facts:

- only the accepted `order_write` row schema keys are allowed; optional
  `latencyMs` / `latencyP95Ms` must be numeric when present, while debug/raw
  order response fields and nullable stale `problem` / `error` / `retryAfterMs`
  residue are rejected
- `source=live_order_proof_runtime`
- `status=ok`
- `message` is a non-empty, non-placeholder string
- no stale `problem`, `error`, or `retryAfterMs` fields on the accepted row
- fresh `freshnessMs` within the verifier threshold
- `generatedAtMs - observedAtMs == freshnessMs`
- committed hash-mode samples use plausible Unix epoch millisecond values
  (`>= 1_735_689_600_000`, 2025-01-01 UTC) for `generatedAtMs`,
  `observedAtMs`, `sample_place_checked_at_ms`, and
  `sample_cancel_checked_at_ms`; the same-venue companion operation row also
  carries a plausible Unix epoch `observedAtMs`; these timestamps cannot be
  future-dated beyond the verifier clock plus the allowed skew, and companion
  `observedAtMs` cannot be later than `generatedAtMs`
- `requested=2` and `rows=2`, proving both place and cancel/finality sides exist
- evidence is the backend runtime contract: `method=internal`,
  `path=live_order_proof.runtime`, `authKind=live_order_remote_proof`, empty
  `docUrls` / `rateScopes`, `weight=0`, `useCases` include `order_write` and
  `live_place_cancel_remote_proof`, and `dataKinds` include `order_ack`,
  `cancel_request_ack`, and `cancel_finality`
- `probe_scope=<venue>.order_write.live_place_cancel`
- `live_place_remote_proof=ok`
- `live_cancel_remote_proof=ok`
- `place_ack_count >= 1`, `cancel_finality_count >= 1`, and
  `cancel_requested_count` is present as a non-negative integer
- `sample_place_checked_at_ms` and `sample_cancel_checked_at_ms` are present in
  `evidence.requestContext`, cancel/finality is not earlier than place, and
  `observedAtMs` equals the later of the two sample timestamps
- `sample_place_symbol` and `sample_cancel_symbol` are present, are not generic
  redaction placeholders, and match
- `evidence.requestContext` is an unambiguous `key=value` list with no duplicate
  keys and no blank values
- `evidence.requestId`, when present, matches `sample_place_request_id` or
  `sample_cancel_request_id`
- `evidence.requestId`, `sample_place_request_id`, and
  `sample_cancel_request_id` cannot be generic redaction placeholders
- optional native transport metadata is internally grouped: any
  `sample_*_native_request_id` or `sample_*_native_response_id` requires the
  same-side `sample_*_native_transport`, and native metadata values cannot be
  generic redaction placeholders
- the entire committed artifact cannot carry explicit non-live markers such as
  `paper`, `testnet`, `sandbox`, `demo`, `mock`, `simulated`, or `dry_run`
- matching non-empty stable hashed place/cancel order identity
- `sample_place_source=adapter_ack`
- `sample_cancel_source=order_query`, `private_ws_order`, or
  `private_ws_non_user_cancel` for committed hash-mode samples
- hash identity values must not be low-entropy placeholders such as 64 repeated
  hex characters
- no sensitive keys or values such as API secrets, authorization headers,
  exchange access-key headers, cookies, signatures, passphrases, private keys,
  or bearer tokens

Hash live order identifiers before committing a sample. Use a local salt that is
not committed, and keep the place/cancel hash equal for any one identity family:

```text
sample_place_internal_order_id_hash=hmac-sha256:<64 lowercase hex>
sample_cancel_internal_order_id_hash=hmac-sha256:<64 lowercase hex>
```

The verifier also accepts `sha256:<64 hex>` or 64 lowercase hex. Local transient
checks may omit `--require-hashed-identity` to inspect raw captures before
redaction, but any committed `docs/live_samples/` evidence row must use the flag
and must not contain raw `sample_*_order_id` context fields alongside hashes.
It does not accept arbitrary placeholders such as `[redacted]`, `<redacted>`,
`masked`, `***`, or low-entropy repeated hex because those do not preserve a
credible repeatable equality proof. The verifier also rejects malformed or
duplicate `requestContext` keys so a sample cannot hide conflicting proof values
behind an earlier accepted key, and it rejects inconsistent timestamp fields so
`freshnessMs` cannot be spoofed independently of the captured snapshot time.
Committed samples also reject toy low-epoch timestamps such as `1800` or `1000`
and future-dated timestamps even when the freshness arithmetic and sample
timeline are internally consistent.
If multiple identity families are retained, every family that has both place
and cancel values must be internally consistent; a matching internal hash cannot
hide a conflicting exchange or client hash pair.
Committed samples must keep the runtime sample timestamps after identity
redaction; deleting or rewriting them breaks the proof timeline.
They must also keep the sample symbols because symbol matching is part of the
same-order proof and is not sensitive credential material.

`bash scripts/check_product_audit_evidence_index.sh` independently reruns this
verifier for every future `live-sample-acceptance:<venue>` evidence row. A row
cannot pass by merely naming the verifier command; the TSV command must be the
canonical verifier invocation exactly, the committed artifact must validate for
the matching venue, and the artifact must be git-tracked under
`docs/live_samples/`.

## 🟡 Capture Steps

1. Start the backend in Live mode with the intended venue credentials.
2. Submit a smallest-safe live order under explicit human supervision.
3. Cancel the same order and wait for REST order-query or private WS finality.
4. Fetch `/api/system/venue-operation-health`.
5. Save only the operation-health JSON snapshot after reviewing it for secrets.
6. Run the verifier command above.
7. Add the JSON artifact to git tracking.
8. Add a TSV row named `live-sample-acceptance:<venue>` only after the verifier
   succeeds against the captured real sample. The TSV artifact path must be a
   relative `.json` file contained under `docs/live_samples/`; path traversal
   such as `docs/live_samples/../...` is rejected, and untracked local JSON
   files are rejected by the evidence index. The TSV command must be exactly
   `python3 scripts/check_live_order_runtime_acceptance.py <artifact> --venue <venue> --require-hashed-identity`.

## ❌ Non-Acceptance Cases

Do not mark PR-M complete for any of these:

- self-test fixtures
- one-row verifier fixtures without a same-venue companion operation row
- Paper/Testnet/Sandbox/Demo/Mock samples
- save-time safe cancel/noop/pre-check probes
- runtime rows with `status=warn`, `status=unknown`, or `problem`
- snapshots missing matching place/cancel identity
- snapshots using arbitrary redaction placeholders instead of sha256 identity
  hashes
- committed snapshots that mix raw `sample_*_order_id` fields with hashed
  identity proof
- snapshots where any retained internal/exchange/client place/cancel identity
  family conflicts with another value from the same family
- snapshots whose runtime evidence shape no longer matches the backend
  `live_order_proof.runtime` contract
- snapshots missing runtime proof counters or showing zero place/finality counts
- snapshots missing place/cancel sample checked_at timestamps, with cancel
  earlier than place, or with `observedAtMs` not matching the later sample time
- committed snapshots whose generated/observed/sample checked-at values are
  low-epoch toy timestamps instead of Unix epoch milliseconds
- committed snapshots whose same-venue companion operation row has a low-epoch
  or missing `observedAtMs`
- committed snapshots whose generated/observed/sample checked-at or companion
  timestamps are future-dated beyond the verifier clock plus allowed skew
- committed snapshots whose same-venue companion `observedAtMs` is later than
  the top-level `generatedAtMs`
- snapshots missing place/cancel sample symbols, showing different symbols, or
  using redaction placeholders for symbols
- snapshots whose accepted row message is blank or a redaction placeholder
- snapshots whose `rowCount`, `attentionCount`, or `retryAfterMs` envelope
  fields do not match the captured rows
- snapshots whose accepted `status=ok` row still carries stale `error` or
  `retryAfterMs`
- snapshots with unknown top-level keys, unknown accepted row keys, or nullable
  `problem` / `error` / `retryAfterMs` residue on an otherwise accepted row
- snapshots with blank `requestContext` values, placeholder request ids,
  unmatched `evidence.requestId`, malformed native transport metadata, or
  committed hash-mode `sample_cancel_source=adapter_ack`
- snapshots containing raw credentials, headers, cookies, signatures, or keys
- snapshots containing non-live markers anywhere in the committed artifact
- snapshots captured before a credential update/rotation; saving credentials
  invalidates previous live order proof, so a fresh sample is required

## ✅ Live Acceptance

The live-order path is accepted only when all are true:

- a real venue-specific snapshot is stored in the approved evidence location and
  tracked in Git
- `python3 scripts/check_live_order_runtime_acceptance.py <snapshot> --venue <venue> --require-hashed-identity`
  passes locally
- the corresponding adapter/parser/request tests and runtime acceptance test pass
- the snapshot contains the request correlation, place/cancel and finality evidence
  required by this runbook

Private-order-stream acceptance additionally requires its venue-specific verifier to pass and the
venue-bound JSON to be tracked under the approved live-sample location.
