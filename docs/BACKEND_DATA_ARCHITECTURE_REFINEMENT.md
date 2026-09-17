# CROSSLINE Realtime Product Flow Refinement

> Updated: 2026-08-09.
> This is an evidence and decision ledger, not an implementation checklist or architecture
> authority. Product behavior and measured runtime facts drive the work; this file records the
> result after each complete user flow is improved.

## Product Direction

CROSSLINE should feel direct in the same way Hyperliquid feels direct: one live truth, few states,
short paths, immediate actions and dense but understandable feedback. A user-visible number should
not be independently reconstructed by the API, automation worker and frontend. They should all read
the same current fact and preserve its source and freshness.

Large internal changes are valid when they remove latency, duplicated work or ambiguous product
behavior. Compatibility layers that only protect an obsolete path should not determine the new
shape.

## Working Principles

1. **One market truth**: one shared owner for public WS ticker, funding and selected-book state.
2. **One account truth**: private WS owns live order, fill, balance and position changes; REST is a
   bounded seed and repair path, never a second competing runtime.
3. **Discovery stays cheap**: all-market opportunity discovery uses ticker and funding snapshots.
   Bilateral depth belongs to candidate preflight and the final order decision.
4. **Profit is a fact, not a score**: every executable row must state the native settlement events,
   entry/exit price exposure, fees, slippage, borrow/inventory constraints and failure risk that make
   the trade profitable or block it.
5. **One action state machine**: build, submit, acknowledge, fill, compensate and exit use one run ID
   and one terminal result from backend to UI.
6. **Push state, do not re-poll it**: AppWS publishes compact changes from the shared truth. Polling is
   only bounded recovery when a stream is proven stale.
7. **Performance and efficiency move together**: fewer duplicate subscriptions, requests, clones,
   serializations and writes without slowing effective market cadence.
8. **Measure the user path**: latency, CPU, RSS, traffic, connection count and recovery are measured
   from exchange event to visible product state, not only inside one function.

## End-To-End Order

Only one complete flow is changed at a time:

| Flow | User result |
|---|---|
| F1 Public market -> five strategies -> futures UI | Fast, stable opportunity rows from one live market snapshot |
| F2 Opportunity -> preflight -> two-leg execution | A candidate becomes executable only after fresh depth, cost and risk proof |
| F3 Private streams -> orders/positions/balances -> UI | Order and account changes appear once, quickly, with one terminal truth |
| F4 Position -> risk/exit -> close | Profit target, stop, liquidation risk and paired close share one exit state machine |
| F5 Automation/webhook -> execution -> review | Automation uses the same candidates and executor as manual operation |
| F6 Runtime convergence | API/AppWS, cache, storage, lifecycle and observability have clear ownership |

## F1 Current Flow

```text
exchange public WS
        |
        v
shared ticker + funding snapshot
        |
        v
five strategy calculations
        |
        v
profit/risk projection -> AppWS delta -> futures UI
        |
        +---- selected candidate ----> bilateral depth preflight -> hedge build/submit
```

The opportunity list must not request every candidate's order book. Depth changes too quickly to be
meaningful after a long global fanout and is only actionable for the candidate being prepared for
execution.

### Baseline Before F1 Change

Local debug runtime after streams reached steady activity:

| Measure | Baseline |
|---|---:|
| Lifecycle tasks | 21 / 21 healthy |
| Opportunity scan | 30,223-31,012 ms |
| Intended scan cadence | 5,000 ms |
| Candidate output | 1,172-1,523 rows |
| Funding aggregation | 5,364 ms / 4,346 rows |
| Process CPU sample | 102.4% |
| Resident memory | 353,328 KiB |
| Exchange-side TCP connections | 56 |
| Browser/AppWS TCP connections | 7 |
| Startup cache fresh-hit ratio | 1.39% |
| Order-book cache misses | 280: 26 perp + 254 spot |

The scan path ranked up to 64 candidates for each of five strategies, generated as many as 640 leg
requests per round and could run two rounds. That made discovery wait for execution-only data.

### F1 Decisions

- [x] Remove scan-time global order-book fanout and its ranking/retry machinery.
- [x] Keep list depth absent instead of pretending old depth is current.
- [x] Preserve spot/perp instrument identity for fee and execution planning.
- [x] Preserve strict bilateral depth in selected candidate, automation preflight and order submit.
- [x] Use one WS ingest path for the 100ms live projection and 15s subscription prewarm.
- [x] Do not issue per-symbol REST requests when WS is pending, partial or unsupported; keep those
      states explicit and reserve REST for cold-start and rotating discovery snapshots.
