#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUDGET="$ROOT/scripts/dependency_budget.tsv"
TMP_DIR="${TMPDIR:-/tmp}/crossline-dependency-budget.$$"
trap 'rm -rf "$TMP_DIR"' EXIT

mkdir -p "$TMP_DIR"

TODAY="${DEPENDENCY_BUDGET_TODAY:-$(date +%F)}"
CURRENT="$TMP_DIR/current.tsv"
BUDGET_KEYS="$TMP_DIR/budget_keys.tsv"
BAD="$TMP_DIR/bad.txt"
: >"$BAD"

emit_workspace_deps() {
  awk '
    /^\[workspace.dependencies\]/{insec=1;next}
    /^\[/{insec=0}
    insec && /^[[:space:]]*[A-Za-z0-9_-]+[[:space:]]*=/ {
      line=$0
      sub(/#.*/, "", line)
      name=line
      sub(/=.*/, "", name)
      gsub(/[[:space:]]/, "", name)
      if (line !~ /path[[:space:]]*=/) print "workspace\t" name
    }
  ' "$ROOT/Cargo.toml"
}

emit_frontend_deps() {
  awk '
    /^\[dependencies\]/{insec=1;next}
    /^\[/{insec=0}
    insec && /^[[:space:]]*[A-Za-z0-9_-]+[[:space:]]*=/ {
      line=$0
      sub(/#.*/, "", line)
      name=line
      sub(/=.*/, "", name)
      gsub(/[[:space:]]/, "", name)
      if (line !~ /path[[:space:]]*=/) print "frontend\t" name
    }
  ' "$ROOT/frontend/Cargo.toml"
}

emit_api_normal_debt() {
  awk '
    /^\[dependencies\]/{insec=1;next}
    /^\[/{insec=0}
    insec && /^[[:space:]]*(llm|options|simulation)[[:space:]]*=/ {
      line=$0
      sub(/#.*/, "", line)
      name=line
      sub(/=.*/, "", name)
      gsub(/[[:space:]]/, "", name)
      if (line !~ /optional[[:space:]]*=[[:space:]]*true/) {
        print "api-normal\t" name
      } else {
        print "api-feature\t" name
      }
    }
  ' "$ROOT/crates/api/Cargo.toml"
}

validate_budget_file() {
  awk -F '\t' -v today="$TODAY" '
    BEGIN { ok = 1 }
    /^#/ || NF == 0 { next }
    NF < 7 {
      printf "FAIL malformed dependency budget line %d: expected 7 tab-separated fields\n", NR
      ok = 0
      next
    }
    $1 !~ /^(workspace|frontend|api-normal|api-feature)$/ {
      printf "FAIL invalid dependency budget scope line %d: %s\n", NR, $1
      ok = 0
    }
    $2 !~ /^[A-Za-z0-9_-]+$/ {
      printf "FAIL invalid dependency name line %d: %s\n", NR, $2
      ok = 0
    }
    $3 !~ /^(core|perf|dev|test|debt)$/ {
      printf "FAIL invalid dependency tier line %d: %s\n", NR, $3
      ok = 0
    }
    $4 == "" || $5 == "" || $6 == "" || $7 == "" {
      printf "FAIL dependency budget line %d must include owner, ticket, expires and reason\n", NR
      ok = 0
    }
    $6 < today {
      printf "FAIL expired dependency budget line %d: %s %s expired %s\n", NR, $1, $2, $6
      ok = 0
    }
    seen[$1 "\t" $2]++ {
      printf "FAIL duplicate dependency budget entry line %d: %s %s\n", NR, $1, $2
      ok = 0
    }
    END { exit ok ? 0 : 1 }
  ' "$BUDGET"
}

validate_api_debt_tier() {
  awk -F '\t' '
    /^#/ || NF == 0 { next }
    $1 == "api-normal" && $3 != "debt" {
      printf "FAIL api-normal dependency must be tier=debt until feature-gated: %s\n", $2
      bad = 1
    }
    $1 == "api-feature" && $3 != "debt" {
      printf "FAIL api-feature dependency must be tier=debt until legacy surface is removed: %s\n", $2
      bad = 1
    }
    END { exit bad ? 1 : 0 }
  ' "$BUDGET"
}

validate_pr_cp_debt_closed() {
  awk -F '\t' '
    !/^#/ && NF >= 7 && $3 == "debt" && $5 == "PR-CP" {
      printf "FAIL PR-CP dependency debt remains: %s %s\n", $1, $2
      bad = 1
    }
    END { exit bad ? 1 : 0 }
  ' "$BUDGET"
}

emit_budget_keys() {
  awk -F '\t' '!/^#/ && NF >= 7 { print $1 "\t" $2 }' "$BUDGET" | sort -u
}

{
  emit_workspace_deps
  emit_frontend_deps
  emit_api_normal_debt
} | sort -u >"$CURRENT"

validate_budget_file
validate_api_debt_tier
validate_pr_cp_debt_closed
emit_budget_keys >"$BUDGET_KEYS"

if comm -23 "$CURRENT" "$BUDGET_KEYS" >"$BAD" && [ -s "$BAD" ]; then
  printf 'FAIL unbudgeted direct dependencies; add owner/ticket/reason to scripts/dependency_budget.tsv\n' >&2
  cat "$BAD" >&2
  exit 1
fi

if comm -13 "$CURRENT" "$BUDGET_KEYS" >"$BAD" && [ -s "$BAD" ]; then
  printf 'FAIL stale dependency budget entries; remove or update scripts/dependency_budget.tsv\n' >&2
  cat "$BAD" >&2
  exit 1
fi

printf 'OK dependency budget gate (%s entries)\n' "$(wc -l <"$CURRENT" | tr -dc '0-9')"
