#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="${TMPDIR:-/tmp}/crossline-product-copy.$$"
RAW="$TMP.raw"
trap 'rm -f "$TMP" "$RAW"' EXIT

if rg -n '"[^"]*(live-readiness|Dry-run|Testnet|US equity reference|RWA Basket|RWA|rwa_basket|三角套利|资金费 Carry|期权-永续|标的事件|池 / 标的|\b(bp|bps)\b)[^"]*"' \
  "$ROOT/frontend/src" \
  --glob '*.rs' >"$RAW" && \
  rg -v 'assert!\(!.*contains|labels\.contains' "$RAW" >"$TMP"; then
  printf 'product copy gate failed: old or unverified user-facing copy in frontend\n' >&2
  cat "$TMP" >&2
  exit 1
fi

if rg -n '"[^"]*(链上后续|链上池子|\b(DEX|dex)\b)[^"]*"' \
  "$ROOT/frontend/src" \
  --glob '*.rs' \
  --glob '!**/panels/modules/onchain/**' \
  --glob '!**/panels/shared/onchain_provider_credentials/**' >"$TMP"; then
  printf 'product copy gate failed: DEX/onchain copy must stay inside canonical onchain surfaces\n' >&2
  cat "$TMP" >&2
  exit 1
fi

if rg -n '"[^"]*\b(Paper|Live)\b[^"]*"' \
  "$ROOT/frontend/src" \
  --glob '*.rs' >"$TMP"; then
  printf 'product copy gate failed: product execution environments must use the shared 模拟/实盘 labels\n' >&2
  cat "$TMP" >&2
  exit 1
fi

if rg -n '"[^"]*(保存 API|验证 API|全部已验证|字段已配置|已配置|未配置|声明支持写入|未声明写入|实现已接|可下单)[^"]*"' \
  "$ROOT/frontend/src/panels/modules/settings" \
  --glob '*.rs' >"$RAW" && \
  rg -v 'assert!\(!.*contains|labels\.contains' "$RAW" >"$TMP"; then
  printf 'product copy gate failed: settings credential copy must separate filled fields from verified permissions\n' >&2
  cat "$TMP" >&2
  exit 1
fi

CR_CREDENTIAL_COPY=(
  "$ROOT/frontend/src/panels/modules/settings/tabs/venue_credentials/format.rs"
  "$ROOT/frontend/src/panels/modules/settings/tabs/venue_credentials/format/status.rs"
  "$ROOT/frontend/src/panels/modules/settings/tabs/venue_credentials/panels.rs"
  "$ROOT/frontend/src/panels/modules/settings/tabs/venue_credentials/panels/operation_health.rs"
)
CR_CREDENTIAL_INPUTS="$ROOT/frontend/src/panels/modules/settings/tabs/venue_credentials/inputs.rs"
CR_CREDENTIAL_VIEW="$ROOT/frontend/src/panels/modules/settings/tabs/venue_credentials/editor.rs"
CR_OPPORTUNITY_EMPTY="$ROOT/frontend/src/panels/modules/opportunity_counts/empty_label.rs"
CR_STRATEGY="$ROOT/shared-types/src/strategy.rs"
CR_OKX_WS="$ROOT/crates/exchange/src/ws/trading.rs"
if ! rg -q '"字段已填写"' "${CR_CREDENTIAL_COPY[@]}" || \
  ! rg -q '"当前可用"' "${CR_CREDENTIAL_COPY[@]}" || \
  ! rg -q '当前状态待运行态证据' "${CR_CREDENTIAL_COPY[@]}"; then
  printf 'product copy gate failed: credential copy must distinguish filled fields, saved validation, and current runtime availability\n' >&2
  exit 1
fi

if rg -n '等待交易所凭证规格' "$ROOT/frontend/src/panels/modules/settings" --glob '*.rs' >"$TMP"; then
  printf 'product copy gate failed: credential specs must expose typed loading/error/missing states\n' >&2
  cat "$TMP" >&2
  exit 1
fi

