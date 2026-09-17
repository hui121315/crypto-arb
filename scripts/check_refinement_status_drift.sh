#!/usr/bin/env bash
set -euo pipefail

SCRIPT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

case "${1:-}" in
  "") ;;
  --root)
    [[ $# -eq 2 ]] || {
      printf 'usage: %s [--root <repo-root>|--self-test]\n' "$0" >&2
      exit 2
    }
    ROOT="$(cd "$2" && pwd)"
    ;;
  --self-test)
    fixture="$(mktemp -d "${TMPDIR:-/tmp}/crossline-refinement-drift-self-test.XXXXXX")"
    trap 'rm -rf "$fixture"' EXIT
    mkdir -p "$fixture/docs"
    printf '# Product audit\n' >"$fixture/docs/PRODUCT_FULL_AUDIT_REFINEMENT.md"
    printf '# Specialized\n\n> diagnostic snapshot, not 100%% complete\n' \
      >"$fixture/docs/SAMPLE_REFINEMENT.md"
    if bash "$SCRIPT" --root "$fixture" >/dev/null 2>&1; then
      printf 'refinement status drift self-test failed: missing authority link passed\n' >&2
      exit 1
    fi
    printf '# Specialized\n\n> diagnostic snapshot, not 100%% complete; authority: docs/PRODUCT_FULL_AUDIT_REFINEMENT.md\n\n状态：✅ 完成\n' \
      >"$fixture/docs/SAMPLE_REFINEMENT.md"
    if bash "$SCRIPT" --root "$fixture" >/dev/null 2>&1; then
      printf 'refinement status drift self-test failed: optimistic completion passed\n' >&2
      exit 1
    fi
    printf '# Specialized\n\n> diagnostic snapshot, not 100%% complete; authority: docs/PRODUCT_FULL_AUDIT_REFINEMENT.md\n\n状态：🟡 部分完成\n\n- [x] stale completion claim\n' \
      >"$fixture/docs/SAMPLE_REFINEMENT.md"
    if bash "$SCRIPT" --root "$fixture" >/dev/null 2>&1; then
      printf 'refinement status drift self-test failed: legacy checkbox passed\n' >&2
      exit 1
    fi
    printf '# Specialized\n\n> diagnostic snapshot, not 100%% complete; authority: docs/PRODUCT_FULL_AUDIT_REFINEMENT.md\n\n状态：🟡 部分完成\n' \
      >"$fixture/docs/SAMPLE_REFINEMENT.md"
    bash "$SCRIPT" --root "$fixture" >/dev/null
    printf 'OK refinement status drift self-test\n'
    exit 0
    ;;
  *)
    printf 'usage: %s [--root <repo-root>|--self-test]\n' "$0" >&2
    exit 2
    ;;
esac

TMP="${TMPDIR:-/tmp}/crossline-refinement-drift.$$"
trap 'rm -f "$TMP"' EXIT

: >"$TMP"

for file in "$ROOT"/docs/*_REFINEMENT.md; do
  [ -f "$file" ] || continue
  [ "$(basename "$file")" = "PRODUCT_FULL_AUDIT_REFINEMENT.md" ] && continue

  if ! rg -q 'PRODUCT_FULL_AUDIT_REFINEMENT\.md' "$file"; then
    printf '%s: missing current authority link\n' "${file#$ROOT/}" >>"$TMP"
    continue
  fi

  if ! head -40 "$file" | rg -q '专项诊断快照|diagnostic snapshot'; then
    printf '%s: top section must identify the document as a diagnostic snapshot\n' \
      "${file#$ROOT/}" >>"$TMP"
  fi

  rg -n \
    '✅\s*[0-9]+\s*/\s*🟡\s*0\s*/\s*⏳\s*0|状态：✅\s*已完成并收口|状态：✅\s*完成' \
    "$file" \
    | sed "s#^#${file#$ROOT/}:#" >>"$TMP" || true

  rg -n '^\s*[-*]\s+\[[ xX]\]' "$file" \
    | sed "s#^#${file#$ROOT/}:legacy checkbox status marker: #" >>"$TMP" || true

  rg -n '/Users/|/home/[A-Za-z0-9._-]+/' "$file" \
    | sed "s#^#${file#$ROOT/}:foreign absolute path: #" >>"$TMP" || true
done

if [ -s "$TMP" ]; then
  printf 'refinement status drift gate failed: specialized docs need current authority, diagnostic-snapshot truth, repo-local paths, and non-optimistic status markers\n' >&2
  cat "$TMP" >&2
  exit 1
fi

printf 'OK refinement status drift gate\n'
