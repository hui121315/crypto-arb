#!/usr/bin/env bash
set -euo pipefail

# Supply chain & lockfile governance gate (PR-BG).
# Verifies deny.toml structure, Rust lockfile determinism (root + frontend),
# the exact duplicate-version budget (workspace + frontend), the Node dev-only
# policy, and the CI wiring that runs cargo-deny + npm audit.
# Emits a dependency report artifact.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CI="$ROOT/.github/workflows/ci.yml"
DENY="$ROOT/deny.toml"
PKG="$ROOT/package.json"
TMP_DIR="${TMPDIR:-/tmp}/crossline-supply-chain.$$"
trap 'rm -rf "$TMP_DIR"' EXIT
mkdir -p "$TMP_DIR"

TODAY="${SUPPLY_CHAIN_TODAY:-$(date +%F)}"
REPORT="${SUPPLY_CHAIN_REPORT:-$TMP_DIR/dependency-report.txt}"

fail() { printf 'supply chain gate failed: %s\n' "$1" >&2; exit 1; }

# 1. deny.toml structure: advisory/license/source/ban governance must exist.
[ -s "$DENY" ] || fail "missing deny.toml"
for section in advisories licenses bans sources; do
  rg -q "^\[$section\]" "$DENY" || fail "deny.toml missing [$section] section"
done

# 2. Lockfile determinism for both Rust workspaces.
( cd "$ROOT" && cargo metadata --locked --format-version 1 >/dev/null 2>&1 ) \
  || fail "root Cargo.lock not deterministic (cargo metadata --locked failed)"
( cd "$ROOT/frontend" && cargo metadata --locked --format-version 1 >/dev/null 2>&1 ) \
  || fail "frontend Cargo.lock not deterministic (cargo metadata --locked failed)"

# 3. Exact duplicate version-set reconciliation and direct-dependency parity.
DUPLICATE_REPORT="$TMP_DIR/workspace-duplicate-report.tsv"
WORKSPACE_DUPLICATE_BUDGET_TODAY="$TODAY" \
  WORKSPACE_DUPLICATE_REPORT="$DUPLICATE_REPORT" \
  bash "$ROOT/scripts/check_workspace_duplicate_budget.sh"

# 4. Node supply chain: no runtime deps; Playwright/ws stay dev/e2e only.
node - "$PKG" <<'NODE'
const fs = require("fs");
const pkg = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
const deps = pkg.dependencies || {};
const dev = pkg.devDependencies || {};
const errs = [];
if (Object.keys(deps).length) {
  errs.push("package.json must not declare runtime dependencies (browser/e2e tooling is dev-only): " + Object.keys(deps).join(", "));
}
for (const name of ["@playwright/test", "ws"]) {
  if (!dev[name]) errs.push(`${name} must be a devDependency (e2e/browser only)`);
  if (deps[name]) errs.push(`${name} must not be a runtime dependency`);
}
if (errs.length) {
  console.error("supply chain gate failed:\n  " + errs.join("\n  "));
  process.exit(1);
}
console.log("OK node supply chain (0 runtime deps; playwright/ws dev-only)");
NODE

# 5. CI wiring: cargo-deny advisory/license/source/bans + locked npm install + audit.
rg -q 'cargo-deny|deny check' "$CI" || fail "CI must run cargo-deny advisory/license/source/bans"
rg -q 'apt-get install -y ripgrep' "$CI" || fail "CI supply-chain job must install ripgrep before local gate scripts"
rg -q 'npm ci' "$CI" || fail "CI must run npm ci"
rg -q 'npm audit --audit-level=high' "$CI" || fail "CI must run npm audit --audit-level=high"

# 6. Dependency report artifact.
{
  printf '# CROSSLINE Omni dependency report (PR-BG)\n'
  printf 'generated: %s\n\n' "$TODAY"
  printf '## exact duplicate dependency version sets and direct parity\n'
  cat "$DUPLICATE_REPORT"
} >"$REPORT"

printf 'OK supply chain gate (%s duplicate version sets budgeted across workspace+frontend; lockfiles deterministic; node dev-only; report=%s)\n' \
  "$(awk -F '\t' '!/^#/ && NF == 3 { count++ } END { print count + 0 }' "$DUPLICATE_REPORT")" "$REPORT"
