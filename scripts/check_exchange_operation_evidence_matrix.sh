#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ALLOWLIST="$ROOT/scripts/exchange_evidence_debt_allowlist.tsv"
MATRIX="$ROOT/scripts/exchange_operation_evidence_matrix.tsv"
WS_TEST="$ROOT/crates/exchange/tests/ws_trading_specs_test.rs"
WS_REGISTRY="$ROOT/crates/exchange/src/ws/trading.rs"
REST_REGISTRY="$ROOT/crates/exchange/src/rest_registry.rs"
API_REGISTRY_TEST="$ROOT/crates/api/src/routers/trading/tests/cases_registry.rs"
DIAG_TEST="$ROOT/crates/exchange/tests/diag_real_responses.rs"
VENUE_SPEC="$ROOT/crates/exchange/src/venue_spec.rs"

fail() {
  printf 'exchange operation evidence matrix gate failed: %s\n' "$*" >&2
  exit 1
}

validate_matrix() {
  awk -F '\t' '
    BEGIN {
      expected_header = "venue\trest_trade_write_order_ack\trest_private_order_status\trest_private_account_balance\trest_private_account_position\tws_live_write_path\tws_private_stream_evidence\tclose_position_boundary\tdiagnostic_fixture_boundary\tdiagnostic_fixture_ids\tdiagnostic_fixture_hashes\tws_operation_fixture_ids\tws_operation_fixture_hashes\tfinality_boundary"
      expected_venues = "\tBinance\tOkx\tBybit\tBitget\tGate\tKucoin\tHyperliquid\tgate_crossex\tKraken\t"
    }
    NR == 1 {
      if ($0 != expected_header) {
        printf "bad matrix header\n" > "/dev/stderr"
        exit 1
      }
      next
    }
    {
      if (NF != 14) {
        printf "bad matrix column count for %s\n", $1 > "/dev/stderr"
        exit 1
      }
      if (index(expected_venues, "\t" $1 "\t") == 0) {
        printf "unexpected venue %s\n", $1 > "/dev/stderr"
        exit 1
      }
      if (index(seen, "\t" $1 "\t") != 0) {
        printf "duplicate venue %s\n", $1 > "/dev/stderr"
        exit 1
      }
      seen = seen "\t" $1 "\t"
      for (col = 2; col <= 5; col++) {
        expected_rest = ($1 == "gate_crossex" && col == 2) ? "not_applicable_ws_only" : "recorded"
        if ($col != expected_rest) {
          printf "REST evidence column %d for %s must be %s\n", col, $1, expected_rest > "/dev/stderr"
          exit 1
        }
      }
      expected_write = ($1 == "Kucoin") ? "display_only_schema_pending" : "recorded_place_cancel"
      if ($6 != expected_write) {
        printf "bad ws_live_write_path for %s\n", $1 > "/dev/stderr"
        exit 1
      }
      if ($7 != "recorded" || $8 != "display_only_without_operation_evidence" || $14 != "ack_not_final") {
        printf "bad boundary/finality columns for %s\n", $1 > "/dev/stderr"
        exit 1
      }
      expected_diag = ($1 == "Binance" || $1 == "Okx" || $1 == "Bybit" || $1 == "Bitget" || $1 == "Gate" || $1 == "Kucoin" || $1 == "Hyperliquid") ? "committed_diag_fixture" : "not_targeted_by_diag_real_responses"
      if ($9 != expected_diag) {
        printf "bad diagnostic fixture boundary for %s\n", $1 > "/dev/stderr"
        exit 1
      }
      if ($9 == "committed_diag_fixture" && ($10 == "not_applicable" || $11 == "not_applicable")) {
        printf "diagnostic fixture row for %s must name fixture ids and hashes\n", $1 > "/dev/stderr"
        exit 1
      }
      if ($9 == "not_targeted_by_diag_real_responses" && ($10 != "not_applicable" || $11 != "not_applicable")) {
        printf "non-targeted diagnostic row for %s must keep fixture ids/hashes not_applicable\n", $1 > "/dev/stderr"
        exit 1
      }
      if ($12 == "not_applicable" || $13 == "not_applicable") {
        printf "WS operation fixture row for %s must name fixture ids and hashes\n", $1 > "/dev/stderr"
        exit 1
      }
      ws_fixture_count = split($12, ws_fixtures, ",")
      ws_hash_count = split($13, ws_hashes, ",")
      if (ws_fixture_count != ws_hash_count) {
        printf "WS operation fixture/hash count mismatch for %s\n", $1 > "/dev/stderr"
        exit 1
      }
      count++
    }
    END {
      if (count != 9) {
        printf "matrix must contain exactly 9 venue rows, got %d\n", count > "/dev/stderr"
        exit 1
      }
    }
  ' "$MATRIX" || fail "bad ${MATRIX#$ROOT/}"
}

require_unique_matrix_fixture_columns() {
  awk -F '\t' '
    NR == 1 { next }
    function check_csv(kind, venue, csv, values, count, i, value, key) {
      if (csv == "not_applicable") {
        return
      }
      count = split(csv, values, ",")
      for (i = 1; i <= count; i++) {
        value = values[i]
        if (value == "" || value == "not_applicable") {
          printf "%s column for %s contains invalid fixture/hash value\n", kind, venue > "/dev/stderr"
          exit 1
        }
        key = kind SUBSEP value
        if (seen[key] != "") {
          printf "%s %s for %s duplicates first use by %s\n", kind, value, venue, seen[key] > "/dev/stderr"
          exit 1
        }
        seen[key] = venue
      }
    }
    {
      check_csv("diagnostic fixture", $1, $10)
      check_csv("diagnostic hash", $1, $11)
      check_csv("WS operation fixture", $1, $12)
      check_csv("WS operation hash", $1, $13)
    }
  ' "$MATRIX" || fail "matrix fixture/hash columns must be unique by evidence kind"
}

require_rest_bucket() {
  local venue="$1"
  local use_case="$2"
  local data_kind="$3"

  awk -F '\t' -v venue="$venue" -v use_case="$use_case" -v data_kind="$data_kind" '
    NR > 1 && $1 == venue && $4 == use_case && $5 == data_kind {
      if ($6 != "not_recorded" && $7 != "not_recorded" && $8 != "not_recorded" &&
          $9 != "not_recorded" && $10 != "not_recorded" && $11 != "not_recorded" &&
          $12 != "not_recorded" && $12 != "public") {
        found = 1
      }
    }
    END { exit found ? 0 : 1 }
  ' "$ALLOWLIST" || fail "$venue missing recorded $use_case/$data_kind REST evidence"
}

has_runnable_test_in_file() {
  local file="$1"
  local test_name="$2"
  perl -Mstrict -Mwarnings -e '
    my ($test_name, $path) = @ARGV;
    open my $fh, "<", $path or die "$path: $!";
    my $source = remove_block_comments(do { local $/; <$fh> });
    my @lines = split /\n/, $source, -1;

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
	      if (!$inside_disabled_parent && $lines[$i] =~ /^\s*(?:async\s+)?fn\s+\Q$test_name\E\s*\(/) {
	      my ($saw_test, $invalid_attr) = test_attr_state(attr_blocks_before(\@lines, $i));
	      exit 0 if $saw_test && !$invalid_attr;
	      }
	      $scope_depth += $opens - $closes;
	      while (@disabled_module_depths && $scope_depth < $disabled_module_depths[-1]) {
	        pop @disabled_module_depths;
	      }
	    }
	    exit 1;
	  ' "$test_name" "$file"
	}

require_test_fn() {
  local test_name="$1"
  has_runnable_test_in_file "$WS_TEST" "$test_name" \
    || fail "missing runnable WS operation evidence test $test_name"
}

require_registry_test_fn() {
  local test_name="$1"
  has_runnable_test_in_file "$WS_REGISTRY" "$test_name" \
    || fail "missing runnable WS operation registry test $test_name"
}

require_rest_registry_test_fn() {
  local test_name="$1"
  has_runnable_test_in_file "$REST_REGISTRY" "$test_name" \
    || fail "missing runnable REST endpoint registry test $test_name"
}

require_api_registry_test_fn() {
  local test_name="$1"
  has_runnable_test_in_file "$API_REGISTRY_TEST" "$test_name" \
    || fail "missing runnable API transport registry test $test_name"
}