- [x] Use Binance perpetual `<symbol>@ticker` as the complete bounded-plan ticker baseline and overlay only
      bounded-plan `<symbol>@bookTicker` BBO updates; remove both the all-market book stream and the
      10-second full-market REST statistics refresh. This
      follows Binance's current [public/market URL split and stream mapping](https://developers.binance.com/en/docs/products/derivatives-trading-usds-futures/websocket-market-streams/Important-WebSocket-Change-Notice).
- [x] Collapse bounded Spot discovery to one official WS row per planned symbol: Binance
      `<symbol>@ticker`, Gate `spot.tickers`, and KuCoin `/market/snapshot:{symbol}`. Remove Binance
      and Gate's duplicate scanner BBO subscriptions and KuCoin's periodic all-market REST volume
      join; keep bilateral order books cold until preview/build.
- [x] Keep transfer-network metadata cold: only a positive `SpotCross`, `SpotPerp` or
      `CrossSpotPerp` candidate whose listing, identity, live legs and cost gates already pass may
      request the involved venue families.
- [x] Build the immutable instrument lookup only when the official registry changes. Opportunity
      scans now load one `ArcSwap` snapshot instead of cloning roughly 25,000 instruments in both
      the execution gate and transfer gate.
- [x] Key transfer networks directly by venue family and currency. A profitable candidate performs
      direct route lookup; a non-candidate does not load the transfer snapshot at all.
- [x] Re-measure the same warm runtime window.
- [x] Trace remaining scan CPU and allocations after network waits are gone.
- [x] Build symbol, venue, exact-contract funding and quote-conversion indexes once per market
      snapshot and reuse them across all five strategies. `CrossSpotPerp` ranks lightweight
      fee-adjusted candidates first and allocates full evidence objects only for its retained
      profitable frontier.
- [x] Confirm all five strategy tabs consume the same opportunity index and stable row identity.
- [x] Complete one affected-route browser pass with no console errors.
- [x] Close the remaining venue WS first-frame gaps, then run one change-aware finish for F1.

### Warm Runtime After F1 Data-Path Change

Local debug runtime after the replacement backend reached steady activity:

| Measure | Result |
|---|---:|
| Lifecycle tasks | 21 / 21 healthy |
| Opportunity scan | 32-75 ms |
| Candidate output | 680-689 rows |
| Process CPU sample | 13.3% |
| Resident memory | 182,416 KiB |
| External established TCP connections | 48 |
| One-second process traffic sample | 100,138 B received / 27,286 B sent |
| Hidden per-symbol REST fallback operations | 0 |
| Binance planned perp WS coverage across rotating warm samples | 2-4 / 2-4 fresh |
| Binance dedicated 10-second full-market ticker refresh | Removed |
| Five-second process traffic sample after bounded Binance subscription | 3,067,118 B received / 61,454 B sent |
| Five-second process traffic sample after bounded Spot stream collapse | 371,191 B received / 13,695 B sent |
| KuCoin periodic Spot all-market volume refresh after more than 60 seconds | 0; cold-start count stayed at 1 |
| Opportunity-list order-book guards / in-flight reads | 0 / 0 |
| Futures strategy tabs rendered | 5 / 5 |
| Browser console warnings/errors | 0 |
| Final Gate rotating WS sample | ticker 7 / 7 fresh; funding 16 / 16 fresh after bounded warm-up |
| Final lifecycle health | 24 / 24 healthy |

The published warm scan exposes an explicit first-event warming row while a new thin symbol joins
the bounded plan; the row cannot satisfy build or execution evidence and is never REST-masked.
Subscriptions now receive `Connected` only after the shared writer is installed, and failed sends
release their claim for one bounded retry. Gate keeps its SBE ticker/BBO stream, bootstraps only one
WS book level for a new or quiet candidate, refreshes sparse ticker snapshots before cache expiry,
and rate-limits full public-stream reconnect recovery. In the final rotating sample, transient Gate
warming recovered to ticker 7/7 and funding 16/16 without a missing state. A single KuCoin
`/api/v3/currencies` read occurred only after a positive SpotCross candidate entered transfer-loop
validation, while list discovery still produced no order-book read.

### Immutable Registry Projection Measurement

