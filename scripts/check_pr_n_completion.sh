#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SPEC="$ROOT/test/e2e/pr_co_visual.spec.ts"
HELPER="$ROOT/test/e2e/helpers/pr_co_visual.ts"
BASELINES="$ROOT/test/e2e/pr_co_visual.spec.ts-snapshots"
CONFIG="$ROOT/playwright.config.ts"

BASELINE_FILES=(
  "pr-co-opportunities-desktop.png"
  "pr-co-opportunities-mobile.png"
  "pr-co-review-desktop.png"
  "pr-co-review-mobile.png"
  "pr-co-settings-desktop.png"
  "pr-co-settings-mobile.png"
)

if [[ "${1:-}" == "--self-test" ]]; then
  target="$BASELINES/${BASELINE_FILES[0]}"
  hidden="$target.pr-n-self-test"
  mv "$target" "$hidden"
  trap 'mv "$hidden" "$target"' EXIT
  if bash "$0"; then
    printf 'PR-N completion self-test unexpectedly accepted a missing baseline\n' >&2
    exit 1
  fi
  printf 'PR-N completion self-test passed\n'
  exit 0
fi

for name in "${BASELINE_FILES[@]}"; do
  if [[ ! -s "$BASELINES/$name" ]]; then
    printf 'PR-N visual baseline missing or empty: %s\n' "$name" >&2
    exit 1
  fi
done

count="$(find "$BASELINES" -maxdepth 1 -type f -name '*.png' | wc -l | tr -d ' ')"
if [[ "$count" != "${#BASELINE_FILES[@]}" ]]; then
  printf 'PR-N visual baseline set must contain exactly %s PNGs, found %s\n' \
    "${#BASELINE_FILES[@]}" "$count" >&2
  exit 1
fi

if ! rg -Fq 'snapshotPathTemplate: "{testDir}/{testFilePath}-snapshots/{arg}{ext}"' "$CONFIG"; then
  printf 'PR-N visual snapshots must use the shared cross-runner path template\n' >&2
  exit 1
fi

if ! rg -Fq 'trunk build --release=false && python3 -m http.server' "$CONFIG"; then
  printf 'PR-N browser harness must build frontend assets before serving them\n' >&2
  exit 1
fi

if ! rg -q 'PR-N visual regression baselines remain reproducible' "$SPEC" \
  || ! rg -q 'toHaveScreenshot' "$SPEC" \
  || ! rg -q 'maxDiffPixelRatio' "$SPEC" \
  || ! rg -q 'coScreenshotMasks' "$SPEC" \
  || ! rg -q 'document.fonts' "$HELPER"; then
  printf 'PR-N visual regression contract is incomplete\n' >&2
  exit 1
fi

if rg -n 'test\.skip|PR_CO_VISUAL_BASELINE|--update-snapshots' "$SPEC" "$HELPER"; then
  printf 'PR-N visual regression must not skip or rewrite baselines during verification\n' >&2
  exit 1
fi

bash "$ROOT/scripts/check_frontend_css_generated.sh"
bash "$ROOT/scripts/check_pr_co_visual_harness.sh"
npx playwright test "$SPEC" --list >/dev/null

printf 'OK PR-N style governance completion gate (6 canonical snapshots)\n'