require_venue_registry_test_fn() {
  local test_name="$1"
  has_runnable_test_in_file "$VENUE_SPEC" "$test_name" \
    || fail "missing runnable venue registry test $test_name"
}

require_source_literals() {
  local file="$1"
  shift
  local literal
  for literal in "$@"; do
    grep -Fq -- "$literal" "$file" \
      || fail "${file#$ROOT/} must keep source literal: $literal"
  done
}

ws_test_includes_fixture() {
  local file="$1"
  local test_name="$2"
  local fixture_suffix="$3"
  perl -Mstrict -Mwarnings -e '
    my ($test_name, $fixture_suffix, $path) = @ARGV;
    open my $fh, "<", $path or die "$path: $!";
    my $source = remove_block_comments(do { local $/; <$fh> });
    my @lines = split /\n/, $source, -1;

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
        if ($line eq "" || $line =~ /^\/\//) {
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

    sub parse_string_literal {
      my ($text, $index) = @_;
      my $len = length($text);
      if (substr($text, $index, 1) eq "\"") {
        my ($value, $i) = ("", $index + 1);
        while ($i < $len) {
          my $ch = substr($text, $i, 1);
          if ($ch eq "\\") {
            $value .= substr($text, $i + 1, 1) if $i + 1 < $len;
            $i += 2;
            next;
          }
          return ($value, $i + 1) if $ch eq "\"";
          $value .= $ch;
          $i++;
        }
        return;
      }
      if (substr($text, $index, 1) eq "r") {
        my $i = $index + 1;
        my $hashes = "";
        while ($i < $len && substr($text, $i, 1) eq "#") {
          $hashes .= "#";
          $i++;
        }
        return unless $i < $len && substr($text, $i, 1) eq "\"";
        my $end = "\"" . $hashes;
        my $body_start = $i + 1;
        my $body_end = index($text, $end, $body_start);
        return if $body_end < 0;
        return (substr($text, $body_start, $body_end - $body_start), $body_end + length($end));
      }
      return;
    }

    sub skip_line_comment {
      my ($text, $index) = @_;
      my $next = index($text, "\n", $index);
      return $next < 0 ? length($text) : $next + 1;
    }

    sub without_strings_and_line_comments {
      my ($text) = @_;
      my $out = "";
      for (my $i = 0; $i < length($text); ) {
        if (substr($text, $i, 2) eq "//") {
          $i = skip_line_comment($text, $i);
          $out .= "\n";
          next;
        }
        if (my (undef, $end) = parse_string_literal($text, $i)) {
          $out .= "\"\"";
          $i = $end;
          next;
        }
        $out .= substr($text, $i, 1);
        $i++;
      }
      return $out;
    }

    sub body_has_skip_marker {
      my ($body) = @_;
      my $scan = without_strings_and_line_comments($body);
      return 1
        if $scan =~ /\b(?:std::env::temp_dir|tempfile|load_or_skip)\b/
        || $scan =~ m{/tmp/}
        || $scan =~ /\breturn\s+Ok\s*\(\s*\(\s*\)\s*\)/
        || $scan =~ /\breturn\s*;/
        || $scan =~ /\bOk\s*\(\s*\(\s*\)\s*\)/;
      return 0;
    }

    sub matching_close_paren {
      my ($text, $open_index) = @_;
      my ($depth, $len) = (0, length($text));
      for (my $i = $open_index; $i < $len; ) {
        if (substr($text, $i, 2) eq "//") {
          $i = skip_line_comment($text, $i);
          next;
        }
        if (my (undef, $end) = parse_string_literal($text, $i)) {
          $i = $end;
          next;
        }
        my $ch = substr($text, $i, 1);
        if ($ch eq "(") {
          $depth++;
        } elsif ($ch eq ")") {
          $depth--;
          return $i if $depth == 0;
        }
        $i++;
      }
      return;
    }

    sub macro_span_matches_fixture {
      my ($span, $fixture_suffix) = @_;
      for (my $i = 0; $i < length($span); ) {
        if (substr($span, $i, 2) eq "//") {
          $i = skip_line_comment($span, $i);
          next;
        }
        if (my ($literal, $end) = parse_string_literal($span, $i)) {
          $literal =~ s{^/+}{};
          return 1 if length($literal) >= length($fixture_suffix)
            && substr($literal, -length($fixture_suffix)) eq $fixture_suffix;
          $i = $end;
          next;
        }
        $i++;
      }
      return 0;
    }

    sub body_has_fixture_include {
      my ($body, $fixture_suffix) = @_;
      for (my $i = 0; $i < length($body); ) {
        if (substr($body, $i, 2) eq "//") {
          $i = skip_line_comment($body, $i);
          next;
        }
        if (my (undef, $end) = parse_string_literal($body, $i)) {
          $i = $end;
          next;
        }
        if (substr($body, $i) =~ /\Ainclude_(?:str|bytes)!/) {
          my $j = $i + length($&);
          $j++ while $j < length($body) && substr($body, $j, 1) =~ /\s/;
          if ($j < length($body) && substr($body, $j, 1) eq "(") {
            if (my $close = matching_close_paren($body, $j)) {
              return 1 if macro_span_matches_fixture(substr($body, $j + 1, $close - $j - 1), $fixture_suffix);
              $i = $close + 1;
              next;
            }
          }
        }
        $i++;
      }
      return 0;
    }

    sub collect_body {
      my ($lines, $start) = @_;
      my ($body, $depth, $saw_open) = ("", 0, 0);
      for (my $j = $start; $j <= $#$lines; $j++) {
        my $line = $lines->[$j];
        my $scan = brace_scan_line($line);
        $saw_open = 1 if $scan =~ /\{/;
        $depth += (() = $scan =~ /\{/g);
        $depth -= (() = $scan =~ /\}/g);
        $body .= $line . "\n";
        return $body if $saw_open && $depth <= 0;
      }
      return "";
    }

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
      if (!$inside_disabled_parent && $lines[$i] =~ /^\s*(?:async\s+)?fn\s+\Q$test_name\E\s*\(/) {
        my ($saw_test, $invalid_attr) = test_attr_state(attr_blocks_before(\@lines, $i));
        if ($saw_test && !$invalid_attr) {
          my $body = collect_body(\@lines, $i);
          exit 0
            if $body ne ""
            && body_has_fixture_include($body, $fixture_suffix)
            && !body_has_skip_marker($body);
        }
      }
      $scope_depth += $opens - $closes;
      while (@disabled_module_depths && $scope_depth < $disabled_module_depths[-1]) {
        pop @disabled_module_depths;
      }
    }
    exit 1;
  ' "$test_name" "$fixture_suffix" "$file"
}

require_ws_parser_fixture() {
  local venue="$1"
  local label="$2"
  local file="$3"
  local test_name="$4"
  local fixture_suffix="$5"
  ws_test_includes_fixture "$file" "$test_name" "$fixture_suffix" \
    || fail "$venue $label parser test $test_name must directly include $fixture_suffix in a non-skipping test"
}

expected_ws_operation_fixture_paths() {
  cat <<'EOF'
crates/exchange/fixtures/binance/ws_order_place_success.json
crates/exchange/fixtures/binance/ws_order_cancel_success.json
crates/exchange/fixtures/okx/ws_trade_place_order_ack.json
crates/exchange/fixtures/okx/ws_trade_cancel_order_ack.json
crates/exchange/fixtures/gate/ws_futures_order_place_success.json
crates/exchange/fixtures/gate/ws_futures_order_cancel_success.json
crates/exchange/fixtures/hyperliquid/ws_post_order_resting.json
crates/exchange/fixtures/hyperliquid/ws_post_cancel_success.json
crates/exchange/fixtures/bybit/ws_order_create_ack.json
crates/exchange/fixtures/bybit/ws_order_cancel_ack.json
crates/exchange/fixtures/bitget/uta_ws_place_order_ack.json
crates/exchange/fixtures/bitget/uta_ws_cancel_order_ack.json
crates/exchange/fixtures/kucoin/wsapi_pro_order_ack.json
crates/exchange/fixtures/kucoin/wsapi_pro_cancel_ack.json
crates/exchange/fixtures/gate_crossex/place_order_ack.json
crates/exchange/fixtures/gate_crossex/cancel_order_ack.json
crates/exchange/fixtures/kraken/spot_v2_add_order_ack.json
crates/exchange/fixtures/kraken/spot_v2_cancel_order_ack.json
EOF
}

matrix_ws_operation_fixture_paths() {
  awk -F '\t' '
    NR > 1 {
      split($12, fixtures, ",")
      for (fixture_index in fixtures) {
        print fixtures[fixture_index]
      }
    }
  ' "$MATRIX" | sort -u
}

require_ws_operation_fixture_closure() {
  local expected actual
  expected="$(mktemp "${TMPDIR:-/tmp}/crossline-ws-operation-expected.XXXXXX")"
  actual="$(mktemp "${TMPDIR:-/tmp}/crossline-ws-operation-actual.XXXXXX")"
  expected_ws_operation_fixture_paths | sort -u >"$expected"
  matrix_ws_operation_fixture_paths >"$actual"
  if ! cmp -s "$expected" "$actual"; then
    diff -u "$expected" "$actual" >&2 || true
    fail "WS operation fixture matrix must exactly match parser evidence fixture set"
  fi
  rm -f "$expected" "$actual"
}

matrix_ws_operation_fixture_hash() {
  local venue="$1"
  local fixture="$2"
  awk -F '\t' -v venue="$venue" -v fixture="$fixture" '
    BEGIN { seen = 0; found = 0 }
    NR > 1 && tolower($1) == tolower(venue) {
      seen = 1
      split($12, fixtures, ",")
      split($13, hashes, ",")
      for (fixture_index in fixtures) {
        if (fixtures[fixture_index] == fixture) {
          print hashes[fixture_index]
          found++
        }
      }
    }
    END { exit (seen && found == 1) ? 0 : 1 }
  ' "$MATRIX"
}

require_ws_operation_fixture() {
  local venue="$1"
  local label="$2"
  local file="$3"
  local test_name="$4"
  local fixture_suffix="$5"
  local fixture="crates/exchange/$fixture_suffix"
  local expected_hash
  expected_hash="$(matrix_ws_operation_fixture_hash "$venue" "$fixture")" \
    || fail "$venue $label fixture must be hash-pinned in operation matrix: $fixture"
  require_pinned_fixture "$venue" "$fixture" "$expected_hash" "WS operation"
  require_ws_parser_fixture "$venue" "$label" "$file" "$test_name" "$fixture_suffix"
}

require_ws_parser_fixtures() {
  require_ws_operation_fixture_closure
  require_ws_operation_fixture "binance" "place_order" "$ROOT/crates/exchange/src/adapters/binance_ws_trade_tests.rs" "binance_ws_place_order_ack_parses_official_fixture" "fixtures/binance/ws_order_place_success.json"
  require_ws_operation_fixture "binance" "cancel_order" "$ROOT/crates/exchange/src/adapters/binance_ws_trade_tests.rs" "binance_ws_cancel_order_ack_parses_official_fixture" "fixtures/binance/ws_order_cancel_success.json"
  require_ws_operation_fixture "okx" "place_order" "$ROOT/crates/exchange/src/adapters/okx_ws_trade_tests.rs" "okx_ws_place_order_ack_parses_official_fixture" "fixtures/okx/ws_trade_place_order_ack.json"
  require_ws_operation_fixture "okx" "cancel_order" "$ROOT/crates/exchange/src/adapters/okx_ws_trade_tests.rs" "okx_ws_cancel_order_ack_parses_official_fixture" "fixtures/okx/ws_trade_cancel_order_ack.json"
  require_ws_operation_fixture "gate" "place_order" "$ROOT/crates/exchange/src/adapters/gate_ws_trade_tests.rs" "gate_ws_order_place_ack_response_parses_official_fixture" "fixtures/gate/ws_futures_order_place_success.json"
  require_ws_operation_fixture "gate" "cancel_order" "$ROOT/crates/exchange/src/adapters/gate_ws_trade_tests.rs" "gate_ws_order_cancel_ack_response_parses_official_fixture" "fixtures/gate/ws_futures_order_cancel_success.json"
  require_ws_operation_fixture "hyperliquid" "place_order" "$ROOT/crates/exchange/src/adapters/hyperliquid_ws_trade_tests.rs" "hyperliquid_ws_place_order_ack_parses_official_fixture" "fixtures/hyperliquid/ws_post_order_resting.json"
  require_ws_operation_fixture "hyperliquid" "cancel_order" "$ROOT/crates/exchange/src/adapters/hyperliquid_ws_trade_tests.rs" "hyperliquid_ws_cancel_order_ack_parses_official_fixture" "fixtures/hyperliquid/ws_post_cancel_success.json"
  require_ws_operation_fixture "bybit" "place_order" "$ROOT/crates/exchange/src/adapters/bybit_ws_trade_tests.rs" "bybit_ws_place_order_ack_parses_official_fixture" "fixtures/bybit/ws_order_create_ack.json"
  require_ws_operation_fixture "bybit" "cancel_order" "$ROOT/crates/exchange/src/adapters/bybit_ws_trade_tests.rs" "bybit_ws_cancel_order_ack_parses_official_fixture" "fixtures/bybit/ws_order_cancel_ack.json"
  require_ws_operation_fixture "bitget" "place_order" "$ROOT/crates/exchange/src/adapters/bitget_uta_ws_trade_tests.rs" "bitget_uta_ws_place_order_ack_parses_official_fixture" "fixtures/bitget/uta_ws_place_order_ack.json"
  require_ws_operation_fixture "bitget" "cancel_order" "$ROOT/crates/exchange/src/adapters/bitget_uta_ws_trade_tests.rs" "bitget_uta_ws_cancel_order_ack_parses_official_fixture" "fixtures/bitget/uta_ws_cancel_order_ack.json"
  require_ws_operation_fixture "kucoin" "place_order" "$ROOT/crates/exchange/src/adapters/kucoin_ws_user_tests.rs" "parses_order_balance_position_and_pro_ack" "fixtures/kucoin/wsapi_pro_order_ack.json"
  require_ws_operation_fixture "kucoin" "cancel_order" "$ROOT/crates/exchange/src/adapters/kucoin_ws_user_tests.rs" "parses_order_balance_position_and_pro_ack" "fixtures/kucoin/wsapi_pro_cancel_ack.json"
  require_ws_operation_fixture "gate_crossex" "place_order" "$ROOT/crates/exchange/src/adapters/gate_crossex_private_data_tests.rs" "parses_ws_api_ack_and_rejects_wrong_request" "fixtures/gate_crossex/place_order_ack.json"
  require_ws_operation_fixture "gate_crossex" "cancel_order" "$ROOT/crates/exchange/src/adapters/gate_crossex_private_data_tests.rs" "parses_ws_api_ack_and_rejects_wrong_request" "fixtures/gate_crossex/cancel_order_ack.json"
  require_ws_operation_fixture "kraken" "place_order" "$ROOT/crates/exchange/src/adapters/kraken_spot_ws_private.rs" "spot_v2_write_responses_parse_official_fixtures" "fixtures/kraken/spot_v2_add_order_ack.json"
  require_ws_operation_fixture "kraken" "cancel_order" "$ROOT/crates/exchange/src/adapters/kraken_spot_ws_private.rs" "spot_v2_write_responses_parse_official_fixtures" "fixtures/kraken/spot_v2_cancel_order_ack.json"
}

diag_test_includes_fixture() {
  local file="$1"
  local include_path="$2"
  local expected_test_name="${3:-}"
  perl -Mstrict -Mwarnings -e '
    my ($include_path, $path, $expected_test_name) = @ARGV;
    open my $fh, "<", $path or die "$path: $!";
    my $source = remove_block_comments(do { local $/; <$fh> });
    my @lines = split /\n/, $source, -1;
    my ($in_test, $brace_depth, $body) = (0, 0, "");

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

    sub parse_string_literal {
      my ($text, $index) = @_;
      my $len = length($text);
      if (substr($text, $index, 1) eq "\"") {
        my ($value, $i) = ("", $index + 1);
        while ($i < $len) {
          my $ch = substr($text, $i, 1);
          if ($ch eq "\\") {
            $value .= substr($text, $i + 1, 1) if $i + 1 < $len;
            $i += 2;
            next;
          }
          return ($value, $i + 1) if $ch eq "\"";
          $value .= $ch;
          $i++;
        }
        return;
      }
      if (substr($text, $index, 1) eq "r") {
        my $i = $index + 1;
        my $hashes = "";
        while ($i < $len && substr($text, $i, 1) eq "#") {
          $hashes .= "#";
          $i++;
        }
        return unless $i < $len && substr($text, $i, 1) eq "\"";
        my $end = "\"" . $hashes;
        my $body_start = $i + 1;
        my $body_end = index($text, $end, $body_start);
        return if $body_end < 0;
        return (substr($text, $body_start, $body_end - $body_start), $body_end + length($end));
      }
      return;
    }

    sub skip_string_literal {
      my ($text, $index) = @_;
      my (undef, $end) = parse_string_literal($text, $index);
      return $end;
    }

    sub include_macro_paths {
      my ($body) = @_;
      my @paths;
      my $len = length($body);
      for (my $i = 0; $i < $len; ) {
        if (defined(my $end = skip_string_literal($body, $i))) {
          $i = $end;
          next;
        }
        if (substr($body, $i) =~ /\Ainclude_(?:str|bytes)!/) {
          my $macro_len = length($&);
          my $j = $i + $macro_len;
          $j++ while $j < $len && substr($body, $j, 1) =~ /\s/;
          if ($j < $len && substr($body, $j, 1) eq "(") {
            $j++;
            $j++ while $j < $len && substr($body, $j, 1) =~ /\s/;
            if (my ($path_value, $path_end) = parse_string_literal($body, $j)) {
              my $k = $path_end;
              $k++ while $k < $len && substr($body, $k, 1) =~ /\s/;
              push @paths, $path_value if $k < $len && substr($body, $k, 1) eq ")";
            }
          }
        }
        $i++;
      }
      return @paths;
    }

    sub body_has_include_macro {
      my ($body, $include_path) = @_;
      for my $path (include_macro_paths($body)) {
        return 1 if $path eq $include_path;
      }
      return 0;
    }

	    my @disabled_module_depths;
	    my $scope_depth = 0;
	    for (my $idx = 0; $idx <= $#lines; $idx++) {
	      my $line = $lines[$idx];
	      while (@disabled_module_depths && $scope_depth < $disabled_module_depths[-1]) {
	        pop @disabled_module_depths;
	      }
	      my $inside_disabled_parent = @disabled_module_depths > 0;
	      my $depth_line = brace_scan_line($line);
	      my $opens = () = $depth_line =~ /\{/g;
	      my $closes = () = $depth_line =~ /\}/g;
	      if (!$in_test && $line =~ /^\s*(?:pub(?:\s*\([^)]*\))?\s+)?mod\s+[A-Za-z0-9_]+\s*\{/) {
	        my $module_depth = $scope_depth + $opens - $closes;
	        if ($module_depth > $scope_depth &&
	            ($inside_disabled_parent || module_attr_disables_scope(attr_blocks_before(\@lines, $idx)))) {
	          push @disabled_module_depths, $module_depth;
	        }
	      }
	      my $trim = $line;
	      $trim =~ s/^\s+|\s+$//g;
	      if (!$in_test && $trim eq "") {
	        $scope_depth += $opens - $closes;
	        next;
	      }
	      my $matches_diag_fn = $expected_test_name ne ""
	        ? $line =~ /^\s*async\s+fn\s+\Q$expected_test_name\E\s*\(\)\s*\{/
	        : $line =~ /^\s*async\s+fn\s+diagnose_[A-Za-z0-9_]+_real_response\s*\(\)\s*\{/;
	      if (!$in_test && $matches_diag_fn) {
	        my ($saw_test, $invalid_attr) = test_attr_state(attr_blocks_before(\@lines, $idx));
	        $in_test = !$inside_disabled_parent && $saw_test && !$invalid_attr;
	        $brace_depth = 1;
	        $body = "";
	        $scope_depth += $opens - $closes;
	        next;
	      }
	      if (!$in_test) {
	        $scope_depth += $opens - $closes;
	        next;
	      }
	      my $body_line = $line;
	      $body_line =~ s{//.*$}{};
	      $body .= $body_line;
	      $brace_depth += $opens - $closes;
	      if ($brace_depth <= 0) {
	        exit 0 if body_has_include_macro($body, $include_path);
	        $in_test = 0;
	        $body = "";
	      }
	      $scope_depth += $opens - $closes;
	      while (@disabled_module_depths && $scope_depth < $disabled_module_depths[-1]) {
	        pop @disabled_module_depths;
	      }
	    }
	    exit 1;
	  ' "$include_path" "$file" "$expected_test_name"
	}

diag_test_fixture_includes() {
  local file="$1"
  perl -Mstrict -Mwarnings -e '
    my ($path) = @ARGV;
    open my $fh, "<", $path or die "$path: $!";
    my $source = remove_block_comments(do { local $/; <$fh> });
    my @lines = split /\n/, $source, -1;
    my ($in_test, $brace_depth, $body) = (0, 0, "");

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
        if ($line eq "" || $line =~ /^\/\//) {
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

    sub parse_string_literal {
      my ($text, $index) = @_;
      my $len = length($text);
      if (substr($text, $index, 1) eq "\"") {
        my ($value, $i) = ("", $index + 1);
        while ($i < $len) {
          my $ch = substr($text, $i, 1);
          if ($ch eq "\\") {
            $value .= substr($text, $i + 1, 1) if $i + 1 < $len;
            $i += 2;
            next;
          }
          return ($value, $i + 1) if $ch eq "\"";
          $value .= $ch;
          $i++;
        }
        return;
      }
      if (substr($text, $index, 1) eq "r") {
        my $i = $index + 1;
        my $hashes = "";
        while ($i < $len && substr($text, $i, 1) eq "#") {
          $hashes .= "#";
          $i++;
        }
        return unless $i < $len && substr($text, $i, 1) eq "\"";
        my $end = "\"" . $hashes;
        my $body_start = $i + 1;
        my $body_end = index($text, $end, $body_start);
        return if $body_end < 0;
        return (substr($text, $body_start, $body_end - $body_start), $body_end + length($end));
      }
      return;
    }

    sub skip_string_literal {
      my ($text, $index) = @_;
      my (undef, $end) = parse_string_literal($text, $index);
      return $end;
    }

    sub include_macro_paths {
      my ($body) = @_;
      my @paths;
      my $len = length($body);
      for (my $i = 0; $i < $len; ) {
        if (defined(my $end = skip_string_literal($body, $i))) {
          $i = $end;
          next;
        }
        if (substr($body, $i) =~ /\Ainclude_(?:str|bytes)!/) {
          my $macro_len = length($&);
          my $j = $i + $macro_len;
          $j++ while $j < $len && substr($body, $j, 1) =~ /\s/;
          if ($j < $len && substr($body, $j, 1) eq "(") {
            $j++;
            $j++ while $j < $len && substr($body, $j, 1) =~ /\s/;
            if (my ($path_value, $path_end) = parse_string_literal($body, $j)) {
              my $k = $path_end;
              $k++ while $k < $len && substr($body, $k, 1) =~ /\s/;
              push @paths, $path_value if $k < $len && substr($body, $k, 1) eq ")";
            }
          }
        }
        $i++;
      }
      return @paths;
    }

	    my @disabled_module_depths;
	    my $scope_depth = 0;
	    for (my $idx = 0; $idx <= $#lines; $idx++) {
	      my $line = $lines[$idx];
	      while (@disabled_module_depths && $scope_depth < $disabled_module_depths[-1]) {
	        pop @disabled_module_depths;
	      }
	      my $inside_disabled_parent = @disabled_module_depths > 0;
	      my $depth_line = brace_scan_line($line);
	      my $opens = () = $depth_line =~ /\{/g;
	      my $closes = () = $depth_line =~ /\}/g;
	      if (!$in_test && $line =~ /^\s*(?:pub(?:\s*\([^)]*\))?\s+)?mod\s+[A-Za-z0-9_]+\s*\{/) {
	        my $module_depth = $scope_depth + $opens - $closes;
	        if ($module_depth > $scope_depth &&
	            ($inside_disabled_parent || module_attr_disables_scope(attr_blocks_before(\@lines, $idx)))) {
	          push @disabled_module_depths, $module_depth;
	        }
	      }
	      my $trim = $line;
	      $trim =~ s/^\s+|\s+$//g;
	      if (!$in_test && $trim eq "") {
	        $scope_depth += $opens - $closes;
	        next;
	      }
	      if (!$in_test && $line =~ /^\s*async\s+fn\s+diagnose_[A-Za-z0-9_]+_real_response\s*\(\)\s*\{/) {
	        my ($saw_test, $invalid_attr) = test_attr_state(attr_blocks_before(\@lines, $idx));
	        $in_test = !$inside_disabled_parent && $saw_test && !$invalid_attr;
	        $brace_depth = 1;
	        $body = "";
	        $scope_depth += $opens - $closes;
	        next;
	      }
	      if (!$in_test) {
	        $scope_depth += $opens - $closes;
	        next;
	      }
	      my $body_line = $line;
	      $body_line =~ s{//.*$}{};
	      $body .= $body_line;
	      $brace_depth += $opens - $closes;
	      if ($brace_depth <= 0) {
	        print "$_\n" for include_macro_paths($body);
	        $in_test = 0;
	        $body = "";
	      }
	      $scope_depth += $opens - $closes;
	      while (@disabled_module_depths && $scope_depth < $disabled_module_depths[-1]) {
	        pop @disabled_module_depths;
	      }
	    }
	  ' "$file"
	}

require_diag_test_include() {
  local venue="$1"
  local fixture="$2"
  local venue_slug expected_test_name
  venue_slug="$(printf '%s' "$venue" | tr '[:upper:]' '[:lower:]')"
  expected_test_name="diagnose_${venue_slug}_real_response"
  local include_path="../${fixture#crates/exchange/}"
  diag_test_includes_fixture "$DIAG_TEST" "$include_path" "$expected_test_name" \
    || fail "diag_real_responses.rs does not include $include_path from runnable $expected_test_name"
}

diag_fixture_matches_venue() {
  local venue="$1"
  local fixture="$2"
  local venue_slug
  venue_slug="$(printf '%s' "$venue" | tr '[:upper:]' '[:lower:]')"
  case "$fixture" in
    "crates/exchange/fixtures/${venue_slug}/"*) return 0 ;;
    *) return 1 ;;
  esac
}

fixture_hash_is_canonical() {
  local hash="$1"
  [[ "$hash" =~ ^sha256:[0-9a-f]{64}$ ]]
}

require_pinned_fixture() {
  local venue="$1"
  local fixture="$2"
  local expected_hash="$3"
  local kind="$4"
  diag_fixture_matches_venue "$venue" "$fixture" \
    || fail "$venue $kind fixture must live under its venue fixture directory: $fixture"
  fixture_hash_is_canonical "$expected_hash" \
    || fail "$venue $kind fixture hash must use sha256:<64 lowercase hex>: $fixture"
  [ -s "$ROOT/$fixture" ] || fail "missing committed $kind fixture $fixture"
  git -C "$ROOT" ls-files --error-unmatch "$fixture" >/dev/null \
    || fail "$kind fixture is not git-tracked: $fixture"
  local actual_hash
  actual_hash="$(shasum -a 256 "$ROOT/$fixture" | awk '{print $1}')"
  expected_hash="${expected_hash#sha256:}"
  [ "$actual_hash" = "$expected_hash" ] || fail "$kind fixture hash drifted for $fixture"
}

matrix_diag_include_paths() {
  awk -F '\t' '
    NR > 1 && $9 == "committed_diag_fixture" {
      split($10, fixtures, ",")
      for (fixture_index in fixtures) {
        sub(/^crates\/exchange\//, "../", fixtures[fixture_index])
        print fixtures[fixture_index]
      }
    }
  ' "$MATRIX" | sort -u
}

require_diag_include_closure() {
  local expected actual include_path
  expected="$(mktemp "${TMPDIR:-/tmp}/crossline-diag-expected.XXXXXX")"
  actual="$(mktemp "${TMPDIR:-/tmp}/crossline-diag-actual.XXXXXX")"
  matrix_diag_include_paths >"$expected"
  diag_test_fixture_includes "$DIAG_TEST" | sort -u >"$actual"

  while IFS= read -r include_path; do
    [ -n "$include_path" ] || continue
    grep -Fxq -- "$include_path" "$expected" \
      || fail "diag_real_responses.rs includes unpinned diagnostic fixture $include_path"
  done <"$actual"

  rm -f "$expected" "$actual"
}

require_diag_fixture() {
  local venue="$1"
  local fixture="$2"
  local expected_hash="$3"
  require_pinned_fixture "$venue" "$fixture" "$expected_hash" "diagnostic"
  require_diag_test_include "$venue" "$fixture"
}

require_diag_fixtures() {
  local venue="$1"
  local boundary="$2"
  local fixture_ids="$3"
  local fixture_hashes="$4"

  if [ "$boundary" = "not_targeted_by_diag_real_responses" ]; then
    [ "$fixture_ids" = "not_applicable" ] && [ "$fixture_hashes" = "not_applicable" ] \
      || fail "$venue diagnostic fixture metadata must be not_applicable"
    return
  fi

  [ "$boundary" = "committed_diag_fixture" ] || fail "$venue has unknown diagnostic fixture boundary $boundary"
  IFS=',' read -r -a fixtures <<< "$fixture_ids"
  IFS=',' read -r -a hashes <<< "$fixture_hashes"
  [ "${#fixtures[@]}" -gt 0 ] || fail "$venue missing diagnostic fixture ids"
  [ "${#fixtures[@]}" -eq "${#hashes[@]}" ] || fail "$venue diagnostic fixture/hash count mismatch"

  local index
  for index in "${!fixtures[@]}"; do
    require_diag_fixture "$venue" "${fixtures[$index]}" "${hashes[$index]}"
  done
}

run_self_test() {
  local tmp ws_fixture diag_fixture matrix_fixture source_anchor_fixture canonical_hash
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/crossline-operation-evidence-self-test.XXXXXX")"
  ws_fixture="$tmp/ws_tests.rs"
  diag_fixture="$tmp/diag_tests.rs"
  matrix_fixture="$tmp/matrix.tsv"
  source_anchor_fixture="$tmp/source_anchor.rs"
  canonical_hash="sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
  trap 'rm -rf "$tmp"' RETURN

  cat >"$source_anchor_fixture" <<'EOF'
include_str!("exchange_operation_evidence_matrix.tsv");
const REST_COLUMNS: &[&str] = &[
  "rest_trade_write_order_ack",
  "rest_private_order_status",
  "rest_private_account_balance",
  "rest_private_account_position",
];
EOF
  require_source_literals \
    "$source_anchor_fixture" \
    "exchange_operation_evidence_matrix.tsv" \
    "rest_trade_write_order_ack" \
    "rest_private_order_status" \
    "rest_private_account_balance" \
    "rest_private_account_position"
  if (require_source_literals "$source_anchor_fixture" "missing_matrix_literal") 2>/dev/null; then
    fail "self-test accepted missing source literal"
  fi

  diag_fixture_matches_venue "Bybit" "crates/exchange/fixtures/bybit/example.json" \
    || fail "self-test rejected venue-owned diagnostic fixture path"
  if diag_fixture_matches_venue "Bybit" "crates/exchange/fixtures/okx/example.json" \
    || diag_fixture_matches_venue "Bybit" "fixtures/bybit/example.json"; then
    fail "self-test accepted cross-venue or non-canonical diagnostic fixture path"
  fi
  fixture_hash_is_canonical "$canonical_hash" \
    || fail "self-test rejected canonical diagnostic fixture hash"
  if fixture_hash_is_canonical "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef" \
    || fixture_hash_is_canonical "sha256:0123456789ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef" \
    || fixture_hash_is_canonical "sha256:0123456789abcdef"; then
    fail "self-test accepted bare, uppercase, or short diagnostic fixture hash"
  fi
  cat >"$matrix_fixture" <<EOF
venue	rest_trade_write_order_ack	rest_private_order_status	rest_private_account_balance	rest_private_account_position	ws_live_write_path	ws_private_stream_evidence	close_position_boundary	diagnostic_fixture_boundary	diagnostic_fixture_ids	diagnostic_fixture_hashes	ws_operation_fixture_ids	ws_operation_fixture_hashes	finality_boundary
Binance	recorded	recorded	recorded	recorded	recorded_place_cancel	recorded	display_only_without_operation_evidence	committed_diag_fixture	crates/exchange/fixtures/binance/diag.json	$canonical_hash	crates/exchange/fixtures/binance/ws.json	$canonical_hash	ack_not_final
EOF
  MATRIX="$matrix_fixture"
  require_unique_matrix_fixture_columns
  cat >"$matrix_fixture" <<EOF
venue	rest_trade_write_order_ack	rest_private_order_status	rest_private_account_balance	rest_private_account_position	ws_live_write_path	ws_private_stream_evidence	close_position_boundary	diagnostic_fixture_boundary	diagnostic_fixture_ids	diagnostic_fixture_hashes	ws_operation_fixture_ids	ws_operation_fixture_hashes	finality_boundary
Binance	recorded	recorded	recorded	recorded	recorded_place_cancel	recorded	display_only_without_operation_evidence	committed_diag_fixture	crates/exchange/fixtures/binance/diag.json	sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa	crates/exchange/fixtures/binance/ws-a.json	sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb	ack_not_final
Okx	recorded	recorded	recorded	recorded	recorded_place_cancel	recorded	display_only_without_operation_evidence	committed_diag_fixture	crates/exchange/fixtures/binance/diag.json	sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc	crates/exchange/fixtures/okx/ws-a.json	sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd	ack_not_final
EOF
  if (require_unique_matrix_fixture_columns) 2>/dev/null; then
    fail "self-test accepted duplicate diagnostic fixture path"
  fi
  cat >"$matrix_fixture" <<EOF
venue	rest_trade_write_order_ack	rest_private_order_status	rest_private_account_balance	rest_private_account_position	ws_live_write_path	ws_private_stream_evidence	close_position_boundary	diagnostic_fixture_boundary	diagnostic_fixture_ids	diagnostic_fixture_hashes	ws_operation_fixture_ids	ws_operation_fixture_hashes	finality_boundary
Binance	recorded	recorded	recorded	recorded	recorded_place_cancel	recorded	display_only_without_operation_evidence	committed_diag_fixture	crates/exchange/fixtures/binance/diag-a.json	sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa	crates/exchange/fixtures/binance/ws-a.json	sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb	ack_not_final
Okx	recorded	recorded	recorded	recorded	recorded_place_cancel	recorded	display_only_without_operation_evidence	committed_diag_fixture	crates/exchange/fixtures/okx/diag-a.json	sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa	crates/exchange/fixtures/okx/ws-a.json	sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd	ack_not_final
EOF
  if (require_unique_matrix_fixture_columns) 2>/dev/null; then
    fail "self-test accepted duplicate diagnostic fixture hash"
  fi
  cat >"$matrix_fixture" <<EOF
venue	rest_trade_write_order_ack	rest_private_order_status	rest_private_account_balance	rest_private_account_position	ws_live_write_path	ws_private_stream_evidence	close_position_boundary	diagnostic_fixture_boundary	diagnostic_fixture_ids	diagnostic_fixture_hashes	ws_operation_fixture_ids	ws_operation_fixture_hashes	finality_boundary
Binance	recorded	recorded	recorded	recorded	recorded_place_cancel	recorded	display_only_without_operation_evidence	committed_diag_fixture	crates/exchange/fixtures/binance/diag-a.json	sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa	crates/exchange/fixtures/binance/ws-a.json	sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb	ack_not_final
Okx	recorded	recorded	recorded	recorded	recorded_place_cancel	recorded	display_only_without_operation_evidence	committed_diag_fixture	crates/exchange/fixtures/okx/diag-a.json	sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc	crates/exchange/fixtures/binance/ws-a.json	sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd	ack_not_final
EOF
  if (require_unique_matrix_fixture_columns) 2>/dev/null; then
    fail "self-test accepted duplicate WS operation fixture path"
  fi
  cat >"$matrix_fixture" <<EOF
venue	rest_trade_write_order_ack	rest_private_order_status	rest_private_account_balance	rest_private_account_position	ws_live_write_path	ws_private_stream_evidence	close_position_boundary	diagnostic_fixture_boundary	diagnostic_fixture_ids	diagnostic_fixture_hashes	ws_operation_fixture_ids	ws_operation_fixture_hashes	finality_boundary
Binance	recorded	recorded	recorded	recorded	recorded_place_cancel	recorded	display_only_without_operation_evidence	committed_diag_fixture	crates/exchange/fixtures/binance/diag-a.json	sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa	crates/exchange/fixtures/binance/ws-a.json	sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb	ack_not_final
Okx	recorded	recorded	recorded	recorded	recorded_place_cancel	recorded	display_only_without_operation_evidence	committed_diag_fixture	crates/exchange/fixtures/okx/diag-a.json	sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc	crates/exchange/fixtures/okx/ws-a.json	sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb	ack_not_final
EOF
  if (require_unique_matrix_fixture_columns) 2>/dev/null; then
    fail "self-test accepted duplicate WS operation fixture hash"
  fi
  cat >"$matrix_fixture" <<EOF
venue	rest_trade_write_order_ack	rest_private_order_status	rest_private_account_balance	rest_private_account_position	ws_live_write_path	ws_private_stream_evidence	close_position_boundary	diagnostic_fixture_boundary	diagnostic_fixture_ids	diagnostic_fixture_hashes	ws_operation_fixture_ids	ws_operation_fixture_hashes	finality_boundary
Binance	recorded	recorded	recorded	recorded	recorded_place_cancel	recorded	display_only_without_operation_evidence	committed_diag_fixture	crates/exchange/fixtures/binance/diag.json	$canonical_hash	crates/exchange/fixtures/binance/ws.json	$canonical_hash	ack_not_final
EOF
  [ "$(matrix_ws_operation_fixture_hash "Binance" "crates/exchange/fixtures/binance/ws.json")" = "$canonical_hash" ] \
    || fail "self-test could not resolve WS operation fixture hash from matrix"
  if matrix_ws_operation_fixture_hash "Binance" "crates/exchange/fixtures/binance/missing.json" >/dev/null; then
    fail "self-test accepted unpinned WS operation fixture"
  fi

  cat >"$ws_fixture" <<'EOF'
#[test]
fn accepted_plain_test() {}

#[tokio::test]
async fn accepted_tokio_test() {}

mod enabled_parent {
    #[test]
    fn accepted_nested_test() {}
}

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
    fn parent_cfg_disabled_test() {}
}

#[cfg_attr(all(), cfg(any()))]
mod cfg_attr_disabled_parent {
    #[test]
    fn parent_cfg_attr_disabled_test() {}
}

#[cfg(any())]
// cfg reason must not hide the disabling parent module
mod disabled_parent_with_comment_gap {
    #[test]
    fn parent_cfg_comment_gap_test() {}
}

#[cfg_attr(any(), ignore)]
mod ignored_parent {
    #[test]
    fn parent_cfg_attr_ignore_test() {}
}

#[test]
fn fixture_direct_ok() {
    let _body = include_str!("../fixtures/ws/direct.json");
}

#[test]
fn fixture_multiline_ok() {
    let _body = include_bytes!(
        "../fixtures/ws/multiline.json"
    );
}

#[test]
fn fixture_concat_ok() {
    let _body = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/ws/concat.json"
    ));
}

#[test]
fn fixture_skip_marker_string_ok() {
    let _body = include_str!("../fixtures/ws/string-marker-ok.json");
    let _note = "return Ok(()) load_or_skip /tmp/";
}

const INDIRECT_FIXTURE: &str = include_str!("../fixtures/ws/indirect.json");

#[test]
fn fixture_indirect_const_fails() {
    let _body = INDIRECT_FIXTURE;
}

fn helper_fixture() -> &'static str {
    include_str!("../fixtures/ws/helper.json")
}

#[test]
fn fixture_helper_fails() {
    let _body = helper_fixture();
}

#[test]
fn fixture_comment_only_fails() {
    // let _body = include_str!("../fixtures/ws/comment.json");
}

#[test]
fn fixture_string_literal_fails() {
    let _note = "include_str!(\"../fixtures/ws/string.json\")";
}

#[test]
fn fixture_wrong_fails() {
    let _body = include_str!("../fixtures/ws/wrong.json");
}

#[test]
fn fixture_return_unit_fails() {
    let _body = include_str!("../fixtures/ws/return-unit.json");
    return;
}

#[test]
fn fixture_return_ok_fails() -> Result<(), ()> {
    let _body = include_str!("../fixtures/ws/return-ok.json");
    return Ok(());
}

#[test]
fn fixture_load_or_skip_fails() {
    let _body = include_str!("../fixtures/ws/load-or-skip.json");
    load_or_skip();
}

#[test]
fn fixture_temp_dir_fails() {
    let _body = include_str!("../fixtures/ws/temp-dir.json");
    let _path = std::env::temp_dir();
}

#[cfg(any())]
mod disabled_fixture_parent {
    #[test]
    fn fixture_disabled_parent_fails() {
        let _body = include_str!("../fixtures/ws/disabled.json");
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

	  has_runnable_test_in_file "$ws_fixture" accepted_plain_test
	  has_runnable_test_in_file "$ws_fixture" accepted_tokio_test
	  has_runnable_test_in_file "$ws_fixture" accepted_nested_test
	  if has_runnable_test_in_file "$ws_fixture" ignored_test \
	    || has_runnable_test_in_file "$ws_fixture" panic_expected_test \
	    || has_runnable_test_in_file "$ws_fixture" cfg_panic_expected_test \
    || has_runnable_test_in_file "$ws_fixture" cfg_disabled_with_comment_gap_test \
    || has_runnable_test_in_file "$ws_fixture" ignored_with_comment_gap_test \
    || has_runnable_test_in_file "$ws_fixture" cfg_attr_cfg_comment_gap_test \
	    || has_runnable_test_in_file "$ws_fixture" cfg_disabled_test \
	    || has_runnable_test_in_file "$ws_fixture" cfg_attr_cfg_disabled_test \
	    || has_runnable_test_in_file "$ws_fixture" multiline_cfg_ignore_test \
	    || has_runnable_test_in_file "$ws_fixture" multiline_cfg_panic_expected_test \
	    || has_runnable_test_in_file "$ws_fixture" parent_cfg_disabled_test \
	    || has_runnable_test_in_file "$ws_fixture" parent_cfg_attr_disabled_test \
	    || has_runnable_test_in_file "$ws_fixture" parent_cfg_comment_gap_test \
	    || has_runnable_test_in_file "$ws_fixture" parent_cfg_attr_ignore_test \
	    || has_runnable_test_in_file "$ws_fixture" block_commented_test \
	    || has_runnable_test_in_file "$ws_fixture" nested_block_commented_test; then
	    fail "self-test accepted ignored or panic-expected WS evidence test"
	  fi

  ws_test_includes_fixture "$ws_fixture" fixture_direct_ok "fixtures/ws/direct.json"
  ws_test_includes_fixture "$ws_fixture" fixture_multiline_ok "fixtures/ws/multiline.json"
  ws_test_includes_fixture "$ws_fixture" fixture_concat_ok "fixtures/ws/concat.json"
  ws_test_includes_fixture "$ws_fixture" fixture_skip_marker_string_ok "fixtures/ws/string-marker-ok.json"
  if ws_test_includes_fixture "$ws_fixture" fixture_direct_ok "fixtures/ws/wrong.json" \
    || ws_test_includes_fixture "$ws_fixture" fixture_indirect_const_fails "fixtures/ws/indirect.json" \
    || ws_test_includes_fixture "$ws_fixture" fixture_helper_fails "fixtures/ws/helper.json" \
    || ws_test_includes_fixture "$ws_fixture" fixture_comment_only_fails "fixtures/ws/comment.json" \
    || ws_test_includes_fixture "$ws_fixture" fixture_string_literal_fails "fixtures/ws/string.json" \
    || ws_test_includes_fixture "$ws_fixture" fixture_wrong_fails "fixtures/ws/expected.json" \
    || ws_test_includes_fixture "$ws_fixture" fixture_return_unit_fails "fixtures/ws/return-unit.json" \
    || ws_test_includes_fixture "$ws_fixture" fixture_return_ok_fails "fixtures/ws/return-ok.json" \
    || ws_test_includes_fixture "$ws_fixture" fixture_load_or_skip_fails "fixtures/ws/load-or-skip.json" \
    || ws_test_includes_fixture "$ws_fixture" fixture_temp_dir_fails "fixtures/ws/temp-dir.json" \
    || ws_test_includes_fixture "$ws_fixture" fixture_disabled_parent_fails "fixtures/ws/disabled.json"; then
    fail "self-test accepted wrong, indirect, skipped, comment-only, string-only, or disabled WS fixture include"
  fi

  cat >"$diag_fixture" <<'EOF'
#[tokio::test]
async fn diagnose_ok_real_response() {
    let _body = include_str!("../fixtures/ok.json");
}

#[tokio::test]
async fn diagnose_multiline_ok_real_response() {
    let _body = include_str!(
        "../fixtures/multiline_ok.json"
    );
}

mod enabled_parent {
    #[tokio::test]
    async fn diagnose_nested_ok_real_response() {
        let _body = include_str!("../fixtures/nested_ok.json");
    }
}

#[tokio::test]
async fn diagnose_bybit_real_response() {
    let _body = include_str!("../fixtures/bybit_bound.json");
}

#[tokio::test]
async fn diagnose_okx_real_response() {
    let _body = include_str!("../fixtures/okx_bound.json");
}

#[tokio::test]
async fn diagnose_comment_only_real_response() {
    // let _body = include_str!("../fixtures/comment_only.json");
}

#[tokio::test]
async fn diagnose_string_literal_only_real_response() {
    let _note = "include_str!(\"../fixtures/string_literal_only.json\")";
}

#[should_panic]
#[tokio::test]
async fn diagnose_panic_real_response() {
    let _body = include_str!("../fixtures/panic.json");
}

#[cfg(any())]
// cfg reason must not hide the disabling attribute
#[tokio::test]
async fn diagnose_cfg_disabled_with_comment_gap_real_response() {
    let _body = include_str!("../fixtures/cfg_disabled_with_comment_gap.json");
}

#[cfg_attr(all(), cfg(any()))]
// cfg_attr reason must not hide the disabling attribute
#[tokio::test]
async fn diagnose_cfg_attr_cfg_comment_gap_real_response() {
    let _body = include_str!("../fixtures/cfg_attr_cfg_comment_gap.json");
}

#[cfg(any())]
#[tokio::test]
async fn diagnose_cfg_disabled_real_response() {
    let _body = include_str!("../fixtures/cfg_disabled.json");
}

#[cfg_attr(
    all(),
    cfg(any())
)]
#[tokio::test]
async fn diagnose_cfg_attr_cfg_disabled_real_response() {
    let _body = include_str!("../fixtures/cfg_attr_cfg_disabled.json");
}

#[cfg_attr(
    any(),
    ignore
)]
#[tokio::test]
async fn diagnose_multiline_ignore_real_response() {
    let _body = include_str!("../fixtures/multiline_ignore.json");
}

#[cfg_attr(
    any(),
    should_panic
)]
#[tokio::test]
async fn diagnose_multiline_panic_real_response() {
    let _body = include_str!("../fixtures/multiline_panic.json");
}

