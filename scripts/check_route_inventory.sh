#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
INVENTORY="$ROOT/docs/API_ROUTE_INVENTORY.tsv"
ACTUAL="${TMPDIR:-/tmp}/crossline-routes-actual.$$"
ACTUAL_METHODS="${TMPDIR:-/tmp}/crossline-routes-actual-methods.$$"
LISTED="${TMPDIR:-/tmp}/crossline-routes-listed.$$"
LISTED_METHODS="${TMPDIR:-/tmp}/crossline-routes-listed-methods.$$"
FRONTEND="${TMPDIR:-/tmp}/crossline-routes-frontend.$$"
FRONTEND_CLIENTS="${TMPDIR:-/tmp}/crossline-routes-frontend-clients.$$"
LISTED_CLIENTS="${TMPDIR:-/tmp}/crossline-routes-listed-clients.$$"
INVENTORY_FRONTEND_NONE="${TMPDIR:-/tmp}/crossline-routes-inventory-frontend-none.$$"
INVENTORY_NORMAL="${TMPDIR:-/tmp}/crossline-routes-inventory-normal.$$"
BAD="${TMPDIR:-/tmp}/crossline-routes-bad.$$"
trap 'rm -f "$ACTUAL" "$ACTUAL_METHODS" "$LISTED" "$LISTED_METHODS" "$FRONTEND" "$FRONTEND_CLIENTS" "$LISTED_CLIENTS" "$INVENTORY_FRONTEND_NONE" "$INVENTORY_NORMAL" "$BAD"' EXIT

if [ ! -s "$INVENTORY" ]; then
  printf 'route inventory gate failed: missing %s\n' "${INVENTORY#$ROOT/}" >&2
  exit 1
fi

extract_routes() {
  extract_route_methods | cut -f1 | sort -u
}

frontend_rest_files() {
  find "$ROOT/frontend/src/api" -maxdepth 2 -type f -name '*.rs' \
    | sort \
    | rg '/rest(\.rs|/)'
}

