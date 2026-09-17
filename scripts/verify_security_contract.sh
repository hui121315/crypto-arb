#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"

cd "$ROOT"

run_contract() {
  local label="$1"
  shift
  printf 'security %s...\n' "$label"
  "$@"
}

run_contract \
  bind_gate \
  cargo test -p common --lib non_loopback_without_auth_is_rejected --no-fail-fast

run_contract \
  public_bind_cors_gate \
  cargo test -p common --lib non_loopback_with_auth_requires_cors_origins --no-fail-fast

run_contract \
  public_bind_wildcard_cors_gate \
  cargo test -p common --lib non_loopback_with_wildcard_cors_origin_is_rejected --no-fail-fast

run_contract \
  public_bind_audit_gate \
  cargo test -p common --lib non_loopback_with_auth_and_cors_requires_audit_log --no-fail-fast

run_contract \
  auth_exempt_guard \
  cargo test -p common --lib high_risk_api_prefix_cannot_bypass_auth --no-fail-fast

run_contract \
  auth_exempt_segment_match \
  cargo test -p common --lib auth_exemption_matches_only_exact_or_child_paths --no-fail-fast

run_contract \
  request_id_normalizer \
  cargo test -p common --lib request_id --no-fail-fast

run_contract \
  request_id_extension \
  cargo test -p api --bin crypto-arb-api request_extension_carries_normalized_request_id --no-fail-fast

run_contract \
  health_exempt \
  cargo test -p common --lib health_auth_exemption_survives_partial_security_config --no-fail-fast

run_contract \
  auth_cors \
  cargo test -p api --bin crypto-arb-api prod_like_security_contract_rejects_no_auth_and_evil_origin --no-fail-fast

run_contract \
  cors_invalid_fail_closed \
  cargo test -p api --bin crypto-arb-api invalid_configured_cors_origins_do_not_fallback_to_any --no-fail-fast

run_contract \
  action_run_audit \
  cargo test -p api --bin crypto-arb-api kill_switch_action_run_writes_audit_entries --no-fail-fast

run_contract \
  audit_sink_health \
  cargo test -p api --bin crypto-arb-api audit_sink_health --no-fail-fast

run_contract \
  mutation_audit_contract \
  bash scripts/check_mutation_audit_contract.sh

run_contract \
  api_security_runtime_smoke \
  bash scripts/verify_api_security_runtime_smoke.sh

printf 'OK security contracts verified\n'