#[cfg(any())]
mod disabled_parent {
    #[tokio::test]
    async fn diagnose_parent_cfg_disabled_real_response() {
        let _body = include_str!("../fixtures/parent_cfg_disabled.json");
    }
}

#[cfg_attr(all(), cfg(any()))]
mod cfg_attr_disabled_parent {
    #[tokio::test]
    async fn diagnose_parent_cfg_attr_disabled_real_response() {
        let _body = include_str!("../fixtures/parent_cfg_attr_disabled.json");
    }
}

#[cfg(any())]
// cfg reason must not hide the disabling parent module
mod disabled_parent_with_comment_gap {
    #[tokio::test]
    async fn diagnose_parent_cfg_comment_gap_real_response() {
        let _body = include_str!("../fixtures/parent_cfg_comment_gap.json");
    }
}

#[cfg_attr(any(), ignore)]
mod ignored_parent {
    #[tokio::test]
    async fn diagnose_parent_cfg_attr_ignore_real_response() {
        let _body = include_str!("../fixtures/parent_cfg_attr_ignore.json");
    }
}

/*
#[tokio::test]
async fn diagnose_block_commented_real_response() {
    let _body = include_str!("../fixtures/block_commented.json");
}
*/

/*
outer
/* nested */
#[tokio::test]
async fn diagnose_nested_block_commented_real_response() {
    let _body = include_str!("../fixtures/nested_block_commented.json");
}
*/

