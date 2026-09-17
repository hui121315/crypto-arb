#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
INVENTORY="$ROOT/docs/API_ROUTE_INVENTORY.tsv"
AUDIT_ACTIONS="$ROOT/crates/api/src/services/action_runs/audit_log.rs"
AUTH_DENIAL="$ROOT/crates/api/src/services/action_runs/auth_denial.rs"
AUTH_EDGE="$ROOT/crates/api/src/middleware/auth/denial.rs"
AUTH_MIDDLEWARE="$ROOT/crates/api/src/middleware/auth.rs"
REGISTRY="$ROOT/crates/api/src/route_specs.rs"

fail() {
  printf 'mutation audit contract failed: %s\n' "$*" >&2
  exit 1
}

command -v rg >/dev/null 2>&1 || fail "missing command: rg"

require_inventory_row() {
  local method="$1"
  local path="$2"
  local audit_policy="$3"

  awk -F '\t' \
    -v method="$method" \
    -v path="$path" \
    -v audit_policy="$audit_policy" '
      NR == 1 { next }
      $1 == path && ("," $2 ",") ~ ("," method ",") {
        found = 1
        if ($4 != "main_p0") {
          printf "path=%s class=%s expected=main_p0\n", path, $4 > "/dev/stderr"
          bad = 1
        }
        if ($6 != "high") {
          printf "path=%s risk=%s expected=high\n", path, $6 > "/dev/stderr"
          bad = 1
        }
        if ($7 != "bearer") {
          printf "path=%s auth_policy=%s expected=bearer\n", path, $7 > "/dev/stderr"
          bad = 1
        }
        if ($8 != audit_policy) {
          printf "path=%s audit_policy=%s expected=%s\n", path, $8, audit_policy > "/dev/stderr"
          bad = 1
        }
      }
      END {
        if (!found) {
          printf "missing inventory row method=%s path=%s\n", method, path > "/dev/stderr"
          bad = 1
        }
        exit bad
      }
    ' "$INVENTORY" || fail "inventory row is not a high-risk bearer mutation: $method $path"
}

require_action_kind() {
  local label="$1"
  local source="$2"
  local kind="$3"

  rg -q "ActionRunKind::${kind}" "$ROOT/$source" \
    || fail "$label does not reference ActionRunKind::$kind in $source"
}

require_registry_kind() {
  local method="$1"
  local path="$2"
  local audit_policy="$3"
  local kind="$4"
  local pattern

  if [[ "$method" == "POST" || "$method" == "PATCH" ]]; then
    local constructor
    constructor="$(printf '%s' "$method" | tr '[:upper:]' '[:lower:]')_action_run"
    pattern="RouteEndpointSpec::${constructor}\\(\\s*\"${path}\",\\s*\"main_p0\",\\s*\"always\",\\s*\"high\",\\s*\"${audit_policy}\",\\s*ActionRunKind::${kind},?\\s*\\)"
  else
    pattern="path:\\s*\"${path}\",\\s*methods:\\s*\"${method}\",\\s*class:\\s*\"main_p0\",\\s*default_exposure:\\s*\"always\",\\s*risk:\\s*\"high\",\\s*auth_policy:\\s*\"bearer\",\\s*audit_policy:\\s*\"${audit_policy}\",\\s*action_run_kind:\\s*Some\\(ActionRunKind::${kind}\\)"
  fi
  rg -Uq "$pattern" "$REGISTRY" \
    || fail "$method $path does not bind $audit_policy to ActionRunKind::$kind in route_specs.rs"
}

require_audit_action() {
  local kind="$1"
  local action="$2"

  rg -Fq "ActionRunKind::${kind} => \"${action}\"" "$AUDIT_ACTIONS" \
    || fail "ActionRunKind::$kind does not map to audit action $action"
}

require_no_generic_p0_high_risk_audit() {
  awk -F '\t' '
    NR == 1 { next }
    $4 == "main_p0" && $6 == "high" && $8 !~ /^(action_run|secret_mutation)$/ {
      printf "%s %s uses audit_policy=%s\n", $2, $1, $8 > "/dev/stderr"
      bad = 1
    }
    END { exit bad }
  ' "$INVENTORY" || fail "main_p0 high-risk mutations must use action_run or secret_mutation"
}

