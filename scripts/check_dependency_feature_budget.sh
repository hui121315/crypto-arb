#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUDGET="$ROOT/scripts/dependency_feature_budget.tsv"
TMP_DIR="${TMPDIR:-/tmp}/crossline-dependency-feature-budget.$$"
trap 'rm -rf "$TMP_DIR"' EXIT

mkdir -p "$TMP_DIR"

TODAY="${DEPENDENCY_BUDGET_TODAY:-$(date +%F)}"
CURRENT="$TMP_DIR/current.tsv"
BUDGET_KEYS="$TMP_DIR/budget_keys.tsv"
BAD="$TMP_DIR/bad.txt"
: >"$BAD"

emit_manifest_features() {
  local manifest="$1"
  local scope="$2"
  local section="$3"

  awk -v scope="$scope" -v section="$section" '
    function trim(s) {
      gsub(/^[[:space:]]+|[[:space:]]+$/, "", s)
      return s
    }
    function emit_items(text, items, count, i, feature) {
      sub(/#.*/, "", text)
      gsub(/[\[\]]/, "", text)
      count = split(text, items, ",")
      for (i = 1; i <= count; i++) {
        feature = trim(items[i])
        gsub(/^"|"$/, "", feature)
        if (feature != "") {
          print scope "\t" dep "\t" feature
        }
      }
    }
    /^\[/ {
      marker = $0
      gsub(/[[:space:]]/, "", marker)
      in_section = (marker == "[" section "]")
      in_features = 0
      dep = ""
      next
    }
    !in_section { next }
    /^[[:space:]]*[A-Za-z0-9_-]+[[:space:]]*=/ {
      line = $0
      sub(/#.*/, "", line)
      dep = line
      sub(/=.*/, "", dep)
      gsub(/[[:space:]]/, "", dep)
      if (line ~ /path[[:space:]]*=/) {
        dep = ""
        in_features = 0
        next
      }
      if (line ~ /features[[:space:]]*=[[:space:]]*\[/) {
        sub(/.*features[[:space:]]*=[[:space:]]*\[/, "", line)
        if (line ~ /\]/) {
          sub(/\].*/, "", line)
          emit_items(line)
          in_features = 0
        } else {
          emit_items(line)
          in_features = 1
        }
      }
      next
    }
    in_features {
      line = $0
      if (line ~ /\]/) {
        sub(/\].*/, "", line)
        emit_items(line)
        in_features = 0
      } else {
        emit_items(line)
      }
    }
  ' "$manifest"
}

validate_budget_file() {
  awk -F '\t' -v today="$TODAY" '
    BEGIN { ok = 1 }
    /^#/ || NF == 0 { next }
    NF < 8 {
      printf "FAIL malformed dependency feature budget line %d: expected 8 tab-separated fields\n", NR
      ok = 0
      next
    }
    $1 !~ /^(workspace|frontend)$/ {
      printf "FAIL invalid dependency feature scope line %d: %s\n", NR, $1
      ok = 0
    }
    $2 !~ /^[A-Za-z0-9_-]+$/ {
      printf "FAIL invalid dependency name line %d: %s\n", NR, $2
      ok = 0
    }
    $3 !~ /^[A-Za-z0-9_.+-]+$/ {
      printf "FAIL invalid dependency feature line %d: %s\n", NR, $3
      ok = 0
    }
    $4 !~ /^(core|perf|dev|test|debt)$/ {
      printf "FAIL invalid dependency feature tier line %d: %s\n", NR, $4
      ok = 0
    }
    $5 == "" || $6 == "" || $7 == "" || $8 == "" {
      printf "FAIL dependency feature budget line %d must include owner, ticket, expires and reason\n", NR
      ok = 0
    }
    $7 < today {
      printf "FAIL expired dependency feature budget line %d: %s %s/%s expired %s\n", NR, $1, $2, $3, $7
      ok = 0
    }
    seen[$1 "\t" $2 "\t" $3]++ {
      printf "FAIL duplicate dependency feature budget entry line %d: %s %s/%s\n", NR, $1, $2, $3
      ok = 0
    }
    END { exit ok ? 0 : 1 }
  ' "$BUDGET"
}

validate_pr_cp_debt_closed() {
  awk -F '\t' '
    !/^#/ && NF >= 8 && $4 == "debt" && $6 == "PR-CP" {
      printf "FAIL PR-CP dependency feature debt remains: %s %s/%s\n", $1, $2, $3
      bad = 1
    }
    END { exit bad ? 1 : 0 }
  ' "$BUDGET"
}

emit_budget_keys() {
  awk -F '\t' '!/^#/ && NF >= 8 { print $1 "\t" $2 "\t" $3 }' "$BUDGET" | sort -u
}

{
  emit_manifest_features "$ROOT/Cargo.toml" "workspace" "workspace.dependencies"
  emit_manifest_features "$ROOT/frontend/Cargo.toml" "frontend" "dependencies"
} | sort -u >"$CURRENT"

validate_budget_file
validate_pr_cp_debt_closed
emit_budget_keys >"$BUDGET_KEYS"

if comm -23 "$CURRENT" "$BUDGET_KEYS" >"$BAD" && [ -s "$BAD" ]; then
  printf 'FAIL unbudgeted direct dependency features; add owner/ticket/reason to scripts/dependency_feature_budget.tsv\n' >&2
  cat "$BAD" >&2
  exit 1
fi

if comm -13 "$CURRENT" "$BUDGET_KEYS" >"$BAD" && [ -s "$BAD" ]; then
  printf 'FAIL stale dependency feature budget entries; remove or update scripts/dependency_feature_budget.tsv\n' >&2
  cat "$BAD" >&2
  exit 1
fi

printf 'OK dependency feature budget gate (%s entries)\n' "$(wc -l <"$CURRENT" | tr -dc '0-9')"
