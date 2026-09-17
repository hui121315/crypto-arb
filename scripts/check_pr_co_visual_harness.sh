#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SPEC="$ROOT/test/e2e/pr_co_visual.spec.ts"
HELPER="$ROOT/test/e2e/helpers/pr_co_visual.ts"
BASELINES="$ROOT/test/e2e/pr_co_visual.spec.ts-snapshots"

for path in "$SPEC" "$HELPER" "$BASELINES/.gitkeep"; do
  if [[ ! -e "$path" ]]; then
    printf 'PR-CO visual harness missing: %s\n' "$path" >&2
    exit 1
  fi
done

if ! rg -q 'toHaveScreenshot' "$SPEC" \
  || ! rg -q 'animations: "disabled"' "$SPEC" \
  || ! rg -q 'caret: "hide"' "$SPEC" \
  || ! rg -q 'scale: "css"' "$SPEC" \
  || ! rg -q 'document.fonts' "$HELPER" \
  || ! rg -q 'rootOverflow' "$HELPER" \
  || ! rg -q 'sticky header position' "$HELPER" \
  || ! rg -q 'collectBrowserErrors' "$HELPER"; then
  printf 'PR-CO visual harness contract is incomplete\n' >&2
  exit 1
fi

if rg -n 'PR_CO_VISUAL_BASELINE|test\.skip|--update-snapshots' "$SPEC" "$HELPER"; then
  printf 'PR-CO visual harness must compare baselines without an opt-out or update path\n' >&2
  exit 1
fi

if ! command -v npx >/dev/null 2>&1; then
  printf 'PR-CO visual harness requires npx\n' >&2
  exit 1
fi

npx playwright test test/e2e/pr_co_visual.spec.ts --list
printf 'OK PR-CO visual harness\n'