#[tokio::test]
async fn diagnose_brace_comment_leak_real_response() {
    // {
}

async fn helper_sibling_only_fixture() {
    let _body = include_str!("../fixtures/sibling_only.json");
}
EOF

	  diag_test_includes_fixture "$diag_fixture" "../fixtures/ok.json"
	  diag_test_includes_fixture "$diag_fixture" "../fixtures/multiline_ok.json"
	  diag_test_includes_fixture "$diag_fixture" "../fixtures/nested_ok.json"
	  diag_test_includes_fixture "$diag_fixture" "../fixtures/bybit_bound.json" "diagnose_bybit_real_response"
	  if diag_test_includes_fixture "$diag_fixture" "../fixtures/comment_only.json" \
	    || diag_test_includes_fixture "$diag_fixture" "../fixtures/string_literal_only.json" \
    || diag_test_includes_fixture "$diag_fixture" "../fixtures/okx_bound.json" "diagnose_bybit_real_response" \
    || diag_test_includes_fixture "$diag_fixture" "../fixtures/panic.json" \
    || diag_test_includes_fixture "$diag_fixture" "../fixtures/cfg_disabled_with_comment_gap.json" \
    || diag_test_includes_fixture "$diag_fixture" "../fixtures/cfg_attr_cfg_comment_gap.json" \
    || diag_test_includes_fixture "$diag_fixture" "../fixtures/cfg_disabled.json" \
	    || diag_test_includes_fixture "$diag_fixture" "../fixtures/cfg_attr_cfg_disabled.json" \
	    || diag_test_includes_fixture "$diag_fixture" "../fixtures/multiline_ignore.json" \
	    || diag_test_includes_fixture "$diag_fixture" "../fixtures/multiline_panic.json" \
	    || diag_test_includes_fixture "$diag_fixture" "../fixtures/parent_cfg_disabled.json" \
	    || diag_test_includes_fixture "$diag_fixture" "../fixtures/parent_cfg_attr_disabled.json" \
	    || diag_test_includes_fixture "$diag_fixture" "../fixtures/parent_cfg_comment_gap.json" \
	    || diag_test_includes_fixture "$diag_fixture" "../fixtures/parent_cfg_attr_ignore.json" \
	    || diag_test_includes_fixture "$diag_fixture" "../fixtures/block_commented.json" \
	    || diag_test_includes_fixture "$diag_fixture" "../fixtures/nested_block_commented.json" \
	    || diag_test_includes_fixture "$diag_fixture" "../fixtures/sibling_only.json"; then
	    fail "self-test accepted comment-only, panic-expected, or cfg-disabled diagnostic fixture include"
	  fi
	  if ! diag_test_fixture_includes "$diag_fixture" | grep -Fxq "../fixtures/ok.json" \
	    || ! diag_test_fixture_includes "$diag_fixture" | grep -Fxq "../fixtures/multiline_ok.json" \
	    || ! diag_test_fixture_includes "$diag_fixture" | grep -Fxq "../fixtures/nested_ok.json" \
	    || diag_test_fixture_includes "$diag_fixture" | grep -Fxq "../fixtures/string_literal_only.json" \
	    || diag_test_fixture_includes "$diag_fixture" | grep -Fxq "../fixtures/parent_cfg_disabled.json"; then
	    fail "self-test diagnostic fixture include scanner lost real macros or accepted string literals"
	  fi

  printf 'OK exchange operation evidence matrix self-test\n'
}