extract_route_methods() {
  find "$ROOT/crates/api/src/routers" -maxdepth 2 -type f -name '*.rs' ! -name '*_tests.rs' \
    | sort \
    | while IFS= read -r file; do
      perl -ne '
        if (!$capture && /\.route\(/) {
          $capture = 1;
          $block = "";
          $depth = 0;
        }
        if ($capture) {
          $block .= $_;
          $depth += tr/(//;
          $depth -= tr/)//;
          if ($depth <= 0) {
            if ($block =~ /\.route\(\s*"([^"]+)"/s) {
              $path = $1;
              next unless $path =~ m{^/(api|health|metrics|ws)(/|$)};
              @methods = ();
              push @methods, "GET" if $block =~ /\bget\s*\(/;
              push @methods, "POST" if $block =~ /\bpost\s*\(/;
              push @methods, "PATCH" if $block =~ /\bpatch\s*\(/;
              push @methods, "DELETE" if $block =~ /\bdelete\s*\(/;
              push @methods, "PUT" if $block =~ /\bput\s*\(/;
              for my $method (@methods) {
                print "$path\t$method\n";
              }
            }
            $capture = 0;
          }
        }
      ' "$file"
    done \
    | sort -u
}

extract_route_methods >"$ACTUAL_METHODS"
cut -f1 "$ACTUAL_METHODS" | sort -u >"$ACTUAL"
awk -F '\t' 'NR > 1 { print $1 }' "$INVENTORY" | sort -u >"$LISTED"
awk -F '\t' '
  NR > 1 {
    split($2, methods, ",")
    for (idx in methods) {
      print $1 "\t" methods[idx]
    }
  }
' "$INVENTORY" | sort -u >"$LISTED_METHODS"

if ! diff -u "$ACTUAL" "$LISTED" >"$BAD"; then
  printf 'route inventory gate failed: docs/API_ROUTE_INVENTORY.tsv must match registered axum routes\n' >&2
  cat "$BAD" >&2
  exit 1
fi

if ! diff -u "$ACTUAL_METHODS" "$LISTED_METHODS" >"$BAD"; then
  printf 'route inventory gate failed: route methods in docs/API_ROUTE_INVENTORY.tsv drifted from axum routes\n' >&2
  cat "$BAD" >&2
  exit 1
fi

extract_frontend_routes() {
  frontend_rest_files \
    | while IFS= read -r file; do
      perl -ne 'while (m@"(/(?:api|health)[^"]*)"@g) { print "$1\n" }' "$file"
    done \
    | perl -ne '
      chomp;
      s/\?.*$//;
      s/\{qs\}$//;
      s/\{[^}]*\}/:param/g;
      s/:[^\/\n]+/:param/g;
      print "$_\n" if m{^/(api|health)(/|$)};
    ' \
    | sort -u
}

extract_frontend_client_methods() {
  frontend_rest_files \
    | while IFS= read -r file; do
      perl -ne 'print "ApiClient::$1\n" if /\bpub\s+async\s+fn\s+([A-Za-z0-9_]+)/' "$file"
    done \
    | sort -u
}

awk -F '\t' 'NR > 1 { print $1 }' "$INVENTORY" \
  | perl -pe 's/:[^\/\n]+/:param/g' \
  | sort -u >"$INVENTORY_NORMAL"
awk -F '\t' '
  NR > 1 {
    paths[$1] = 1
    if ($11 != "none") {
      clients[$1] = 1
    }
  }
  END {
    for (path in paths) {
      if (!clients[path]) {
        print path
      }
    }
  }
' "$INVENTORY" \
  | perl -pe 's/:[^\/\n]+/:param/g' \
  | sort -u >"$INVENTORY_FRONTEND_NONE"
awk -F '\t' 'NR > 1 && $11 != "none" { print $11 }' "$INVENTORY" \
  | tr '+' '\n' \
  | sort -u >"$LISTED_CLIENTS"
extract_frontend_routes >"$FRONTEND"
extract_frontend_client_methods >"$FRONTEND_CLIENTS"

if comm -23 "$FRONTEND" "$INVENTORY_NORMAL" >"$BAD"; then
  if [ -s "$BAD" ]; then
    printf 'route inventory gate failed: frontend REST path lacks inventory row\n' >&2
    cat "$BAD" >&2
    exit 1
  fi
fi

if comm -12 "$FRONTEND" "$INVENTORY_FRONTEND_NONE" >"$BAD"; then
  if [ -s "$BAD" ]; then
    printf 'route inventory gate failed: frontend REST path is marked frontend_client=none\n' >&2
    cat "$BAD" >&2
    exit 1
  fi
fi

if comm -23 "$LISTED_CLIENTS" "$FRONTEND_CLIENTS" >"$BAD"; then
  if [ -s "$BAD" ]; then
    printf 'route inventory gate failed: frontend_client method is not defined on ApiClient\n' >&2
    cat "$BAD" >&2
    exit 1
  fi
fi

if comm -23 "$FRONTEND_CLIENTS" "$LISTED_CLIENTS" >"$BAD"; then
  if [ -s "$BAD" ]; then
    printf 'route inventory gate failed: frontend ApiClient method lacks inventory frontend_client coverage\n' >&2
    cat "$BAD" >&2
    exit 1
  fi
fi

awk -F '\t' '
  NR == 1 {
    expected = "path\tmethods\trouter\tclass\tdefault_exposure\trisk\tauth_policy\taudit_policy\tfeature_flag\tshared_dto\tfrontend_client\towner\tnotes"
    if ($0 != expected) {
      printf "route inventory gate failed: bad header\n" > "/dev/stderr"
      exit 1
    }
    next
  }
  NF != 13 {
    printf "route inventory gate failed: %s has %d columns, expected 13\n", $1, NF > "/dev/stderr"
    exit 1
  }
  $1 !~ /^\/(api|health|metrics|ws)(\/|$)/ {
    printf "route inventory gate failed: bad path %s\n", $1 > "/dev/stderr"
    exit 1
  }
  $2 !~ /^(GET|POST|PATCH|DELETE|PUT)(,(GET|POST|PATCH|DELETE|PUT))*$/ {
    printf "route inventory gate failed: bad methods for %s\n", $1 > "/dev/stderr"
    exit 1
  }
  $4 !~ /^(main_p0|product_support|diagnostic|legacy|non_main|liveness|readiness)$/ {
    printf "route inventory gate failed: bad class for %s\n", $1 > "/dev/stderr"
    exit 1
  }
  $5 !~ /^(always|default_on|default_off)$/ {
    printf "route inventory gate failed: bad default_exposure for %s\n", $1 > "/dev/stderr"
    exit 1
  }
  $6 !~ /^(low|medium|high)$/ {
    printf "route inventory gate failed: bad risk for %s\n", $1 > "/dev/stderr"
    exit 1
  }
  $7 !~ /^(public_liveness|bearer|bearer_ws|scrape_bearer)$/ {
    printf "route inventory gate failed: bad auth_policy for %s\n", $1 > "/dev/stderr"
    exit 1
  }
  $8 !~ /^(none|read|action_run|required|secret_mutation|external_payload|metrics)$/ {
    printf "route inventory gate failed: bad audit_policy for %s\n", $1 > "/dev/stderr"
    exit 1
  }
  $9 !~ /^(core|api_surface\.(chat|llm_diagnostics|options|watchlist_alerts|spot_v1|strategy_v1))$/ {
    printf "route inventory gate failed: bad feature_flag for %s\n", $1 > "/dev/stderr"
    exit 1
  }
  $10 !~ /^(shared|frontend_local|router_local|text|websocket|prometheus|none)$/ {
    printf "route inventory gate failed: bad shared_dto for %s\n", $1 > "/dev/stderr"
    exit 1
  }
  $11 !~ /^(none|ApiClient::[A-Za-z0-9_]+(\+ApiClient::[A-Za-z0-9_]+)*)$/ {
    printf "route inventory gate failed: bad frontend_client for %s\n", $1 > "/dev/stderr"
    exit 1
  }
  $6 == "high" && $8 ~ /^(none|read)$/ {
    printf "route inventory gate failed: high-risk route needs mutation audit policy for %s\n", $1 > "/dev/stderr"
    exit 1
  }
  $4 == "liveness" && $7 != "public_liveness" {
    printf "route inventory gate failed: liveness route must be public_liveness for %s\n", $1 > "/dev/stderr"
    exit 1
  }
  $4 != "liveness" && $7 == "public_liveness" {
    printf "route inventory gate failed: only liveness routes may be public_liveness for %s\n", $1 > "/dev/stderr"
    exit 1
  }
  $4 == "diagnostic" && $7 !~ /^(bearer|scrape_bearer)$/ {
    printf "route inventory gate failed: diagnostic route needs bearer auth for %s\n", $1 > "/dev/stderr"
    exit 1
  }
  $5 != "always" && $9 == "core" {
    printf "route inventory gate failed: gated route needs explicit feature flag for %s\n", $1 > "/dev/stderr"
    exit 1
  }
  $9 ~ /^api_surface\./ && $5 != "default_off" {
    printf "route inventory gate failed: optional api surface must be default_off for %s\n", $1 > "/dev/stderr"
    exit 1
  }
  $4 == "product_support" && ($5 != "default_off" || $9 !~ /^api_surface\./) {
    printf "route inventory gate failed: product-support route must be default_off and api_surface-gated for %s\n", $1 > "/dev/stderr"
    exit 1
  }
  $4 == "legacy" && $1 != "/api/v3/arbitrage/opportunities" && ($5 != "default_off" || $9 !~ /^api_surface\./) {
    printf "route inventory gate failed: legacy route must be default_off and api_surface-gated for %s\n", $1 > "/dev/stderr"
    exit 1
  }
  $12 == "" || $13 == "" {
    printf "route inventory gate failed: owner/notes required for %s\n", $1 > "/dev/stderr"
    exit 1
  }
' "$INVENTORY"

if rg -n 'live-readiness|/api/v1/rwa|rwa_basket|US equity reference' "$INVENTORY"; then
  printf 'route inventory gate failed: canceled product semantics must not appear in route inventory\n' >&2
  exit 1
fi

printf 'OK route inventory gate\n'
