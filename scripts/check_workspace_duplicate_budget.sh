#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUDGET="${WORKSPACE_DUPLICATE_BUDGET_FILE:-$ROOT/scripts/workspace_duplicate_budget.tsv}"
TMP_DIR="${TMPDIR:-/tmp}/crossline-workspace-duplicates.$$"
TODAY="${WORKSPACE_DUPLICATE_BUDGET_TODAY:-$(date +%F)}"
CARGO_BIN="${CARGO_BIN:-cargo}"
REPORT="${WORKSPACE_DUPLICATE_REPORT:-$TMP_DIR/workspace-duplicate-report.tsv}"
trap 'rm -rf "$TMP_DIR"' EXIT
mkdir -p "$TMP_DIR"

emit_duplicate_version_sets() {
  local scope="$1"
  shift
  LC_ALL=C "$CARGO_BIN" tree "$@" --all-features --prefix none --format '{p}' \
    | sed 's/ (\*)$//' \
    | LC_ALL=C sort -u \
    | awk -v scope="$scope" '
        {
          name = $1
          version = $2
          sub(/^v/, "", version)
          versions[name] = versions[name] (versions[name] ? "," : "") version
          count[name]++
        }
        END {
          for (name in count) {
            if (count[name] > 1) print scope "\t" name "\t" versions[name]
          }
        }
      ' \
    | LC_ALL=C sort -t $'\t' -k1,1 -k2,2
}

emit_direct_version_sets() {
  local scope="$1"
  shift
  LC_ALL=C "$CARGO_BIN" tree "$@" --all-features --depth 1 --prefix depth --format '{p}' \
    | sed 's/ (\*)$//' \
    | awk -v scope="$scope" '
        /^1/ {
          package = $0
          sub(/^1/, "", package)
          split(package, fields, " ")
          if (fields[2] ~ /^v/) {
            version = fields[2]
            sub(/^v/, "", version)
            print scope "\t" fields[1] "\t" version
          }
        }
      ' \
    | LC_ALL=C sort -u \
    | awk -F '\t' '
        {
          versions[$1 FS $2] = versions[$1 FS $2] (versions[$1 FS $2] ? "," : "") $3
        }
        END {
          for (key in versions) print key FS versions[key]
        }
      ' \
    | LC_ALL=C sort -t $'\t' -k1,1 -k2,2
}

validate_budget() {
  [ -f "$BUDGET" ] || {
    printf 'FAIL missing duplicate budget: %s\n' "$BUDGET" >&2
    return 1
  }
  awk -F '\t' -v today="$TODAY" '
    BEGIN { ok = 1 }
    /^#/ || NF == 0 { next }
    NF != 7 { printf "FAIL malformed duplicate budget line %d: expected 7 tab fields\n", NR; ok = 0; next }
    $1 !~ /^(workspace|frontend)$/ { printf "FAIL invalid duplicate scope line %d: %s\n", NR, $1; ok = 0 }
    $2 !~ /^[A-Za-z0-9_.+-]+$/ { printf "FAIL invalid duplicate package line %d: %s\n", NR, $2; ok = 0 }
    $3 !~ /^[0-9][A-Za-z0-9.+-]*(,[0-9][A-Za-z0-9.+-]*)+$/ { printf "FAIL invalid duplicate versions line %d: %s\n", NR, $3; ok = 0 }
    $4 == "" || $5 == "" || $6 == "" || $7 == "" { printf "FAIL duplicate budget line %d requires owner ticket expiry and reason\n", NR; ok = 0 }
    $6 !~ /^[0-9]{4}-[0-9]{2}-[0-9]{2}$/ { printf "FAIL invalid duplicate budget expiry line %d: %s\n", NR, $6; ok = 0 }
    $6 < today { printf "FAIL expired duplicate budget line %d: %s %s\n", NR, $1, $2; ok = 0 }
    seen[$1 "\t" $2]++ { printf "FAIL duplicate duplicate-budget key line %d: %s %s\n", NR, $1, $2; ok = 0 }
    END { exit ok ? 0 : 1 }
  ' "$BUDGET"
}

validate_budget
emit_duplicate_version_sets workspace --workspace >"$TMP_DIR/current-workspace.tsv"
emit_duplicate_version_sets frontend --manifest-path "$ROOT/frontend/Cargo.toml" >"$TMP_DIR/current-frontend.tsv"
LC_ALL=C cat "$TMP_DIR/current-workspace.tsv" "$TMP_DIR/current-frontend.tsv" \
  | LC_ALL=C sort -t $'\t' -k1,1 -k2,2 >"$TMP_DIR/current.tsv"
LC_ALL=C awk -F '\t' '!/^#/ && NF == 7 { print $1 "\t" $2 "\t" $3 }' "$BUDGET" \
  | LC_ALL=C sort -t $'\t' -k1,1 -k2,2 >"$TMP_DIR/budget.tsv"

if ! diff -u "$TMP_DIR/budget.tsv" "$TMP_DIR/current.tsv"; then
  printf 'FAIL workspace/frontend duplicate version sets changed; refresh scripts/workspace_duplicate_budget.tsv for intentional debt reduction or a reviewed new version\n' >&2
  exit 1
fi

emit_direct_version_sets workspace --workspace >"$TMP_DIR/direct-workspace.tsv"
emit_direct_version_sets frontend --manifest-path "$ROOT/frontend/Cargo.toml" >"$TMP_DIR/direct-frontend.tsv"
awk -F '\t' '
  NR == FNR { workspace[$2] = $3; next }
  $2 in workspace {
    parity = workspace[$2] == $3 ? "match" : "mismatch"
    print $2 "\t" workspace[$2] "\t" $3 "\t" parity
  }
' "$TMP_DIR/direct-workspace.tsv" "$TMP_DIR/direct-frontend.tsv" \
  | LC_ALL=C sort -t $'\t' -k1,1 >"$TMP_DIR/direct-parity.tsv"

if ! awk -F '\t' '
  $4 != "match" {
    printf "FAIL direct dependency version mismatch: %s workspace=%s frontend=%s\n", $1, $2, $3
    bad = 1
  }
  END { exit bad ? 1 : 0 }
' "$TMP_DIR/direct-parity.tsv"; then
  exit 1
fi

{
  printf '# scope\tpackage\tversions\n'
  cat "$TMP_DIR/current.tsv"
  printf '\n# package\tworkspace_direct_versions\tfrontend_direct_versions\tparity\n'
  cat "$TMP_DIR/direct-parity.tsv"
} >"$REPORT"

printf 'OK workspace duplicate budget (%s workspace, %s frontend package names)\n' \
  "$(wc -l <"$TMP_DIR/current-workspace.tsv" | tr -dc '0-9')" \
  "$(wc -l <"$TMP_DIR/current-frontend.tsv" | tr -dc '0-9')"
printf 'OK direct dependency parity report: %s\n' "$REPORT"