The focused 2026-08-08 profile loaded 25,445 official instruments and more than 11,000 transfer
network rows. Before the change, each opportunity cycle rebuilt two instrument maps and route
checks scanned the complete transfer map. After the change, those clone/scan stacks disappeared
from a five-second process sample. The full-market engine remained about 290-318 ms, while the
effective event-to-event opportunity refresh shortened from roughly 3.8-4.2 seconds to 1.18-1.32
seconds because post-engine registry work no longer inflated the adaptive duty window.

This is a latency win, not yet a complete heat result. Debug-runtime CPU still varied between 34%
and 72% while the faster event loop was active. Transfer metadata itself stayed cold until a fresh
WS candidate with verified bilateral fees and positive net edge appeared.

### Shared Strategy Index And Candidate Allocation Benchmark

A fixed Criterion workload of 256 symbols across eight venue families isolates the strategy CPU
path from exchange and AppWS variance. Before the change, each strategy rebuilt overlapping symbol,
venue, exact-contract and quote-conversion maps. `CrossSpotPerp` also allocated complete evidence
objects for every qualifying venue pair before discarding most of them.

The engine now prepares one borrowed market index per snapshot. `CrossSpotPerp` first calculates
native-event gross yield and official standard-fee net yield using references and scalar values,
then constructs descriptions, cloned funding rows and conversion evidence only for the retained
frontier. This is actual net-yield ordering, not a product score; deposit/withdraw evidence remains
a candidate-gated execution/Webhook check after market and cost proof.

| Criterion measure | Before | After | Change |
|---|---:|---:|---:|
| Raw `CrossSpotPerp` median | 51.45 ms | 23.67 ms | -54.0% |
| Raw `CrossSpotPerp` output objects | 12,544 | 4,096 | -67.4% |
| Full five-strategy median | 223.28 ms | 181.35 ms | -18.8% |
| Full five-strategy output objects | 8,353 | 5,376 | -35.6% |

All 208 arbitrage unit tests and all-target Clippy pass after the change. These are deterministic
CPU-path measurements, not replacements for the next warm-runtime CPU, traffic and first-frame
recovery sample.

## F6 Runtime Convergence

Current materialized product state now has an explicit owner and read-only delivery paths:

```text
exchange WS / bounded lifecycle discovery
                    |
                    v
           canonical runtime cache
                    |
          +---------+---------+
          |                   |
          v                   v
     lifecycle publish    REST / AppWS replay
          |                   |
          v                   v
   current product envelope   pure read only
```

Funding rows live only in `MarketDataCache`; public WS and the bounded funding lifecycle are its
writers. System, Portfolio and Review materialized envelopes are written only by their owning
lifecycle. REST and AppWS no longer compute a replacement snapshot or mutate a cache while serving
a read. A cold subscriber waits for the first lifecycle publication; Review REST returns typed
warming evidence so the UI can distinguish startup from a valid empty history.

### F6 Structural Result

| Duplicate or ambiguous path | Before | After |
|---|---:|---:|
| Current Funding snapshot stores | 2 | 1 |
| Request/replay/Portfolio Funding fallback write-backs | 3 | 0 |
| Cold AppWS SystemHealth recomputations | 1 | 0 |
| Cold AppWS Portfolio envelope fabrication paths | 1 | 0 |
| Default Review REST ledger materializers/cache writers | 1 | 0 |

The focused proof covers 20 behaviors: 14 AppWS replay cases, two Funding REST cache cases, three
Funding lifecycle cases and one Review warming case. They verify that cold reads do not populate
the lifecycle caches and that fresh WS Funding remains authoritative over bounded REST discovery.
No exchange process was started for this structural batch, so it claims no new CPU, RSS, traffic or
connection measurement; the existing F1 warm-runtime measurements remain the latest live sample.

## F7 On-chain / CEX Execution Closure

```text
fresh DEX firm build + fresh CEX Spot depth
                    |
                    v
inventory / gas / allowance / instrument / profit recheck
                    |
                    v
one-time build claim -> server-side exact signing
                    |
                    v
CEX IOC full fill -> chain broadcast -> terminal follow-up
          |                    |
          | partial/fail       | definite rejection
          v                    v
 cancel + reverse       reverse CEX compensation

ambiguous chain transport -> keep CEX hedge -> bounded same-transaction follow-up
```

- [x] The browser submits only an immutable one-time `buildId`; it cannot replace signed payload,
      calldata, CEX symbol, quantity or direction.