if [ "${1:-}" = "--self-test" ]; then
  run_self_test
  exit 0
fi

[ -s "$ALLOWLIST" ] || fail "missing ${ALLOWLIST#$ROOT/}"
[ -s "$MATRIX" ] || fail "missing ${MATRIX#$ROOT/}"
validate_matrix
require_unique_matrix_fixture_columns

tail -n +2 "$MATRIX" | while IFS=$'\t' read -r venue rest_trade rest_order rest_balance rest_position _ws_write _ws_private _close_boundary diag_boundary diag_fixture_ids diag_fixture_hashes _ws_fixture_ids _ws_fixture_hashes _finality; do
  [ "$rest_trade" != "recorded" ] || require_rest_bucket "$venue" "TradeWrite" "OrderAck"
  [ "$rest_order" != "recorded" ] || require_rest_bucket "$venue" "PrivateRead" "OrderStatus"
  [ "$rest_balance" != "recorded" ] || require_rest_bucket "$venue" "PrivateRead" "AccountBalance"
  [ "$rest_position" != "recorded" ] || require_rest_bucket "$venue" "PrivateRead" "AccountPosition"
  require_diag_fixtures "$venue" "$diag_boundary" "$diag_fixture_ids" "$diag_fixture_hashes"
done
require_diag_include_closure

