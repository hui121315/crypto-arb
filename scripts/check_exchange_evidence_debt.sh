#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SPEC="$ROOT/crates/exchange/src/venue_spec.rs"
ALLOWLIST="$ROOT/scripts/exchange_evidence_debt_allowlist.tsv"
ACTUAL="${TMPDIR:-/tmp}/crossline-endpoint-evidence-actual.$$"
LISTED="${TMPDIR:-/tmp}/crossline-endpoint-evidence-listed.$$"
ALLOWLIST_ROWS="${TMPDIR:-/tmp}/crossline-endpoint-evidence-allowlist-rows.$$"
REGISTERED="${TMPDIR:-/tmp}/crossline-endpoint-evidence-registered.$$"
BAD="${TMPDIR:-/tmp}/crossline-endpoint-evidence-bad.$$"
TESTS="${TMPDIR:-/tmp}/crossline-endpoint-evidence-tests.$$"
TEST_INCLUDES="${TMPDIR:-/tmp}/crossline-endpoint-evidence-test-includes.$$"
TEST_BODIES="${TMPDIR:-/tmp}/crossline-endpoint-evidence-test-bodies.$$"
TEST_SKIP_MARKERS="${TMPDIR:-/tmp}/crossline-endpoint-evidence-test-skip-markers.$$"
SAFE_PROBES="${TMPDIR:-/tmp}/crossline-safe-probe-endpoints.$$"
DOC_URLS="${TMPDIR:-/tmp}/crossline-endpoint-doc-urls.$$"
trap 'rm -f "$ACTUAL" "$LISTED" "$ALLOWLIST_ROWS" "$REGISTERED" "$BAD" "$TESTS" "$TEST_INCLUDES" "$TEST_BODIES" "$TEST_SKIP_MARKERS" "$SAFE_PROBES" "$DOC_URLS"' EXIT
TODAY="$(date +%F)"

checked_at_is_not_future() {
  local checked_at="$1"
  if [ "$checked_at" = "not_recorded" ]; then
    return 0
  fi
  [[ ! "$checked_at" > "$TODAY" ]]
}

endpoint_fixture_venue_slug() {
  case "$1" in
    Binance) printf 'binance' ;;
    Okx) printf 'okx' ;;
    Bybit) printf 'bybit' ;;
    Bitget) printf 'bitget' ;;
    Gate) printf 'gate' ;;
    GateCrossEx) printf 'gate_crossex' ;;
    Kucoin) printf 'kucoin' ;;
    Hyperliquid) printf 'hyperliquid' ;;
    Kraken) printf 'kraken' ;;
    *) return 1 ;;
  esac
}

endpoint_fixture_matches_venue() {
  local venue="$1"
  local fixture_id="$2"
  local venue_slug
  venue_slug="$(endpoint_fixture_venue_slug "$venue")" || return 1
  case "$fixture_id" in
    "crates/exchange/fixtures/$venue_slug/"*) return 0 ;;
    *) return 1 ;;
  esac
}

if [ ! -s "$ALLOWLIST" ]; then
  printf 'exchange evidence gate failed: missing %s\n' "${ALLOWLIST#$ROOT/}" >&2
  exit 1
fi

perl -0ne '
  while (/EndpointSpec\s*\{\s*venue:\s*VenueId::([A-Za-z0-9_]+),(.*?)\},/sg) {
    $venue = $1;
    next if $venue eq "Htx";
    $body = $2;
    ($method) = $body =~ /method:\s*HttpMethod::([A-Za-z0-9_]+)/;
    ($path) = $body =~ /path:\s*"([^"]+)"/;
    ($use_case) = $body =~ /use_case:\s*EndpointUseCase::([A-Za-z0-9_]+)/;
    ($data_kind) = $body =~ /data_kind:\s*EndpointDataKind::([A-Za-z0-9_]+)/;
    print "$venue\t$method\t$path\t$use_case\t$data_kind\n";
  }
' "$SPEC" | sort >"$ACTUAL"

perl -0ne '
  while (/EndpointSpec\s*\{\s*venue:\s*VenueId::([A-Za-z0-9_]+),(.*?)\},/sg) {
    $venue = $1;
    next if $venue eq "Htx";
    $body = $2;
    ($doc_url) = $body =~ /doc_url:\s*"([^"]+)"/;
    print "$venue\t$doc_url\n";
  }
' "$SPEC" | sort >"$DOC_URLS"

awk -F '\t' -v today="$TODAY" '
  NR == 1 {
    expected = "venue\tmethod\tpath\tuse_case\tdata_kind\tchecked_at\tdoc_version\tschema_hash\tfixture_id\tparser_test\trequest_builder_test\tauth_kind"
    if ($0 != expected) {
      printf "exchange evidence gate failed: bad allowlist header\n" > "/dev/stderr"
      exit 1
    }
    next
  }
  NF != 12 {
    printf "exchange evidence gate failed: %s has %d columns, expected 12\n", $0, NF > "/dev/stderr"
    exit 1
  }
  $1 !~ /^[A-Za-z0-9_]+$/ || $2 !~ /^(Get|Post|Delete)$/ || $3 !~ /^\// {
    printf "exchange evidence gate failed: bad endpoint identity %s\n", $0 > "/dev/stderr"
    exit 1
  }
  $4 !~ /^(HotPathFallback|ColdStart|Baseline|History|Metadata|Calibration|PrivateRead|TradeWrite)$/ {
    printf "exchange evidence gate failed: bad use_case %s\n", $0 > "/dev/stderr"
    exit 1
  }
  $5 !~ /^(OrderBook|ServerTime|InstrumentMetadata|FundingRate|FundingPayment|MarkIndex|OpenInterest|PerpTicker|SpotTicker|OrderAck|OrderStatus|TradeFill|AccountConfig|AccountFeeRate|AccountBalance|AccountPosition)$/ {
    printf "exchange evidence gate failed: bad data_kind %s\n", $0 > "/dev/stderr"
    exit 1
  }
  $6 != "not_recorded" && $6 !~ /^20[0-9][0-9]-[01][0-9]-[0-3][0-9]$/ {
    printf "exchange evidence gate failed: checked_at must be not_recorded or YYYY-MM-DD for %s\n", $0 > "/dev/stderr"
    exit 1
  }
  $6 != "not_recorded" && $6 > today {
    printf "exchange evidence gate failed: checked_at must not be in the future for %s\n", $0 > "/dev/stderr"
    exit 1
  }
  $7 != "not_recorded" && $7 !~ /^[A-Za-z0-9._:\/#+-]+$/ {
    printf "exchange evidence gate failed: bad doc_version %s\n", $0 > "/dev/stderr"
    exit 1
  }
  $8 != "not_recorded" && $8 !~ /^sha256:[0-9a-f]{64}$/ {
    printf "exchange evidence gate failed: schema_hash must be not_recorded or sha256:<64 hex> for %s\n", $0 > "/dev/stderr"
    exit 1
  }
  $9 != "not_recorded" && $9 !~ /^[A-Za-z0-9._:\/-]+$/ {
    printf "exchange evidence gate failed: bad fixture_id %s\n", $0 > "/dev/stderr"
    exit 1
  }
  $10 != "not_recorded" && $10 !~ /^[A-Za-z0-9_:\/.-]+$/ {
    printf "exchange evidence gate failed: bad parser_test %s\n", $0 > "/dev/stderr"
    exit 1
  }
  $11 != "not_recorded" && $11 !~ /^[A-Za-z0-9_:\/.-]+$/ {
    printf "exchange evidence gate failed: bad request_builder_test %s\n", $0 > "/dev/stderr"
    exit 1
  }
  $12 != "not_recorded" && $12 !~ /^(public|api_key|signed|user_address|ws_login|session_token)$/ {
    printf "exchange evidence gate failed: bad auth_kind %s\n", $0 > "/dev/stderr"
    exit 1
  }
  $4 ~ /^(PrivateRead|TradeWrite)$/ && $12 == "public" {
    printf "exchange evidence gate failed: private/trade endpoint cannot be public auth_kind %s\n", $0 > "/dev/stderr"
    exit 1
  }
  { print $1 "\t" $2 "\t" $3 "\t" $4 "\t" $5 }
' "$ALLOWLIST" | sort >"$LISTED"
tail -n +2 "$ALLOWLIST" | sort >"$ALLOWLIST_ROWS"