- [x] Solana and EVM private keys are address-checked and read from the backend credential store.
- [x] Jupiter, 0x and OKX DEX use firm build endpoints; CoW Fast Quote remains non-executable.
- [x] CEX full fill, chain finality, compensation and unresolved exposure share one backend run ID.
- [x] Ambiguous broadcast/finality never triggers an early reverse that could create double exposure.
- [x] Recent runs survive page refresh in the active backend process; only a pending run enables the
      frontend's two-second recovery read.

Focused proof: the API reconciliation-window test and the frontend provider-route test both pass.
No exchange order, wallet signature or chain broadcast was sent during verification.

## Change Record

| Date | Flow | Product change | Evidence |
|---|---|---|---|
| 2026-08-07 | F1 | Discovery no longer fetches global order books; depth moves to selected-candidate preflight | Focused engine test proves a spot-perp scan leaves both depth legs absent even when the source can provide them |
| 2026-08-07 | F1 | Subscription prewarm and live projection now share one WS ingest path; implicit ticker/funding/spot REST fallback was removed | 15 lifecycle tests pass; runtime exposes missing WS first events directly and emits no fallback operation samples |
| 2026-08-07 | F1 | Binance per-symbol ticker WS now owns the full baseline; only the bounded plan receives real-time book-ticker overlays, and the periodic full-market REST statistics refresh is gone | 14 focused tests and exchange Clippy pass; rotating plans remain 2-4/2-4 fresh, bookTicker REST stays 2 -> 2, and the sole 24hr REST increment in a 20-second sample is the retained rotating discovery baseline |
| 2026-08-07 | F1 | Binance, Gate and KuCoin Spot discovery now consume one official bounded WS row per symbol; duplicate BBO streams and KuCoin's periodic full-market REST volume join are removed | 53 focused Spot-WS tests and exchange Clippy pass; 5-second receive sample fell from 3,067,118 B to 371,191 B, KuCoin all-market Spot REST remained at its single cold-start request beyond 60 seconds, and order-book guards stayed 0 |
| 2026-08-08 | F1 | Instrument and transfer gates now read immutable direct-lookup snapshots instead of rebuilding or scanning registries on every opportunity cycle | 26 focused registry tests and API Clippy pass; 25,445-row clone stacks disappeared from the runtime sample and effective refresh shortened to 1.18-1.32 seconds |
| 2026-08-08 | F2/F5 | SpotCross, SpotPerp and CrossSpotPerp request official deposit/withdraw evidence only after fresh WS prices, verified bilateral fees and positive net edge; an execution Webhook carries the bound transfer result | Focused transfer, ticket and Webhook tests pass; cross-venue routes require a bilateral loop, while same-venue SpotPerp requires fresh Base and Quote networks with both deposit and withdrawal enabled |
| 2026-08-08 | F1 | Five strategies reuse one borrowed market index; CrossSpotPerp separates lightweight profit ranking from full evidence allocation | Fixed 256-symbol/eight-venue benchmark: CrossSpotPerp 51.45 -> 23.67 ms and full engine 223.28 -> 181.35 ms; 208 tests and all-target Clippy pass |
| 2026-08-09 | F1 | Public WS subscriptions cannot race the connection writer; Gate thin-symbol ticker/BBO snapshots recover through bounded WS-only refresh without scanner depth or REST masking | Manager ordering regression, Gate live SBE fixtures and 14 focused Gate tests pass; 80-second rotating runtime reached ticker 7/7 and funding 16/16 fresh with 24/24 lifecycle tasks healthy |
| 2026-08-09 | F6 | Funding, System, Portfolio and Review delivery paths now preserve lifecycle/cache ownership; REST and AppWS replay are pure readers | 20 focused behavior tests prove canonical Funding precedence, no cold-read mutation, no replay fabrication and typed Review warming; no live exchange runtime was started for this structural batch |
| 2026-08-11 | F7 | Selected on-chain/CEX opportunities now build, sign, submit, reconcile and compensate through one backend-owned run | One-time build claim, exact signer tests, bounded finality follow-up and frontend run recovery tests pass; verification sent no real order or chain transaction |
| 2026-08-13 | F7 | Kraken Spot instrument WS now replaces on `snapshot` and merges by native symbol on `update`, so the on-chain CEX selector and execution gate keep the complete official market universe | Official Kraken contract distinguishes snapshot/update; focused parser/merge tests pass; live registry recovered from 3 to 1,429 Spot rows and stayed at 1,429 after 12s, while PUPS/USD and PUPS/EUR both returned fresh `ws_push` evidence |