require_test_fn "trading_ws_matrix_covers_exactly_enabled_venues"
require_test_fn "schema_pending_write_ops_never_enter_live_writer"
require_test_fn "kucoin_write_path_is_not_live_submittable"
require_test_fn "live_submittable_write_paths_must_carry_operation_evidence"
require_test_fn "close_position_without_operation_evidence_stays_display_only"
require_test_fn "close_position_never_enters_operation_registry_without_dedicated_operation_evidence"
require_test_fn "operation_matrix_boundaries_match_ws_registry_projection"
require_test_fn "operation_matrix_ws_fixtures_match_runtime_registry_evidence"
require_test_fn "ready_private_ws_streams_have_operation_evidence"
require_test_fn "private_ws_evidence_points_to_existing_tests"
require_test_fn "binance_live_ws_write_ops_carry_official_evidence"
require_test_fn "okx_live_ws_write_ops_carry_official_evidence"
require_test_fn "gate_live_ws_write_ops_carry_official_evidence"
require_test_fn "hyperliquid_live_ws_write_ops_carry_official_evidence"
require_test_fn "bybit_live_ws_write_ops_carry_official_evidence"
require_test_fn "bitget_live_ws_write_ops_carry_official_evidence"
require_test_fn "kucoin_beta_write_ops_remain_schema_only"
require_test_fn "kucoin_classic_fill_stream_records_identity_evidence_without_invented_fee"
require_test_fn "evidence_fixture_lookup_requires_direct_include_macro"
require_registry_test_fn "ws_operation_registry_keeps_close_position_display_only"
require_registry_test_fn "ws_operation_registry_keeps_ack_rows_out_of_finality_evidence"
require_registry_test_fn "ws_operation_registry_projects_fixture_metadata_by_scope"
require_registry_test_fn "ws_operation_registry_uses_typed_evidence_scope_boundaries"
require_registry_test_fn "ws_operation_registry_keeps_kucoin_schema_pending_writes_non_ready"
require_rest_registry_test_fn "operation_matrix_rest_buckets_match_runtime_registry_projection"
require_rest_registry_test_fn "operation_matrix_rest_buckets_match_exact_allowlist_endpoint_projection"
require_source_literals \
  "$REST_REGISTRY" \
  "exchange_operation_evidence_matrix.tsv" \
  "exchange_evidence_debt_allowlist.tsv" \
  "rest_trade_write_order_ack" \
  "rest_private_order_status" \
  "rest_private_account_balance" \
  "rest_private_account_position"
