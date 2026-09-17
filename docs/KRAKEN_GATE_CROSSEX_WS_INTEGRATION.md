# Kraken And Gate CrossEx WebSocket Integration

Updated: **2026-08-06**

Status summary: **public WS runtime verified; protocol and fixture coverage complete;
credentialed private runtime evidence pending operator configuration**.

This document is the focused operator and engineering reference for CROSSLINE Omni's
Kraken Spot, Kraken Derivatives and Gate CrossEx integration. General transport rules
remain authoritative in [EXCHANGE_TRANSPORT_MATRIX.md](./EXCHANGE_TRANSPORT_MATRIX.md),
while product semantics and code boundaries remain authoritative in
[CROSSLINE_IMPLEMENTATION_GUIDE.md](./CROSSLINE_IMPLEMENTATION_GUIDE.md).

## Scope

| Integration | Public market data | Private/account data | Trading transport |
|---|---|---|---|
| Kraken Spot | WebSocket v2 `instrument`, `ticker`, checksummed `book` | v2 `balances` and `executions` with a REST-issued WS token | v2 `add_order` and `cancel_order` |
| Kraken Derivatives | Futures `ticker` and sequenced `book` | authenticated `balances`, `open_positions`, `open_orders` and `fills` | signed Futures REST v3 because the official derivatives socket is streaming-only |
| Gate CrossEx | dedicated CrossEx `ticker`, `mark_price`, `funding_rate` and `order_book_update` | dedicated private `asset`, `position`, `order` and `usertrades` | dedicated private WS `place_order` and `cancel_order` |

Ordinary Gate Spot or Futures channels are not accepted as Gate CrossEx evidence.
CrossEx route identity always retains its underlying exchange, business type and
native symbol.

## Credentials

Credentials may be configured from **Settings -> Credentials** or injected through
the process environment.

| Product | Required variables |
|---|---|
| Kraken Spot | `KRAKEN_SPOT_API_KEY`, `KRAKEN_SPOT_API_SECRET` |
| Kraken Futures | `KRAKEN_FUTURES_API_KEY`, `KRAKEN_FUTURES_API_SECRET` |
| Gate CrossEx | `GATE_CROSSEX_API_KEY`, `GATE_CROSSEX_API_SECRET` |

Kraken Spot and Futures use independent credential pairs and signing schemes. A
complete pair enables only its own product. The UI never combines a Spot key with a
Futures secret or treats one product's successful probe as proof for the other.

Credentials are not required for public ticker, listing or order-book streams. They
are required before private account/order streams, signed reads, permission probes or
Live writes can become ready.

## Public WebSocket Behavior

### Kraken Spot

- Uses the official WebSocket v2 endpoint.
- `instrument` is the live listing and precision authority; REST `AssetPairs` remains
  a bounded recovery source.
- `ticker` uses explicit product IDs and BBO-triggered updates.
- `book` applies the official snapshot before deltas, validates checksum state and
  resubscribes after a gap.
- Native `XBT` is normalized to canonical `BTC` only through Kraken symbol and
  instrument rules.

### Kraken Derivatives

- Uses the official Futures WebSocket `ticker` and `book` feeds.
- The full ticker supplies BBO, mark, index, current Funding and next settlement.
- The book state rejects an unsafe delta chain and requests a fresh snapshot after a
  sequence gap.
- Subscriptions are deduplicated and split into bounded batches. Reconnect restores
  only active subscription keys.

The initial WS-only discovery probe is limited to BTC, ETH and SOL. The global
Funding scan is different: it keeps one deduplicated full `ticker` subscription over
execution-ready Kraken perpetuals because `ticker_lite` omits Funding, mark and index.
This is a substantial byte stream at Kraken's one-second cadence, but it does not
subscribe order books.

Local evidence captured on 2026-08-06:

| Sample | Received traffic over 10 seconds |
|---|---:|
| Three Kraken Futures products | about 42.7 KB |
| 274 execution-ready perpetuals on one Futures WS | about 3.5 MB |

The linear increase matches the venue's per-product full ticker payload. The runtime
held one Kraken Spot WS and one Kraken Futures WS; the sample did not indicate a
duplicate-connection or reconnect leak.

### Gate CrossEx

- Uses only the dedicated CrossEx public endpoint.
- Every subscription contains explicit native route symbols such as
  `GATE_FUTURE_BTC_USDT`; an empty/all-products subscription is never emitted.
- The first-frame probe uses `gate:BTC`, `gate:ETH` and `gate:SOL`, preserving the
  underlying route identity.
- Books are candidate/preflight-only and expire when no longer touched.
- Public frames can provide the current Funding rate and next settlement time, while
  the native interval comes from authenticated
  `GET /api/v4/crossex/market/funding_info`.

Without Gate CrossEx credentials, Funding health therefore reports the missing
`GATE_CROSSEX_API_KEY` and `GATE_CROSSEX_API_SECRET`. It does not invent an eight-hour
interval or mislabel the condition as a public WS timeout.