for marker in \
  'pub(super) enum CredentialSpecState' \
  'CredentialSpecState::Loading' \
  'CredentialSpecState::Error(problem)' \
  'CredentialSpecState::SelectVenue' \
  'CredentialSpecState::MissingSpec' \
  'data-credential-spec-state="error"' \
  'problem_message("读取凭证规格失败", &problem)'; do
  if ! rg -Fq "$marker" "$CR_CREDENTIAL_INPUTS"; then
    printf 'product copy gate failed: missing typed credential-spec marker: %s\n' "$marker" >&2
    exit 1
  fi
done
if ! rg -Fq '!credential_spec_ready.get()' "$CR_CREDENTIAL_VIEW"; then
  printf 'product copy gate failed: credential save must stay disabled without a ready spec\n' >&2
  exit 1
fi

for marker in \
  'LoadState::Loading' \
  'LoadState::Error(problem)' \
  '品种搜索失败' \
  '机会流降级' \
  '当前筛选无匹配' \
  '当前范围暂无'; do
  if ! rg -Fq "$marker" "$CR_OPPORTUNITY_EMPTY"; then
    printf 'product copy gate failed: opportunity empty-state policy lost marker: %s\n' "$marker" >&2
    exit 1
  fi
done

if ! rg -Fq 'pub enum StrategyExposure' "$CR_STRATEGY" || \
  ! rg -Fq 'Self::MainP0' "$CR_STRATEGY" || \
  ! rg -Fq 'is_main_p0_executable' "$CR_STRATEGY" || \
  ! rg -Fq 'Self::SpotCross => "现货跨所"' "$CR_STRATEGY"; then
  printf 'product copy gate failed: main product strategy exposure must stay explicit\n' >&2
  exit 1
fi

if rg -n '全局准入|全局.*readiness|readiness gate' "$CR_OKX_WS" >"$TMP" || \
  ! rg -Fq 'HedgeTicket 执行前校验双腿凭证与能力' "$CR_OKX_WS"; then
  printf 'product copy gate failed: OKX WS note must stay ticket-scoped instead of global readiness copy\n' >&2
  cat "$TMP" >&2
  exit 1
fi

if rg -n '"[^"]*(运行证据完整|权限验证完整|字段已填写[^"]*当前可用|静态[^" ]*(声明|写侧)[^"]*当前可用|当前状态(未验证|降级|不可用)[^"]*当前可用)[^"]*"' \
  "${CR_CREDENTIAL_COPY[@]}" >"$TMP"; then
  printf 'product copy gate failed: credential copy must not present unknown, degraded, blocked, or static evidence as current availability\n' >&2
  cat "$TMP" >&2
  exit 1
fi

if rg -n '保存并验证.*API|具备实盘写单能力|具备私有账户读取能力|Self::OrderWrite => "下单权限"' \
  "$ROOT/crates/api/src/routers/trading/adapters.rs" \
  "$ROOT/shared-types/src/venues/operation_kind_labels.rs" >"$TMP"; then
  printf 'product copy gate failed: API status copy must separate static fields from live readiness\n' >&2
  cat "$TMP" >&2
  exit 1
fi

if rg -n '暂无已填写字段的实盘交易所能力|保存凭证后需要下单/撤单权限探针|凭证与 adapter live_write 声明共同决定' \
  "$ROOT/frontend/src/panels/modules/settings" \
  --glob '*.rs' >"$TMP"; then
  printf 'product copy gate failed: Settings copy must keep credential/static adapter/runtime evidence separate\n' >&2
  cat "$TMP" >&2
  exit 1
fi

if rg -n '"[^"]*待报价[^"]*"' "$ROOT/frontend/src" --glob '*.rs' >"$RAW" && \
  rg -v 'opportunity_format\.rs:' "$RAW" >"$TMP"; then
  printf 'product copy gate failed: missing quote copy must go through the shared explanation helper\n' >&2
  cat "$TMP" >&2
  exit 1
fi

if rg -n '"[^"]*等待数据[^"]*"' "$ROOT/frontend/src" --glob '*.rs' >"$TMP"; then
  printf 'product copy gate failed: empty states must use LoadState/ApiProblem copy, not 等待数据\n' >&2
  cat "$TMP" >&2
  exit 1
fi

printf 'OK product copy gate\n'