require_api_registry_test_fn "transport_registry_route_unifies_rest_and_ws_evidence"
require_api_registry_test_fn "transport_registry_route_summary_matches_operation_matrix_tsv"
require_api_registry_test_fn "transport_registry_route_preserves_rest_operation_matrix_buckets"
require_api_registry_test_fn "transport_registry_route_keeps_ack_only_boundaries"
require_source_literals \
  "$API_REGISTRY_TEST" \
  "exchange_operation_evidence_matrix.tsv" \
  "rest_trade_write_order_ack" \
  "rest_private_order_status" \
  "rest_private_account_balance" \
  "rest_private_account_position"
require_ws_parser_fixtures
if awk '
  /^[[:space:]]*(\/\/|\/\/!|\/\*)/ { next }
  /#\[(ignore|should_panic)|cfg_attr[^\n]*(ignore|should_panic)|std::env::temp_dir|tempfile|load_or_skip|\/tmp\/|return[[:space:]]+Ok[[:space:]]*\(\(\)\)|return[[:space:]]*;|Ok[[:space:]]*\(\(\)\)/ {
    print FILENAME ":" FNR ":" $0
    found = 1
  }
  END { exit found ? 0 : 1 }
' "$DIAG_TEST"; then
  fail "diag_real_responses.rs must use committed fixtures and must not skip or ignore tests"
fi

private_trade_rows="$(
  awk -F '\t' 'NR > 1 && ($4 == "PrivateRead" || $4 == "TradeWrite") { count++ } END { print count + 0 }' "$ALLOWLIST"
)"

printf 'OK exchange operation evidence matrix gate (9 venues, %s private/trade REST rows, WS evidence tests locked)\n' "$private_trade_rows"