## Private Streams And Finality

Private readiness has three different states:

1. Credentials are present and the authenticated session can be opened.
2. Required account/order subscriptions are acknowledged.
3. A real account or order event has been observed and remains fresh.

Only the third state is runtime evidence. A successful subscription ACK, parser
fixture or read-only credential probe does not prove Live write permission or order
finality.

Kraken Spot terminal state comes from `executions`; account updates come from
`balances`. Kraken Futures uses `open_orders` and `fills` for order finality and
`balances` plus `open_positions` for account state. Gate CrossEx uses `order` and
`usertrades` for terminal evidence and `asset` plus `position` for account state.

Cold snapshots, pagination, history and identity-bound reconciliation remain bounded
REST operations where the official WS does not provide a complete snapshot or point
query. A failed non-idempotent request is never replayed over another transport before
the same client order identity is reconciled.

## Instrument And Execution Safety

- Kraken Spot v2 `instrument` and Kraken Futures REST instruments produce the native
  symbol, product, quote, contract multiplier, price precision and quantity precision.
- Gate CrossEx `/crossex/rule/symbols` supplies route, business type, execution rules
  and supported underlying exchange.
- A ticker without a verified InstrumentSpec cannot become a Live leg.
- Gate CrossEx cannot be paired against the same direct underlying venue as an
  independent arbitrage source.
- Unknown quantity units, Funding intervals, account modes or permission states remain
  blocked rather than defaulting to zero or an assumed value.

## Runtime Health

The following public operations were verified locally with live upstream data:

| Venue | Operation | Expected runtime state |
|---|---|---|
| Kraken | `perp_tickers` | `ok`, fresh WS sample |
| Kraken | `spot_ticks` | `ok`, fresh WS sample |
| Kraken | `funding_rates` | `ok`, fresh WS sample |
| Kraken | `ws_ticker_snapshot` | `ok` |
| Kraken | `ws_spot_snapshot` | `ok` |
| Kraken | `ws_funding_snapshot` | `ok` |
| Gate CrossEx | `perp_tickers` | `ok`, fresh WS sample |
| Gate CrossEx | `spot_ticks` | `ok`, fresh WS sample |
| Gate CrossEx | `funding_rates` | blocked until signed interval metadata is available |

Private operations remain `blocked` with explicit credential instructions until the
operator configures the relevant product pair. They must not be changed to `ok` based
only on fixture tests.

Useful local endpoints:

```bash
curl -fsS 'http://127.0.0.1:8000/api/system/venue-operation-health?venue=kraken' | jq
curl -fsS 'http://127.0.0.1:8000/api/system/venue-operation-health?venue=gate_crossex' | jq
curl -fsS http://127.0.0.1:8000/api/trading/transport/registry | jq
```

## Verification Record

Completed local checks for this integration:

- Kraken and Gate CrossEx parser/request tests use recorded, desensitized fixtures and
  do not skip.
- `cargo clippy -p exchange --all-targets --no-deps -- -D warnings` passed.
- `cargo clippy -p api --bin crypto-arb-api --no-deps -- -D warnings` passed.
- Settings and Futures routes were checked at 1536, 1100, 720 and 480 pixels without
  document-level horizontal overflow, new console errors or Wasm panic.
- Public runtime health was inspected after a clean backend restart.
- Connection count and ten-second traffic were sampled after the streams reached their
  steady state.

The operation-evidence gate additionally requires fixtures to be tracked by Git. It
will remain red in an uncommitted checkout even when the files exist; rerun it after
staging or committing the integration:

```bash
bash scripts/check_exchange_operation_evidence_matrix.sh
```

Configured private WS samples and real order finality are external acceptance steps.
They require operator-owned credentials and must use the small-fund Live runbook; this
document does not authorize an unattended real order.

## Official References

- [Kraken Spot WebSocket v2](https://docs.kraken.com/exchange/api-reference/spot-websocket-v2)
- [Kraken Spot instrument](https://docs.kraken.com/exchange/api-reference/spot-websocket-v2/instrument)
- [Kraken Spot book](https://docs.kraken.com/exchange/api-reference/spot-websocket-v2/book)
- [Kraken Spot add order](https://docs.kraken.com/exchange/api-reference/spot-websocket-v2/add_order)
- [Kraken Futures WebSocket](https://docs.kraken.com/exchange/api-reference/futures-websocket)
- [Kraken Futures ticker](https://docs.kraken.com/exchange/api-reference/futures-websocket/ticker)
- [Kraken Futures ticker lite](https://docs.kraken.com/exchange/api-reference/futures-websocket/ticker_lite)
- [Kraken Futures REST](https://docs.kraken.com/exchange/api-reference/futures-rest-api)
- [Gate CrossEx WebSocket](https://www.gate.com/docs/developers/crossex/ws/en/)
- [Gate CrossEx REST](https://www.gate.com/docs/developers/crossex/en/)