validate_recorded_fixtures() {
  local venue method path use_case data_kind checked_at doc_version schema_hash fixture_id parser_test request_builder_test auth_kind
  IFS=$'\t' read -r _header
  while IFS=$'\t' read -r venue method path use_case data_kind checked_at doc_version schema_hash fixture_id parser_test request_builder_test auth_kind; do
    if [ "$fixture_id" = "not_recorded" ]; then
      validate_unrecorded_probe_provenance \
        "$venue" "$method" "$path" "$use_case" "$data_kind" \
        "$checked_at" "$doc_version" "$schema_hash" "$fixture_id" \
        "$parser_test" "$request_builder_test" "$auth_kind" || exit 1
      continue
    fi
    case "$fixture_id" in
      crates/exchange/fixtures/*) ;;
      *)
        printf 'exchange evidence gate failed: fixture_id must live under crates/exchange/fixtures for %s %s %s\n' "$venue" "$method" "$path" >&2
        exit 1
        ;;
    esac
    if ! endpoint_fixture_matches_venue "$venue" "$fixture_id"; then
      local venue_slug
      venue_slug="$(endpoint_fixture_venue_slug "$venue" || printf '<unknown>')"
      printf 'exchange evidence gate failed: fixture_id must live under crates/exchange/fixtures/%s for %s %s %s: %s\n' \
        "$venue_slug" "$venue" "$method" "$path" "$fixture_id" >&2
      exit 1
    fi
    if [ "$schema_hash" = "not_recorded" ]; then
      printf 'exchange evidence gate failed: recorded fixture requires schema_hash for %s %s %s\n' "$venue" "$method" "$path" >&2
      exit 1
    fi
    if [ "$parser_test" = "not_recorded" ] || [ "$request_builder_test" = "not_recorded" ]; then
      printf 'exchange evidence gate failed: recorded fixture requires parser/request-builder tests for %s %s %s\n' "$venue" "$method" "$path" >&2
      exit 1
    fi
    if [ "$auth_kind" = "not_recorded" ]; then
      printf 'exchange evidence gate failed: recorded fixture requires auth_kind for %s %s %s\n' "$venue" "$method" "$path" >&2
      exit 1
    fi
    local fixture_path="$ROOT/$fixture_id"
    if [ ! -s "$fixture_path" ]; then
      printf 'exchange evidence gate failed: missing fixture %s\n' "$fixture_id" >&2
      exit 1
    fi
    if ! git -C "$ROOT" ls-files --error-unmatch "$fixture_id" >/dev/null; then
      printf 'exchange evidence gate failed: fixture is not git-tracked %s\n' "$fixture_id" >&2
      exit 1
    fi
    local actual_hash
    actual_hash="sha256:$(shasum -a 256 "$fixture_path" | awk '{print $1}')"
    if [ "$actual_hash" != "$schema_hash" ]; then
      printf 'exchange evidence gate failed: fixture hash mismatch for %s\nexpected %s\nactual   %s\n' "$fixture_id" "$schema_hash" "$actual_hash" >&2
      exit 1
    fi
    if ! has_runnable_test "$parser_test"; then
      printf 'exchange evidence gate failed: missing parser_test %s for %s\n' "$parser_test" "$fixture_id" >&2
      exit 1
    fi
    if ! test_body_has_no_skip_marker "$parser_test"; then
      printf 'exchange evidence gate failed: parser_test %s contains skip/early-return marker for %s\n' "$parser_test" "$fixture_id" >&2
      exit 1
    fi
    if ! parser_test_includes_fixture "$parser_test" "$fixture_id"; then
      printf 'exchange evidence gate failed: parser_test %s does not directly include fixture %s\n' "$parser_test" "$fixture_id" >&2
      exit 1
    fi
    if ! has_runnable_test "$request_builder_test"; then
      printf 'exchange evidence gate failed: missing request_builder_test %s for %s\n' "$request_builder_test" "$fixture_id" >&2
      exit 1
    fi
    if ! test_body_has_no_skip_marker "$request_builder_test"; then
      printf 'exchange evidence gate failed: request_builder_test %s contains skip/early-return marker for %s\n' "$request_builder_test" "$fixture_id" >&2
      exit 1
    fi
    if ! request_builder_test_matches_path "$request_builder_test" "$path"; then
      printf 'exchange evidence gate failed: request_builder_test %s does not prove endpoint path %s for %s\n' \
        "$request_builder_test" "$path" "$fixture_id" >&2
      exit 1
    fi
  done
}

validate_doc_url_pair() {
  local venue="$1"
  local doc_url="$2"
  case "$doc_url" in
    *TODO*|*todo*|*placeholder*|*example.com*|*not_recorded*|"")
      printf 'exchange evidence gate failed: placeholder doc_url for %s: %s\n' "$venue" "$doc_url" >&2
      return 1
      ;;
  esac
  case "$venue" in
    Binance) [[ "$doc_url" == https://developers.binance.com/* ]] ;;
    Okx) [[ "$doc_url" == https://www.okx.com/* ]] ;;
    Bybit) [[ "$doc_url" == https://bybit-exchange.github.io/* ]] ;;
    Bitget) [[ "$doc_url" == https://www.bitget.com/* ]] ;;
    Gate) [[ "$doc_url" == https://www.gate.com/* ]] ;;
    GateCrossEx) [[ "$doc_url" == https://www.gate.com/* ]] ;;
    Kucoin) [[ "$doc_url" == https://www.kucoin.com/* ]] ;;
    Hyperliquid) [[ "$doc_url" == https://hyperliquid.gitbook.io/* ]] ;;
    Kraken) [[ "$doc_url" == https://docs.kraken.com/* ]] ;;
    *)
      printf 'exchange evidence gate failed: unknown venue for doc_url gate: %s\n' "$venue" >&2
      return 1
      ;;
  esac || {
    printf 'exchange evidence gate failed: non-official doc_url for %s: %s\n' "$venue" "$doc_url" >&2
    return 1
  }
}

validate_endpoint_doc_urls() {
  local venue doc_url
  while IFS=$'\t' read -r venue doc_url; do
    validate_doc_url_pair "$venue" "$doc_url" || exit 1
  done <"$DOC_URLS"
}

safe_probe_identities() {
  cat <<'EOF'
Binance	Post	/fapi/v1/order/test	TradeWrite	OrderAck
Binance	Delete	/fapi/v1/order	TradeWrite	OrderAck
Okx	Post	/api/v5/trade/order-precheck	TradeWrite	OrderAck
Bybit	Post	/v5/order/pre-check	TradeWrite	OrderAck
Bitget	Post	/api/v3/trade/cancel-order	TradeWrite	OrderAck
Gate	Delete	/api/v4/futures/usdt/orders/{order_id}	TradeWrite	OrderAck
Kucoin	Post	/api/v1/orders/test	TradeWrite	OrderAck
Kucoin	Delete	/api/v1/orders/client-order/{clientOid}	TradeWrite	OrderAck
Hyperliquid	Post	/exchange	TradeWrite	OrderAck
EOF
}

is_allowed_unrecorded_probe_identity() {
  local venue="$1"
  local method="$2"
  local path="$3"
  local use_case="$4"
  local data_kind="$5"
  safe_probe_identities | grep -Fxq -- "$venue"$'\t'"$method"$'\t'"$path"$'\t'"$use_case"$'\t'"$data_kind"
}

validate_unrecorded_probe_provenance() {
  local venue="$1"
  local method="$2"
  local path="$3"
  local use_case="$4"
  local data_kind="$5"
  local checked_at="$6"
  local doc_version="$7"
  local schema_hash="$8"
  local fixture_id="$9"
  local parser_test="${10}"
  local request_builder_test="${11}"
  local auth_kind="${12}"

  if [ "$fixture_id" != "not_recorded" ]; then
    return 0
  fi
  if ! is_allowed_unrecorded_probe_identity "$venue" "$method" "$path" "$use_case" "$data_kind"; then
    printf 'exchange evidence gate failed: unrecorded probe row is not in the explicit safe-probe identity list: %s %s %s %s/%s\n' \
      "$venue" "$method" "$path" "$use_case" "$data_kind" >&2
    return 1
  fi
  if [ "$use_case" != "TradeWrite" ] && [ "$use_case" != "PrivateRead" ]; then
    printf 'exchange evidence gate failed: unrecorded fixture is only allowed for private/trade probe rows: %s %s %s %s/%s\n' \
      "$venue" "$method" "$path" "$use_case" "$data_kind" >&2
    return 1
  fi
  if [ "$checked_at" = "not_recorded" ] || [ "$doc_version" = "not_recorded" ] || [ "$auth_kind" = "not_recorded" ]; then
    printf 'exchange evidence gate failed: unrecorded probe row requires checked_at/doc_version/auth_kind provenance for %s %s %s\n' \
      "$venue" "$method" "$path" >&2
    return 1
  fi
  if [ "$schema_hash" != "not_recorded" ] || [ "$parser_test" != "not_recorded" ]; then
    printf 'exchange evidence gate failed: unrecorded probe row must keep schema_hash and parser_test as not_recorded for %s %s %s\n' \
      "$venue" "$method" "$path" >&2
    return 1
  fi
  if [ "$request_builder_test" = "not_recorded" ]; then
    printf 'exchange evidence gate failed: unrecorded probe row requires request_builder_test evidence for %s %s %s\n' \
      "$venue" "$method" "$path" >&2
    return 1
  fi
  if ! has_runnable_test "$request_builder_test"; then
    printf 'exchange evidence gate failed: missing request_builder_test %s for unrecorded probe row %s %s %s\n' \
      "$request_builder_test" "$venue" "$method" "$path" >&2
    return 1
  fi
  if ! test_body_has_no_skip_marker "$request_builder_test"; then
    printf 'exchange evidence gate failed: request_builder_test %s contains skip/early-return marker for unrecorded probe row %s %s %s\n' \
      "$request_builder_test" "$venue" "$method" "$path" >&2
    return 1
  fi
  if ! request_builder_test_matches_path "$request_builder_test" "$path"; then
    printf 'exchange evidence gate failed: request_builder_test %s does not prove endpoint path %s for unrecorded probe row %s %s %s\n' \
      "$request_builder_test" "$path" "$venue" "$method" "$path" >&2
    return 1
  fi
}

hyperliquid_info_duplicate_operation_evidence_is_present() {
  local expected actual missing
  expected="$(cat <<'EOF'
Info	clearinghouseState	OptionalPerpDex	PrivateRead	AccountBalance
Info	spotClearinghouseState	Spot	PrivateRead	AccountBalance
Info	openOrders	OptionalPerpDex	PrivateRead	OrderStatus
Info	frontendOpenOrders	OptionalPerpDex	PrivateRead	OrderStatus
Info	orderStatus	NotDexScoped	PrivateRead	OrderStatus
EOF
)"
  actual="$(
    perl -0ne '
      my ($block) = /const\s+HYPERLIQUID_OPERATION_EVIDENCE:\s*&\[HyperliquidOperationEvidence\]\s*=\s*&\[(.*?)\];/s;
      exit 0 unless defined $block;
      while ($block =~ /HyperliquidOperationEvidence\s*\{\s*transport:\s*HyperliquidOperationTransport::([A-Za-z0-9_]+),\s*operation:\s*"([^"]+)",\s*dex_scope:\s*HyperliquidDexScope::([A-Za-z0-9_]+),\s*doc_url:\s*[^,]+,\s*use_case:\s*EndpointUseCase::([A-Za-z0-9_]+),\s*data_kind:\s*EndpointDataKind::([A-Za-z0-9_]+),\s*meta:\s*EndpointEvidenceMeta\s*\{.*?\},\s*\},/sg) {
        print join("\t", $1, $2, $3, $4, $5), "\n";
      }
    ' "$SPEC" | sort
  )"
  missing="$(
    comm -23 \
      <(printf '%s\n' "$expected" | sort) \
      <(printf '%s\n' "$actual")
  )"
  if [ -n "$missing" ]; then
    printf 'exchange evidence gate failed: Hyperliquid /info duplicate exception requires operation-scoped registry evidence\n%s\n' \
      "$missing" >&2
    return 1
  fi
}

is_allowed_hyperliquid_info_duplicate_identity() {
  local venue="$1"
  local method="$2"
  local path="$3"
  local use_case="$4"
  local data_kind="$5"
  local count="$6"
  case "$venue:$method:$path:$use_case:$data_kind:$count" in
    Hyperliquid:Post:/info:PrivateRead:AccountBalance:2|Hyperliquid:Post:/info:PrivateRead:OrderStatus:3)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

require_unique_endpoint_identities() {
  local label="$1"
  local file="$2"
  local duplicates rejected=""
  local duplicate_count duplicate_rows venue method path use_case data_kind
  local hyperliquid_evidence_checked=0
  local hyperliquid_evidence_ok=0
  duplicates="$(
    awk -F '\t' '
      {
        key = $1 FS $2 FS $3 FS $4 FS $5
        count[key]++
        rows[key] = rows[key] (rows[key] == "" ? "" : ",") NR
      }
      END {
        for (key in count) {
          if (count[key] > 1) {
            print count[key] "\t" rows[key] "\t" key
            found = 1
          }
        }
      }
    ' "$file" || true
  )"
  if [ -z "$duplicates" ]; then
    return 0
  fi
  while IFS=$'\t' read -r duplicate_count duplicate_rows venue method path use_case data_kind; do
    if is_allowed_hyperliquid_info_duplicate_identity \
      "$venue" "$method" "$path" "$use_case" "$data_kind" "$duplicate_count"; then
      if [ "$hyperliquid_evidence_checked" -eq 0 ]; then
        hyperliquid_evidence_checked=1
        if hyperliquid_info_duplicate_operation_evidence_is_present; then
          hyperliquid_evidence_ok=1
        fi
      fi
      if [ "$hyperliquid_evidence_ok" -eq 1 ]; then
        continue
      fi
    fi
    rejected+="${rejected:+$'\n'}${duplicate_rows}"$'\t'"${venue}"$'\t'"${method}"$'\t'"${path}"$'\t'"${use_case}"$'\t'"${data_kind}"
  done <<<"$duplicates"

  if [ -n "$rejected" ]; then
    printf 'exchange evidence gate failed: duplicate %s endpoint identities\n%s\n' \
      "$label" "$rejected" >&2
    return 1
  fi
}

extract_recorded_endpoint_evidence() {
  local spec_file="$1"
  perl -0ne '
    my %const = (UNRECORDED_EVIDENCE_MARKER => "not_recorded");
    while (/(?:pub\s+)?const\s+([A-Z0-9_]+):\s*&(?:\x27static\s+)?str\s*=\s*"([^"]*)";/sg) {
      $const{$1} = $2;
    }

    sub value_of {
      my ($expr) = @_;
      $expr =~ s/^\s+|\s+$//g;
      return $1 if $expr =~ /^"([^"]*)"$/;
      return $const{$expr} if exists $const{$expr};
      return "UNRESOLVED:$expr";
    }

    my ($block) = /const\s+RECORDED_ENDPOINT_EVIDENCE:\s*&\[EndpointEvidenceEntry\]\s*=\s*&\[(.*?)\];/s;
    die "missing RECORDED_ENDPOINT_EVIDENCE\n" unless defined $block;

    while ($block =~ /EndpointEvidenceEntry\s*\{\s*venue:\s*VenueId::([A-Za-z0-9_]+),\s*method:\s*HttpMethod::([A-Za-z0-9_]+),\s*path:\s*"([^"]+)",\s*use_case:\s*EndpointUseCase::([A-Za-z0-9_]+),\s*data_kind:\s*EndpointDataKind::([A-Za-z0-9_]+),\s*meta:\s*EndpointEvidenceMeta\s*\{\s*checked_at:\s*([^,]+),\s*doc_version:\s*([^,]+),\s*schema_hash:\s*([^,]+),\s*fixture_id:\s*([^,]+),\s*parser_test:\s*([^,]+),\s*request_builder_test:\s*([^,]+),\s*auth_kind:\s*([^,]+),\s*\},\s*\},/sg) {
      my $venue = $1;
      next if $venue eq "Htx";
      print join("\t", $venue, $2, $3, $4, $5, value_of($6), value_of($7), value_of($8), value_of($9), value_of($10), value_of($11), value_of($12)), "\n";
    }

    my ($operation_block) = /const\s+HYPERLIQUID_OPERATION_EVIDENCE:\s*&\[HyperliquidOperationEvidence\]\s*=\s*&\[(.*?)\];/s;
    die "missing HYPERLIQUID_OPERATION_EVIDENCE\n" unless defined $operation_block;

    while ($operation_block =~ /HyperliquidOperationEvidence\s*\{\s*transport:\s*HyperliquidOperationTransport::([A-Za-z0-9_]+),\s*operation:\s*"[^"]+",\s*dex_scope:\s*HyperliquidDexScope::[A-Za-z0-9_]+,\s*doc_url:\s*[^,]+,\s*use_case:\s*EndpointUseCase::([A-Za-z0-9_]+),\s*data_kind:\s*EndpointDataKind::([A-Za-z0-9_]+),\s*meta:\s*EndpointEvidenceMeta\s*\{\s*checked_at:\s*([^,]+),\s*doc_version:\s*([^,]+),\s*schema_hash:\s*([^,]+),\s*fixture_id:\s*([^,]+),\s*parser_test:\s*([^,]+),\s*request_builder_test:\s*([^,]+),\s*auth_kind:\s*([^,]+),\s*\},\s*\},/sg) {
      my ($transport, $use_case, $data_kind, $checked_at, $doc_version, $schema_hash, $fixture_id, $parser_test, $request_builder_test, $auth_kind) =
        ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10);
      next unless $transport eq "Info" && $use_case eq "PrivateRead";
      print join("\t", "Hyperliquid", "Post", "/info", $use_case, $data_kind, value_of($checked_at), value_of($doc_version), value_of($schema_hash), value_of($fixture_id), value_of($parser_test), value_of($request_builder_test), value_of($auth_kind)), "\n";
    }
  ' "$spec_file"
}

require_endpoint_metadata_match() {
  local registered="$1"
  local allowlist="$2"
  if grep -n 'UNRESOLVED:' "$registered" >"$BAD"; then
    printf 'exchange evidence gate failed: unresolved EndpointEvidenceEntry metadata constants\n' >&2
    cat "$BAD" >&2
    return 1
  fi
  if ! diff -u "$registered" "$allowlist" >"$BAD"; then
    printf 'exchange evidence gate failed: RECORDED_ENDPOINT_EVIDENCE metadata must match scripts/exchange_evidence_debt_allowlist.tsv\n' >&2
    cat "$BAD" >&2
    return 1
  fi
}

validate_registered_endpoint_metadata() {
  extract_recorded_endpoint_evidence "$SPEC" | sort >"$REGISTERED"
  require_endpoint_metadata_match "$REGISTERED" "$ALLOWLIST_ROWS"
}

build_test_registry_from_paths() {
  : >"$TESTS"
  : >"$TEST_INCLUDES"
  : >"$TEST_BODIES"
  : >"$TEST_SKIP_MARKERS"
  CROSSLINE_ROOT="$ROOT" CROSSLINE_TESTS="$TESTS" CROSSLINE_TEST_INCLUDES="$TEST_INCLUDES" CROSSLINE_TEST_BODIES="$TEST_BODIES" CROSSLINE_TEST_SKIP_MARKERS="$TEST_SKIP_MARKERS" xargs perl -Mstrict -Mwarnings -MFile::Basename=dirname -MFile::Spec -MCwd=abs_path -e '
    open my $tests_out, ">", $ENV{CROSSLINE_TESTS} or die "$ENV{CROSSLINE_TESTS}: $!";
    open my $includes_out, ">", $ENV{CROSSLINE_TEST_INCLUDES} or die "$ENV{CROSSLINE_TEST_INCLUDES}: $!";
    open my $bodies_out, ">", $ENV{CROSSLINE_TEST_BODIES} or die "$ENV{CROSSLINE_TEST_BODIES}: $!";
    open my $skip_out, ">", $ENV{CROSSLINE_TEST_SKIP_MARKERS} or die "$ENV{CROSSLINE_TEST_SKIP_MARKERS}: $!";

    sub normalize_include {
      my ($root, $rel_path, $path, $include_path) = @_;
      my $abs = abs_path(File::Spec->rel2abs($include_path, dirname($path)));
      return $include_path unless defined $abs;
      if ($root ne "" && index($abs, "$root/") == 0) {
        $abs =~ s/^\Q$root\E\///;
        return $abs;
      }
      return $abs;
    }

    sub remove_block_comments {
      my ($source) = @_;
      my ($out, $depth) = ("", 0);
      for (my $i = 0; $i < length($source); $i++) {
        my $ch = substr($source, $i, 1);
        my $next = $i + 1 < length($source) ? substr($source, $i + 1, 1) : "";
        if ($depth > 0) {
          if ($ch eq "/" && $next eq "*") {
            $depth++;
            $i++;
          } elsif ($ch eq "*" && $next eq "/") {
            $depth--;
            $i++;
          } elsif ($ch eq "\n") {
            $out .= "\n";
          }
          next;
        }
        if ($ch eq "/" && $next eq "*") {
          $depth = 1;
          $i++;
          next;
        }
        $out .= $ch;
      }
      return $out;
    }

    sub trimmed {
      my ($line) = @_;
      $line =~ s/^\s+|\s+$//g;
      return $line;
    }

    sub attr_blocks_before {
      my ($lines, $index) = @_;
      my @attrs;
      my $j = $index - 1;
      while ($j >= 0) {
        my $line = trimmed($lines->[$j]);
        if ($line eq "") {
          $j--;
          next;
        }
        if ($line =~ /^\/\//) {
          $j--;
          next;
        }
        last unless $line =~ /^#\[/ || $line =~ /[\]\)]\s*$/;
        my @block;
        if ($line =~ /^#\[/) {
          unshift @block, $line;
          $j--;
        } else {
          while ($j >= 0) {
            my $part = trimmed($lines->[$j]);
            last if $part eq "" && !@block;
            unshift @block, $part;
            my $started = $part =~ /^#\[/;
            $j--;
            last if $started;
          }
        }
        my $attr = join(" ", @block);
        last unless $attr =~ /^#\[/;
        unshift @attrs, $attr;
      }
      return @attrs;
    }

	    sub test_attr_state {
	      my @attrs = @_;
	      my ($saw_test, $invalid_attr) = (0, 0);
	      for my $attr (@attrs) {
	        $saw_test = 1 if $attr =~ /^#\[\s*(?:tokio::)?test(?:\]|\()/;
        $invalid_attr = 1
          if $attr =~ /^#\[\s*(?:ignore|should_panic)\b/
          || $attr =~ /^#\[\s*cfg\s*\(/
          || $attr =~ /^#\[\s*cfg_attr\s*\(.*(?:\b(?:ignore|should_panic)\b|\bcfg\s*\()/s;
      }
	      return ($saw_test, $invalid_attr);
	    }

	    sub module_attr_disables_scope {
	      my @attrs = @_;
	      for my $attr (@attrs) {
	        next if $attr =~ /^#\[\s*cfg\s*\(\s*test\s*\)\s*\]$/;
	        return 1
	          if $attr =~ /^#\[\s*(?:ignore|should_panic)\b/
	          || $attr =~ /^#\[\s*cfg\s*\(/
	          || $attr =~ /^#\[\s*cfg_attr\s*\(.*(?:\b(?:ignore|should_panic)\b|\bcfg\s*\()/s;
	      }
	      return 0;
	    }

    sub brace_scan_line {
      my ($line) = @_;
      $line =~ s/r#+".*?"#+/""/g;
      $line =~ s/"(?:\\.|[^"\\])*"/""/g;
      $line =~ s{//.*$}{};
      return $line;
    }

    sub body_has_skip_marker {
      my ($body) = @_;
      my $scan = $body;
      $scan =~ s/r#+".*?"#+/""/gs;
      $scan =~ s/r".*?"/""/gs;
      $scan =~ s/"(?:\\.|[^"\\])*"/""/gs;
      return 1
        if $scan =~ /\b(?:std::env::temp_dir|tempfile|load_or_skip)\b/
        || $scan =~ m{/tmp/}
        || $scan =~ /\breturn\s+Ok\s*\(\s*\(\s*\)\s*\)/
        || $scan =~ /\breturn\s*;/;
      return 0;
    }

    for my $path (@ARGV) {
      open my $fh, "<", $path or die "$path: $!";
      my $root = $ENV{CROSSLINE_ROOT} // "";
      my $rel_path = $path;
      $rel_path =~ s/^\Q$root\E\/// if $root ne "";
	      my $source = remove_block_comments(do { local $/; <$fh> });
	      my @lines = split /\n/, $source, -1;
	      my @disabled_module_depths;
	      my $scope_depth = 0;
	      for (my $i = 0; $i <= $#lines; $i++) {
	        while (@disabled_module_depths && $scope_depth < $disabled_module_depths[-1]) {
	          pop @disabled_module_depths;
	        }
	        my $inside_disabled_parent = @disabled_module_depths > 0;
	        my $depth_line = brace_scan_line($lines[$i]);
	        my $opens = () = $depth_line =~ /\{/g;
	        my $closes = () = $depth_line =~ /\}/g;
	        if ($lines[$i] =~ /^\s*(?:pub(?:\s*\([^)]*\))?\s+)?mod\s+[A-Za-z0-9_]+\s*\{/) {
	          my $module_depth = $scope_depth + $opens - $closes;
	          if ($module_depth > $scope_depth &&
	              ($inside_disabled_parent || module_attr_disables_scope(attr_blocks_before(\@lines, $i)))) {
	            push @disabled_module_depths, $module_depth;
	          }
	        }
	        if ($inside_disabled_parent || $lines[$i] !~ /^\s*(?:async\s+)?fn\s+([A-Za-z0-9_]+)\s*\(/) {
	          $scope_depth += $opens - $closes;
	          next;
	        }
	        my $test_name = $1;
	        my ($saw_test, $invalid_attr) = test_attr_state(attr_blocks_before(\@lines, $i));
	        if (!$saw_test || $invalid_attr) {
	          $scope_depth += $opens - $closes;
	          next;
	        }
	        print {$tests_out} "$test_name\t$rel_path\n";

        my ($body, $brace_depth, $started) = ("", 0, 0);
        for (my $k = $i; $k <= $#lines; $k++) {
          my $line = $lines[$k];
          $body .= "$line\n";
          my $depth_line = brace_scan_line($line);
          my $opens = () = $depth_line =~ /\{/g;
          my $closes = () = $depth_line =~ /\}/g;
          $started = 1 if $opens > 0;
          $brace_depth += $opens - $closes if $started;
          last if $started && $brace_depth <= 0;
        }
        $body =~ s{//.*$}{}mg;
        $body =~ s{/[*].*?[*]/}{}sg;
        print {$skip_out} "$test_name\t$rel_path\n" if body_has_skip_marker($body);
        while ($body =~ /include_(?:str|bytes)!\(\s*"([^"]+)"\s*\)/sg) {
          my $fixture = normalize_include($root, $rel_path, $path, $1);
          print {$includes_out} "$test_name\t$rel_path\t$fixture\n";
        }
	        $body =~ s/[\t\r\n]+/ /g;
	        $body =~ s/\s{2,}/ /g;
	        print {$bodies_out} "$test_name\t$rel_path\t$body\n";
	        $scope_depth += $opens - $closes;
	        while (@disabled_module_depths && $scope_depth < $disabled_module_depths[-1]) {
	          pop @disabled_module_depths;
	        }
	      }
	    }
	  '
	  if awk -F '\t' '
	    {
	      key = $1 FS $2
	      count[key]++
	    }
	    END {
	      for (key in count) {
	        if (count[key] > 1) {
	          print key > "/dev/stderr"
	          found = 1
	        }
	      }
	      exit found ? 0 : 1
	    }
	  ' "$TESTS"; then
	    printf 'exchange evidence gate failed: duplicate same-file test names; split evidence refs before registry use\n' >&2
	    return 1
	  fi
	  sort -u "$TESTS" -o "$TESTS"
  sort -u "$TEST_INCLUDES" -o "$TEST_INCLUDES"
  sort -u "$TEST_BODIES" -o "$TEST_BODIES"
  sort -u "$TEST_SKIP_MARKERS" -o "$TEST_SKIP_MARKERS"

  if [ ! -s "$TESTS" ]; then
    printf 'exchange evidence gate failed: no runnable exchange tests discovered\n' >&2
    exit 1
  fi
}

build_test_registry() {
  rg --files "$ROOT/crates/exchange/src" "$ROOT/crates/exchange/tests" -g '*.rs' \
    | build_test_registry_from_paths
}

has_runnable_test() {
  local test_ref="$1"
  local test_name test_path match_count
  if [[ "$test_ref" == *::* ]]; then
    test_path="${test_ref%::*}"
    test_name="${test_ref##*::}"
    if [ -z "$test_path" ] || [ -z "$test_name" ]; then
      printf 'exchange evidence gate failed: bad file-bound test reference %s\n' "$test_ref" >&2
      return 1
    fi
    grep -Fxq -- "$test_name"$'\t'"$test_path" "$TESTS"
    return $?
  fi

  test_name="$test_ref"
  match_count="$(
    awk -F '\t' -v test_name="$test_name" '$1 == test_name { count++ } END { print count + 0 }' "$TESTS"
  )"
  case "$match_count" in
    0)
      return 1
      ;;
    1)
      return 0
      ;;
    *)
      printf 'exchange evidence gate failed: ambiguous test reference %s; use <path>::%s. matches:\n' \
        "$test_ref" "$test_ref" >&2
      awk -F '\t' -v test_name="$test_name" \
        '$1 == test_name { printf "  %s::%s\n", $2, $1 > "/dev/stderr" }' "$TESTS"
      return 1
      ;;
  esac
}

parser_test_includes_fixture() {
  local test_ref="$1"
  local fixture_id="$2"
  local test_name test_path match_count
  if [[ "$test_ref" == *::* ]]; then
    test_path="${test_ref%::*}"
    test_name="${test_ref##*::}"
    grep -Fxq -- "$test_name"$'\t'"$test_path"$'\t'"$fixture_id" "$TEST_INCLUDES"
    return $?
  fi

  test_name="$test_ref"
  match_count="$(
    awk -F '\t' -v test_name="$test_name" '$1 == test_name { count++ } END { print count + 0 }' "$TESTS"
  )"
  case "$match_count" in
    1)
      test_path="$(awk -F '\t' -v test_name="$test_name" '$1 == test_name { print $2; exit }' "$TESTS")"
      grep -Fxq -- "$test_name"$'\t'"$test_path"$'\t'"$fixture_id" "$TEST_INCLUDES"
      ;;
    *)
      return 1
      ;;
  esac
}

test_body_has_no_skip_marker() {
  local test_ref="$1"
  local test_name test_path match_count
  if [[ "$test_ref" == *::* ]]; then
    test_path="${test_ref%::*}"
    test_name="${test_ref##*::}"
    ! grep -Fxq -- "$test_name"$'\t'"$test_path" "$TEST_SKIP_MARKERS"
    return $?
  fi

  test_name="$test_ref"
  match_count="$(
    awk -F '\t' -v test_name="$test_name" '$1 == test_name { count++ } END { print count + 0 }' "$TESTS"
  )"
  case "$match_count" in
    1)
      test_path="$(awk -F '\t' -v test_name="$test_name" '$1 == test_name { print $2; exit }' "$TESTS")"
      ! grep -Fxq -- "$test_name"$'\t'"$test_path" "$TEST_SKIP_MARKERS"
      ;;
    *)
      return 1
      ;;
  esac
}

test_body_for_ref() {
  local test_ref="$1"
  local test_name test_path match_count
  if [[ "$test_ref" == *::* ]]; then
    test_path="${test_ref%::*}"
    test_name="${test_ref##*::}"
    awk -F '\t' -v test_name="$test_name" -v test_path="$test_path" \
      '$1 == test_name && $2 == test_path { print $3; found = 1; exit } END { exit found ? 0 : 1 }' \
      "$TEST_BODIES"
    return $?
  fi

  test_name="$test_ref"
  match_count="$(
    awk -F '\t' -v test_name="$test_name" '$1 == test_name { count++ } END { print count + 0 }' "$TESTS"
  )"
  case "$match_count" in
    1)
      test_path="$(awk -F '\t' -v test_name="$test_name" '$1 == test_name { print $2; exit }' "$TESTS")"
      awk -F '\t' -v test_name="$test_name" -v test_path="$test_path" \
        '$1 == test_name && $2 == test_path { print $3; found = 1; exit } END { exit found ? 0 : 1 }' \
        "$TEST_BODIES"
      ;;
    *)
      return 1
      ;;
  esac
}

endpoint_path_matches_body() {
  local endpoint_path="$1"
  local body="$2"
  if [[ "$body" == *"$endpoint_path"* ]]; then
    return 0
  fi
  if [[ "$endpoint_path" == *"{"*"}"* ]]; then
    local prefix suffix
    prefix="${endpoint_path%%\{*}"
    suffix="${endpoint_path#*\}}"
    if [ -n "$prefix" ] && [[ "$body" == *"$prefix"* ]]; then
      if [ -z "$suffix" ] || [[ "$body" == *"$suffix"* ]]; then
        return 0
      fi
    fi
  fi
  return 1
}

request_builder_test_matches_path() {
  local test_ref="$1"
  local endpoint_path="$2"
  local body
  if ! body="$(test_body_for_ref "$test_ref")"; then
    return 1
  fi
  endpoint_path_matches_body "$endpoint_path" "$body"
}

validate_safe_probe_endpoints() {
  safe_probe_identities >"$SAFE_PROBES"

  local venue method path use_case data_kind
  while IFS=$'\t' read -r venue method path use_case data_kind; do
    if ! grep -Fxq -- "$venue"$'\t'"$method"$'\t'"$path"$'\t'"$use_case"$'\t'"$data_kind" "$ACTUAL"; then
      printf 'exchange evidence gate failed: safe order_permission probe endpoint missing from EndpointSpec: %s %s %s %s %s\n' \
        "$venue" "$method" "$path" "$use_case" "$data_kind" >&2
      exit 1
    fi
  done <"$SAFE_PROBES"
}

run_self_test() {
	  local tmp fixture duplicate_fixture duplicate_same_file_fixture invalid_name
	  tmp="$(mktemp -d "${TMPDIR:-/tmp}/crossline-exchange-evidence-self-test.XXXXXX")"
	  tmp="$(perl -MCwd=abs_path -e 'print abs_path(shift)' "$tmp")"
	  fixture="$tmp/tests.rs"
	  duplicate_fixture="$tmp/duplicate_tests.rs"
	  duplicate_same_file_fixture="$tmp/duplicate_same_file_tests.rs"
  printf '{"ok":true}\n' >"$tmp/accepted.json"
  printf '{"wrong":true}\n' >"$tmp/wrong.json"
  cat >"$fixture" <<'EOF'
#[test]
fn accepted_unique_test() {}

#[test]
fn accepted_fixture_test() {
    let _body = include_str!("accepted.json");
}

#[test]
fn accepted_fixture_string_marker_test() {
    let _body = include_str!("accepted.json");
    let _note = "return Ok(()) load_or_skip /tmp/";
}

#[test]
fn accepted_request_path_test() {
    let path = "/fapi/v1/order/test";
    assert!(path.ends_with("/order/test"));
}

#[test]
fn accepted_request_path_string_marker_test() {
    let path = "/fapi/v1/order/test";
    let _note = "return Ok(()) load_or_skip /tmp/";
    assert!(path.ends_with("/order/test"));
}

#[test]
fn accepted_template_request_path_test() {
    let path = "/api/v4/futures/usdt/orders/t-cid-1";
    assert!(path.starts_with("/api/v4/futures/usdt/orders/"));
}

mod enabled_parent {
    #[test]
    fn accepted_nested_request_path_test() {
        let path = "/fapi/v1/order/test";
        assert!(path.ends_with("/order/test"));
    }
}

#[test]
fn wrong_request_path_test() {
    let path = "/fapi/v1/order";
    assert!(path.ends_with("/order"));
}

#[test]
fn comment_only_request_path_test() {
    // let path = "/fapi/v1/order/test";
    let path = "/different";
    assert!(path.starts_with("/different"));
}

const MODULE_FIXTURE: &str = include_str!("accepted.json");

#[test]
fn module_const_fixture_test() {
    let _body = MODULE_FIXTURE;
}

#[test]
fn comment_only_fixture_test() {
    // let _body = include_str!("accepted.json");
}

#[test]
fn comment_brace_leaks_fixture_test() {
    // {
}

#[test]
fn sibling_fixture_holder_test() {
    let _body = include_str!("accepted.json");
}

#[test]
fn comment_brace_leaks_request_path_test() {
    // {
}

#[test]
fn sibling_request_path_holder_test() {
    let path = "/fapi/v1/order/test";
    assert!(path.ends_with("/order/test"));
}

#[test]
fn wrong_fixture_test() {
    let _body = include_str!("wrong.json");
}

#[test]
fn fixture_return_unit_fails() {
    let _body = include_str!("accepted.json");
    return;
}

#[test]
fn fixture_return_ok_fails() -> Result<(), ()> {
    let _body = include_str!("accepted.json");
    return Ok(());
}

#[test]
fn fixture_load_or_skip_fails() {
    let _body = include_str!("accepted.json");
    load_or_skip();
}

#[test]
fn fixture_temp_dir_fails() {
    let _body = include_str!("accepted.json");
    let _path = std::env::temp_dir();
}

#[test]
fn request_path_return_unit_fails() {
    let path = "/fapi/v1/order/test";
    return;
}

#[test]
fn request_path_return_ok_fails() -> Result<(), ()> {
    let path = "/fapi/v1/order/test";
    return Ok(());
}

#[test]
fn request_path_load_or_skip_fails() {
    let path = "/fapi/v1/order/test";
    load_or_skip();
}

#[test]
fn request_path_temp_dir_fails() {
    let path = "/fapi/v1/order/test";
    let _path = std::env::temp_dir();
}

#[tokio::test]
async fn accepted_tokio_test() {}

#[test]
fn accepted_duplicate_test() {}

#[ignore]
#[test]
fn ignored_test() {}

#[should_panic]
#[test]
fn panic_expected_test() {}

#[cfg_attr(any(), should_panic)]
#[test]
fn cfg_panic_expected_test() {}

#[cfg(any())]
// cfg reason must not hide the disabling attribute
#[test]
fn cfg_disabled_with_comment_gap_test() {}

#[ignore]
// ignore reason must not hide the disabling attribute
#[test]
fn ignored_with_comment_gap_test() {}

#[cfg_attr(all(), cfg(any()))]
// cfg_attr reason must not hide the disabling attribute
#[test]
fn cfg_attr_cfg_comment_gap_test() {}

#[cfg(any())]
#[test]
fn cfg_disabled_test() {}

#[cfg_attr(
    all(),
    cfg(any())
)]
#[test]
fn cfg_attr_cfg_disabled_test() {}

#[cfg_attr(
    any(),
    ignore
)]
#[test]
fn multiline_cfg_ignore_test() {}

#[cfg_attr(
    any(),
    should_panic
)]
#[test]
fn multiline_cfg_panic_expected_test() {}

#[cfg(any())]
mod disabled_parent {
    #[test]
    fn parent_cfg_disabled_request_path_test() {
        let path = "/fapi/v1/order/test";
        assert!(path.ends_with("/order/test"));
    }
}

#[cfg_attr(all(), cfg(any()))]
mod cfg_attr_disabled_parent {
    #[test]
    fn parent_cfg_attr_disabled_request_path_test() {
        let path = "/fapi/v1/order/test";
        assert!(path.ends_with("/order/test"));
    }
}

#[cfg(any())]
// cfg reason must not hide the disabling parent module
mod disabled_parent_with_comment_gap {
    #[test]
    fn parent_cfg_comment_gap_request_path_test() {
        let path = "/fapi/v1/order/test";
        assert!(path.ends_with("/order/test"));
    }
}

#[cfg_attr(any(), ignore)]
mod ignored_parent {
    #[test]
    fn parent_cfg_attr_ignore_request_path_test() {
        let path = "/fapi/v1/order/test";
        assert!(path.ends_with("/order/test"));
    }
}

/*
#[test]
fn block_commented_test() {}
*/

/*
outer
/* nested */
#[test]
fn nested_block_commented_test() {}
*/
EOF

	  cat >"$duplicate_fixture" <<'EOF'
#[test]
fn accepted_duplicate_test() {}
EOF
	  cat >"$duplicate_same_file_fixture" <<'EOF'
mod alpha {
    #[test]
    fn duplicate_same_file_test() {
        let path = "/wrong";
        assert!(path.ends_with("/wrong"));
    }
}

mod beta {
    #[test]
    fn duplicate_same_file_test() {
        let path = "/fapi/v1/order/test";
        assert!(path.ends_with("/order/test"));
    }
}
EOF

	  printf '%s\n%s\n' "$fixture" "$duplicate_fixture" | build_test_registry_from_paths
	  has_runnable_test accepted_unique_test
	  has_runnable_test accepted_tokio_test
	  has_runnable_test "$fixture::accepted_nested_request_path_test"
	  has_runnable_test "$fixture::accepted_duplicate_test"
	  parser_test_includes_fixture "$fixture::accepted_fixture_test" "$tmp/accepted.json"
	  parser_test_includes_fixture "$fixture::accepted_fixture_string_marker_test" "$tmp/accepted.json"
	  test_body_has_no_skip_marker "$fixture::accepted_fixture_string_marker_test"
	  request_builder_test_matches_path "$fixture::accepted_request_path_test" "/fapi/v1/order/test"
	  request_builder_test_matches_path "$fixture::accepted_request_path_string_marker_test" "/fapi/v1/order/test"
	  test_body_has_no_skip_marker "$fixture::accepted_request_path_string_marker_test"
	  request_builder_test_matches_path "$fixture::accepted_template_request_path_test" "/api/v4/futures/usdt/orders/{order_id}"
	  request_builder_test_matches_path "$fixture::accepted_nested_request_path_test" "/fapi/v1/order/test"
	  if request_builder_test_matches_path "$fixture::wrong_request_path_test" "/fapi/v1/order/test" \
	    || request_builder_test_matches_path "$fixture::comment_only_request_path_test" "/fapi/v1/order/test" \
	    || request_builder_test_matches_path "$fixture::comment_brace_leaks_request_path_test" "/fapi/v1/order/test" \
	    || request_builder_test_matches_path "$fixture::parent_cfg_disabled_request_path_test" "/fapi/v1/order/test" \
	    || request_builder_test_matches_path "$fixture::parent_cfg_attr_disabled_request_path_test" "/fapi/v1/order/test" \
	    || request_builder_test_matches_path "$fixture::parent_cfg_comment_gap_request_path_test" "/fapi/v1/order/test" \
	    || request_builder_test_matches_path "$fixture::parent_cfg_attr_ignore_request_path_test" "/fapi/v1/order/test"; then
	    rm -rf "$tmp"
	    printf 'exchange evidence gate self-test failed: request-builder path binding accepted wrong or comment-only path\n' >&2
	    exit 1
  fi
  if parser_test_includes_fixture "$fixture::wrong_fixture_test" "$tmp/accepted.json" \
    || parser_test_includes_fixture "$fixture::comment_only_fixture_test" "$tmp/accepted.json" \
    || parser_test_includes_fixture "$fixture::comment_brace_leaks_fixture_test" "$tmp/accepted.json" \
    || parser_test_includes_fixture "$fixture::module_const_fixture_test" "$tmp/accepted.json"; then
    rm -rf "$tmp"
    printf 'exchange evidence gate self-test failed: parser fixture binding accepted wrong or indirect include\n' >&2
    exit 1
  fi
  if test_body_has_no_skip_marker "$fixture::fixture_return_unit_fails" \
    || test_body_has_no_skip_marker "$fixture::fixture_return_ok_fails" \
    || test_body_has_no_skip_marker "$fixture::fixture_load_or_skip_fails" \
    || test_body_has_no_skip_marker "$fixture::fixture_temp_dir_fails" \
    || test_body_has_no_skip_marker "$fixture::request_path_return_unit_fails" \
    || test_body_has_no_skip_marker "$fixture::request_path_return_ok_fails" \
    || test_body_has_no_skip_marker "$fixture::request_path_load_or_skip_fails" \
    || test_body_has_no_skip_marker "$fixture::request_path_temp_dir_fails"; then
    rm -rf "$tmp"
    printf 'exchange evidence gate self-test failed: endpoint parser/request-builder skip marker accepted\n' >&2
    exit 1
  fi
  validate_doc_url_pair Binance "https://developers.binance.com/docs/derivatives/usds-margined-futures/trade/rest-api/New-Order"
  validate_doc_url_pair Okx "https://www.okx.com/docs-v5/en/#order-book-trading-trade-post-place-order"
  validate_doc_url_pair Bybit "https://bybit-exchange.github.io/docs/v5/order/create-order"
  validate_doc_url_pair Bitget "https://www.bitget.com/api-doc/uta/trade/Place-Order"
  validate_doc_url_pair Gate "https://www.gate.com/docs/developers/apiv4/en/futures/#create-a-futures-order"
  validate_doc_url_pair Kucoin "https://www.kucoin.com/docs-new/rest/futures-trading/orders/add-order-test"
  validate_doc_url_pair Hyperliquid "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/exchange-endpoint#place-an-order"
  checked_at_is_not_future "$TODAY"
  checked_at_is_not_future "not_recorded"
  if checked_at_is_not_future "$(date -v+1d +%F 2>/dev/null || date -d tomorrow +%F)"; then
    rm -rf "$tmp"
    printf 'exchange evidence gate self-test failed: future checked_at accepted\n' >&2
    exit 1
  fi
  endpoint_fixture_matches_venue Binance "crates/exchange/fixtures/binance/example.json"
  endpoint_fixture_matches_venue Okx "crates/exchange/fixtures/okx/example.json"
  if endpoint_fixture_matches_venue Binance "crates/exchange/fixtures/okx/example.json" \
    || endpoint_fixture_matches_venue Binance "fixtures/binance/example.json" \
    || endpoint_fixture_matches_venue Unknown "crates/exchange/fixtures/unknown/example.json"; then
    rm -rf "$tmp"
    printf 'exchange evidence gate self-test failed: fixture venue path binding accepted wrong directory\n' >&2
    exit 1
  fi
  validate_unrecorded_probe_provenance Binance Post /fapi/v1/order/test TradeWrite OrderAck \
    2026-07-07 binance-usdm-futures-test-new-order-2026-07-07 not_recorded not_recorded not_recorded \
    "$fixture::accepted_request_path_test" signed
  if validate_unrecorded_probe_provenance Binance Post /fapi/v1/order/test TradeWrite OrderAck \
      not_recorded binance-usdm-futures-test-new-order-2026-07-07 not_recorded not_recorded not_recorded \
      "$fixture::accepted_request_path_test" signed 2>/dev/null \
    || validate_unrecorded_probe_provenance Binance Post /fapi/v1/order/test TradeWrite OrderAck \
      2026-07-07 binance-usdm-futures-test-new-order-2026-07-07 sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa not_recorded not_recorded \
      "$fixture::accepted_request_path_test" signed 2>/dev/null \
    || validate_unrecorded_probe_provenance Binance Post /fapi/v1/order/test TradeWrite OrderAck \
      2026-07-07 binance-usdm-futures-test-new-order-2026-07-07 not_recorded not_recorded accepted_unique_test \
      "$fixture::accepted_request_path_test" signed 2>/dev/null \
    || validate_unrecorded_probe_provenance Binance Post /fapi/v1/order/test TradeWrite OrderAck \
      2026-07-07 binance-usdm-futures-test-new-order-2026-07-07 not_recorded not_recorded not_recorded \
      not_recorded signed 2>/dev/null \
    || validate_unrecorded_probe_provenance Binance Get /fapi/v1/order PrivateRead OrderStatus \
      2026-07-07 binance-usdm-futures-query-order-2026-07-07 not_recorded not_recorded not_recorded \
      "$fixture::accepted_request_path_test" signed 2>/dev/null \
    || validate_unrecorded_probe_provenance Binance Get /fapi/v1/time Calibration ServerTime \
      2026-07-07 binance-usdm-futures-time-2026-07-07 not_recorded not_recorded not_recorded \
      "$fixture::accepted_request_path_test" public 2>/dev/null; then
    rm -rf "$tmp"
    printf 'exchange evidence gate self-test failed: bad unrecorded probe provenance accepted\n' >&2
    exit 1
  fi
  printf 'Binance\tPost\t/fapi/v1/order/test\tTradeWrite\tOrderAck\n' >"$tmp/unique.tsv"
  printf 'Binance\tPost\t/fapi/v1/order/test\tTradeWrite\tOrderAck\nBinance\tPost\t/fapi/v1/order/test\tTradeWrite\tOrderAck\n' >"$tmp/duplicate.tsv"
  require_unique_endpoint_identities "self-test" "$tmp/unique.tsv"
  if require_unique_endpoint_identities "self-test" "$tmp/duplicate.tsv" 2>/dev/null; then
    rm -rf "$tmp"
    printf 'exchange evidence gate self-test failed: duplicate endpoint identity accepted\n' >&2
    exit 1
  fi
  printf '%s\n' \
    $'Hyperliquid\tPost\t/info\tPrivateRead\tAccountBalance' \
    $'Hyperliquid\tPost\t/info\tPrivateRead\tAccountBalance' \
    $'Hyperliquid\tPost\t/info\tPrivateRead\tOrderStatus' \
    $'Hyperliquid\tPost\t/info\tPrivateRead\tOrderStatus' \
    $'Hyperliquid\tPost\t/info\tPrivateRead\tOrderStatus' >"$tmp/hyperliquid_info_duplicates.tsv"
  require_unique_endpoint_identities "self-test" "$tmp/hyperliquid_info_duplicates.tsv"
  printf '%s\n' \
    $'Binance\tPost\t/fapi/v1/order/test\tTradeWrite\tOrderAck' \
    $'Binance\tPost\t/fapi/v1/order/test\tTradeWrite\tOrderAck' >"$tmp/other_venue_duplicate.tsv"
  if require_unique_endpoint_identities "self-test" "$tmp/other_venue_duplicate.tsv" 2>/dev/null; then
    rm -rf "$tmp"
    printf 'exchange evidence gate self-test failed: duplicate in another venue accepted\n' >&2
    exit 1
  fi
  printf '%s\n' \
    $'Hyperliquid\tPost\t/exchange\tTradeWrite\tOrderAck' \
    $'Hyperliquid\tPost\t/exchange\tTradeWrite\tOrderAck' >"$tmp/hyperliquid_non_info_duplicate.tsv"
  if require_unique_endpoint_identities "self-test" "$tmp/hyperliquid_non_info_duplicate.tsv" 2>/dev/null; then
    rm -rf "$tmp"
    printf 'exchange evidence gate self-test failed: Hyperliquid non-/info duplicate accepted\n' >&2
    exit 1
  fi
  cat >"$tmp/spec.rs" <<'EOF'
const CHECKED_AT: &str = "2026-07-07";
const DOC_VERSION: &str = "binance-usdm-futures-test-new-order-2026-07-07";
const REQUEST_TEST: &str =
    "crates/exchange/src/adapters/binance_private_rest.rs::accepted_unique_test";
const SIGNED_AUTH_KIND: &str = "signed";
const HYPERLIQUID_OPERATION_CHECKED_AT: &str = "2026-07-12";
const HYPERLIQUID_OPERATION_DOC_VERSION: &str = "hyperliquid-info-open-orders-2026-07-12";
const HYPERLIQUID_OPERATION_SCHEMA_HASH: &str =
    "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const HYPERLIQUID_OPERATION_FIXTURE_ID: &str =
    "crates/exchange/fixtures/hyperliquid/info_open_orders.json";
const HYPERLIQUID_OPERATION_PARSER_TEST: &str =
    "hyperliquid_open_orders_parses_official_fixture";
const HYPERLIQUID_OPERATION_REQUEST_TEST: &str =
    "hyperliquid_info_open_orders_requests_info";
const HYPERLIQUID_OPERATION_AUTH_KIND: &str = "user_address";

const RECORDED_ENDPOINT_EVIDENCE: &[EndpointEvidenceEntry] = &[
    EndpointEvidenceEntry {
        venue: VenueId::Binance,
        method: HttpMethod::Post,
        path: "/fapi/v1/order/test",
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
        meta: EndpointEvidenceMeta {
            checked_at: CHECKED_AT,
            doc_version: DOC_VERSION,
            schema_hash: UNRECORDED_EVIDENCE_MARKER,
            fixture_id: UNRECORDED_EVIDENCE_MARKER,
            parser_test: UNRECORDED_EVIDENCE_MARKER,
            request_builder_test: REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
];

const HYPERLIQUID_OPERATION_EVIDENCE: &[HyperliquidOperationEvidence] = &[
    HyperliquidOperationEvidence {
        transport: HyperliquidOperationTransport::Info,
        operation: "openOrders",
        dex_scope: HyperliquidDexScope::OptionalPerpDex,
        doc_url: "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/info-endpoint",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
        meta: EndpointEvidenceMeta {
            checked_at: HYPERLIQUID_OPERATION_CHECKED_AT,
            doc_version: HYPERLIQUID_OPERATION_DOC_VERSION,
            schema_hash: HYPERLIQUID_OPERATION_SCHEMA_HASH,
            fixture_id: HYPERLIQUID_OPERATION_FIXTURE_ID,
            parser_test: HYPERLIQUID_OPERATION_PARSER_TEST,
            request_builder_test: HYPERLIQUID_OPERATION_REQUEST_TEST,
            auth_kind: HYPERLIQUID_OPERATION_AUTH_KIND,
        },
    },
    HyperliquidOperationEvidence {
        transport: HyperliquidOperationTransport::Info,
        operation: "metaAndAssetCtxs",
        dex_scope: HyperliquidDexScope::OptionalPerpDex,
        doc_url: "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/info-endpoint",
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::PerpTicker,
        meta: EndpointEvidenceMeta {
            checked_at: HYPERLIQUID_OPERATION_CHECKED_AT,
            doc_version: HYPERLIQUID_OPERATION_DOC_VERSION,
            schema_hash: HYPERLIQUID_OPERATION_SCHEMA_HASH,
            fixture_id: HYPERLIQUID_OPERATION_FIXTURE_ID,
            parser_test: HYPERLIQUID_OPERATION_PARSER_TEST,
            request_builder_test: HYPERLIQUID_OPERATION_REQUEST_TEST,
            auth_kind: HYPERLIQUID_OPERATION_AUTH_KIND,
        },
    },
    HyperliquidOperationEvidence {
        transport: HyperliquidOperationTransport::WebSocket,
        operation: "allDexsClearinghouseState",
        dex_scope: HyperliquidDexScope::AllPerpDexes,
        doc_url: "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/websocket/subscriptions",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountBalance,
        meta: EndpointEvidenceMeta {
            checked_at: HYPERLIQUID_OPERATION_CHECKED_AT,
            doc_version: HYPERLIQUID_OPERATION_DOC_VERSION,
            schema_hash: HYPERLIQUID_OPERATION_SCHEMA_HASH,
            fixture_id: HYPERLIQUID_OPERATION_FIXTURE_ID,
            parser_test: HYPERLIQUID_OPERATION_PARSER_TEST,
            request_builder_test: HYPERLIQUID_OPERATION_REQUEST_TEST,
            auth_kind: HYPERLIQUID_OPERATION_AUTH_KIND,
        },
    },
];
EOF
  extract_recorded_endpoint_evidence "$tmp/spec.rs" | sort >"$tmp/registered.tsv"
  printf '%s\n' \
    $'Binance\tPost\t/fapi/v1/order/test\tTradeWrite\tOrderAck\t2026-07-07\tbinance-usdm-futures-test-new-order-2026-07-07\tnot_recorded\tnot_recorded\tnot_recorded\tcrates/exchange/src/adapters/binance_private_rest.rs::accepted_unique_test\tsigned' \
    $'Hyperliquid\tPost\t/info\tPrivateRead\tOrderStatus\t2026-07-12\thyperliquid-info-open-orders-2026-07-12\tsha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\tcrates/exchange/fixtures/hyperliquid/info_open_orders.json\thyperliquid_open_orders_parses_official_fixture\thyperliquid_info_open_orders_requests_info\tuser_address' >"$tmp/registered_allowlist.tsv"
  require_endpoint_metadata_match "$tmp/registered.tsv" "$tmp/registered_allowlist.tsv"
  printf 'Binance\tPost\t/fapi/v1/order/test\tTradeWrite\tOrderAck\t2026-07-07\tbinance-usdm-futures-test-new-order-2026-07-07\tnot_recorded\tnot_recorded\tnot_recorded\twrong_test\tsigned\n' >"$tmp/registered_drift.tsv"
  if require_endpoint_metadata_match "$tmp/registered.tsv" "$tmp/registered_drift.tsv" 2>/dev/null; then
    rm -rf "$tmp"
    printf 'exchange evidence gate self-test failed: metadata drift accepted\n' >&2
    exit 1
  fi
  perl -pe 's/hyperliquid_info_open_orders_requests_info/wrong_hyperliquid_operation_request_test/' "$tmp/spec.rs" >"$tmp/operation_drift_spec.rs"
  extract_recorded_endpoint_evidence "$tmp/operation_drift_spec.rs" | sort >"$tmp/operation_drift_registered.tsv"
  if require_endpoint_metadata_match "$tmp/operation_drift_registered.tsv" "$tmp/registered_allowlist.tsv" 2>/dev/null; then
    rm -rf "$tmp"
    printf 'exchange evidence gate self-test failed: Hyperliquid operation metadata drift accepted\n' >&2
    exit 1
  fi
  perl -pe 's/request_builder_test: REQUEST_TEST,/request_builder_test: MISSING_REQUEST_TEST,/' "$tmp/spec.rs" >"$tmp/unresolved_spec.rs"
  extract_recorded_endpoint_evidence "$tmp/unresolved_spec.rs" | sort >"$tmp/unresolved_registered.tsv"
  if require_endpoint_metadata_match "$tmp/unresolved_registered.tsv" "$tmp/registered_allowlist.tsv" 2>/dev/null; then
    rm -rf "$tmp"
    printf 'exchange evidence gate self-test failed: unresolved metadata constant accepted\n' >&2
    exit 1
  fi
  if validate_doc_url_pair Binance "https://example.com/binance" 2>/dev/null \
    || validate_doc_url_pair Okx "http://www.okx.com/docs-v5/en/" 2>/dev/null \
    || validate_doc_url_pair Bybit "not_recorded" 2>/dev/null; then
    rm -rf "$tmp"
    printf 'exchange evidence gate self-test failed: bad doc_url accepted\n' >&2
    exit 1
  fi
  if has_runnable_test accepted_duplicate_test 2>/dev/null; then
    rm -rf "$tmp"
    printf 'exchange evidence gate self-test failed: ambiguous test reference accepted\n' >&2
    exit 1
  fi
	  for invalid_name in ignored_test panic_expected_test cfg_panic_expected_test cfg_disabled_with_comment_gap_test ignored_with_comment_gap_test cfg_attr_cfg_comment_gap_test cfg_disabled_test cfg_attr_cfg_disabled_test multiline_cfg_ignore_test multiline_cfg_panic_expected_test parent_cfg_disabled_request_path_test parent_cfg_attr_disabled_request_path_test parent_cfg_comment_gap_request_path_test parent_cfg_attr_ignore_request_path_test block_commented_test nested_block_commented_test; do
	    if awk -F '\t' -v test_name="$invalid_name" '$1 == test_name { found = 1 } END { exit found ? 0 : 1 }' "$TESTS"; then
	      rm -rf "$tmp"
	      printf 'exchange evidence gate self-test failed: invalid test attribute accepted\n' >&2
	      exit 1
    fi
  done
	  if has_runnable_test "$duplicate_fixture::accepted_unique_test" 2>/dev/null; then
	    rm -rf "$tmp"
	    printf 'exchange evidence gate self-test failed: file-bound test path mismatch accepted\n' >&2
	    exit 1
	  fi
	  if printf '%s\n' "$duplicate_same_file_fixture" | build_test_registry_from_paths 2>/dev/null; then
	    rm -rf "$tmp"
	    printf 'exchange evidence gate self-test failed: duplicate same-file test name accepted\n' >&2
	    exit 1
	  fi
	  rm -rf "$tmp"
	  printf 'OK exchange evidence debt self-test\n'
	}

if [ "${1:-}" = "--self-test" ]; then
  run_self_test
  exit 0
fi

build_test_registry
validate_endpoint_doc_urls
require_unique_endpoint_identities "EndpointSpec" "$ACTUAL"
require_unique_endpoint_identities "allowlist" "$LISTED"
validate_registered_endpoint_metadata
require_unique_endpoint_identities "runtime registry" "$REGISTERED"
validate_recorded_fixtures <"$ALLOWLIST"
validate_safe_probe_endpoints
if ! diff -u "$ACTUAL" "$LISTED" >"$BAD"; then
  printf 'exchange evidence gate failed: EndpointSpec rows must be listed in scripts/exchange_evidence_debt_allowlist.tsv\n' >&2
  cat "$BAD" >&2
  exit 1
fi

printf 'OK exchange evidence debt gate (%s endpoint rows)\n' "$(wc -l <"$ACTUAL" | tr -d ' ')"