required_rows=(
  "POST|/api/arbitrage/opportunities/:id/confirm|action_run|crates/api/src/routers/arbitrage/hedge.rs|HedgeConfirm|hedge.confirm"
  "PATCH|/api/automation/config|action_run|crates/api/src/routers/automation.rs|AutomationConfigUpdate|automation.config.update"
  "POST|/api/automation/control|action_run|crates/api/src/routers/automation.rs|AutomationControl|automation.control"
  "POST|/api/exchanges/credentials|secret_mutation|crates/api/src/routers/exchanges.rs|VenueCredentialsUpdate|venue_credentials.update"
  "POST|/api/exchanges/credentials/clear|secret_mutation|crates/api/src/routers/exchanges.rs|VenueCredentialsClear|venue_credentials.clear"
  "POST|/api/exchanges/credentials/migrate|secret_mutation|crates/api/src/routers/exchanges.rs|VenueCredentialsMigrate|venue_credentials.migrate"
  "POST|/api/onchain/credentials|secret_mutation|crates/api/src/routers/onchain.rs|OnchainProviderCredentialsUpdate|onchain.provider_credentials.update"
  "POST|/api/onchain/credentials/clear|secret_mutation|crates/api/src/routers/onchain.rs|OnchainProviderCredentialsClear|onchain.provider_credentials.clear"
  "POST|/api/trading/adapters/select|action_run|crates/api/src/routers/trading/adapters.rs|TradingAdapterSelect|trading.adapter.select"
  "POST|/api/trading/fee-snapshots|action_run|crates/api/src/routers/trading/account.rs|TradingFeeSnapshotUpsert|trading.fee_snapshot.upsert"
  "POST|/api/trading/kill-switch|action_run|crates/api/src/routers/trading/kill_switch.rs|TradingKillSwitch|trading.kill_switch.set"
  "POST|/api/trading/orders|action_run|crates/api/src/routers/trading/orders.rs|TradingOrderSubmit|trading.order.submit"
  "POST|/api/trading/orders/:id/cancel|action_run|crates/api/src/routers/trading/orders.rs|TradingOrderCancel|trading.order.cancel"
  "POST|/api/trading/orders/reconcile|action_run|crates/api/src/routers/trading/orders.rs|TradingOrderReconcile|trading.order.reconcile"
  "POST|/api/trading/portfolio/close-all|action_run|crates/api/src/routers/portfolio.rs|PortfolioCloseAll|portfolio.positions.close_all"
  "POST|/api/trading/portfolio/close-runs/:close_run_id/compensation-orders|action_run|crates/api/src/routers/portfolio.rs|PortfolioCloseCompensation|portfolio.close_run.compensate"
  "POST|/api/trading/portfolio/close-runs/:close_run_id/manual-terminal|action_run|crates/api/src/routers/portfolio.rs|PortfolioCloseManualTerminal|portfolio.close_run.manual_terminal"
  "POST|/api/trading/portfolio/positions/:venue/:symbol/close|action_run|crates/api/src/routers/portfolio.rs|PortfolioClosePosition|portfolio.position.close"
  "POST|/api/trading/portfolio/positions/:venue/:symbol/close-pair|action_run|crates/api/src/routers/portfolio.rs|PortfolioClosePair|portfolio.position.close_pair"
  "PATCH|/api/trading/risk-config|action_run|crates/api/src/routers/trading/account.rs|TradingRiskConfigUpdate|trading.risk_config.update"
  "PATCH|/api/webhook/config|secret_mutation|crates/api/src/routers/webhook.rs|WebhookConfigUpdate|webhook.config.update"
)

for row in "${required_rows[@]}"; do
  IFS='|' read -r method path audit_policy source kind action <<<"$row"
  require_inventory_row "$method" "$path" "$audit_policy"
  require_registry_kind "$method" "$path" "$audit_policy" "$kind"
  require_action_kind "$method $path" "$source" "$kind"
  require_audit_action "$kind" "$action"
done

require_no_generic_p0_high_risk_audit

rg -Fq 'action_runs::record_auth_denial(request.method(), request.uri().path(), &problem)?;' \
  "$AUTH_EDGE" || fail "auth edge does not route pre-handler denials into the ActionRun audit service"
rg -Fq 'Err(error) => return denial::reject(&request, error)' "$AUTH_MIDDLEWARE" \
  || fail "missing or malformed Bearer errors bypass high-risk denial auditing"
rg -Fq 'denial::reject(&request, AppError::Unauthorized("token mismatch".into()))' \
  "$AUTH_MIDDLEWARE" || fail "Bearer mismatch bypasses high-risk denial auditing"
rg -Fq 'audit::record_durable(&event)' "$AUTH_DENIAL" \
  || fail "pre-handler high-risk denial is not durably acknowledged"
rg -Fq 'auth_denial_matrix_covers_every_high_risk_action_route' "$AUTH_DENIAL" \
  || fail "21-route auth-denial matrix test is missing"
rg -Fq 'assert_eq!(route_count, 21);' "$AUTH_DENIAL" \
  || fail "auth-denial matrix no longer locks all 21 active high-risk routes"

inventory_high_risk_count="$(awk -F '\t' '
  NR > 1 && $4 == "main_p0" && $6 == "high" { count += split($2, methods, ",") }
  END { print count + 0 }
' "$INVENTORY")"
[[ "$inventory_high_risk_count" == "${#required_rows[@]}" ]] \
  || fail "matrix covers ${#required_rows[@]} routes but inventory declares $inventory_high_risk_count"

printf 'OK mutation audit contract (%s high-risk routes with runtime and auth-denial audit actions)\n' \
  "${#required_rows[@]}"
