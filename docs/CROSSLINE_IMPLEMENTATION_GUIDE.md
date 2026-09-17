# CROSSLINE Implementation Guide

> Status: current unified implementation and product authority.
> Updated: 2026-08-09.
> Scope: code shape, product semantics, implementation ordering, verification evidence.

This file consolidates the former product vision, product refinement and P0 task index. The
completed whole-repository audit has been removed from the active checkout and remains available
only through Git history; it no longer participates in development scope or verification.

## 0. Authority

Read order:

1. `AGENTS.md`
2. `docs/CROSSLINE_IMPLEMENTATION_GUIDE.md`
3. `DESIGN.md` for frontend/product UI work
4. `README.md` for operator-facing behavior
5. The file being edited

Conflict order:

1. `AGENTS.md` and this guide decide code shape, module boundaries, DTO ownership, and verification gates.
2. `DESIGN.md` decides visual intent, information hierarchy, interaction behavior, responsive behavior, and visual-review workflow; it does not override product semantics.
3. `frontend/styles/src/tokens.css` is the executable token-value source and must be updated with `DESIGN.md` when their contract changes.
4. `README.md` describes the product and transport boundary but does not override code contracts.
5. The newest explicit user request defines the current delivery scope.

## 1. Code Shape

- Backend HTTP routers stay thin; orchestration belongs in `crates/api/src/services/`.
- Shared DTOs live in `shared-types/src/`; frontend files may re-export or type-alias shared DTOs but must not redefine backend contracts.
- Domain crate roots and frontend module `mod.rs` files stay export-only.
- Read-mostly runtime snapshots use `ArcSwap`; concurrent maps use `DashMap`.
- Frontend modules route API work through `data.rs` hooks and expose `LoadState` / `ActionState<ApiProblem>` instead of erasing errors.
- External API behavior must come from official docs, inspected live API responses, or existing repo specs/tests with evidence.

## 2. Product Boundary

CROSSLINE Omni is a private, Chinese-first, low-latency arbitrage workstation for one expert operator. Its primary modules are 持仓/风控、期货套利、机会扫描、链上套利、自动化、对冲执行、复盘和设置. On-chain monitoring and automation own dedicated first-level routes and data lifecycles; they must not be embedded into opportunity scanning or hedge execution. Interfaces should be dense, operational and honest about missing evidence.

P0 does not include an LLM trading assistant, options UI, a standalone RWA basket, on-chain transaction execution, fabricated reference legs, or global live-readiness checks unrelated to the current ticket.

P0 user-facing arbitrage modules expose only:

- `PerpCross`
- `PerpPriceSpread`
- `SpotPerp`
- `CrossSpotPerp`
- `SpotCross`

The product list is a realtime view, not the discovery cache. Every displayed row requires fresh
WS evidence for both main legs. If profitability uses a USDT/USDC conversion, both executable
  conversion directions and their exact native Spot market are carried with the opportunity; the
  candidate planner subscribes that market only after discovery finds the opportunity. REST or stale
  conversion evidence may bootstrap discovery but cannot make the row product-visible, trigger a
  deterministic-profit Webhook, or authorize preview/automation.
- Funding follows the same two-stage boundary. REST may bootstrap discovery and select the exact
  candidate symbols, but every perpetual leg required by a P0 row must then bind exact-row
  `Fresh + WsPush` Funding evidence. Until those candidate subscriptions produce current rows, the
  opportunity remains internal and cannot enter the product list, Webhook or execution workflow.
- The full-universe Funding REST cycle is discovery input only. History publication, REST reads,
  AppWS replay and strategy scans project the same market cache, where fresh candidate WS rows win
  over REST discovery rows. Aggregate cache health describes the rows actually served; optional
  venue failures remain in fanout and row evidence instead of relabeling every fresh row unavailable.

`PerpCross` execution contract:

- `PerpCross` never uses eight-hour-normalized funding for direction, ranking, profitability or
  display. It compares each venue's current native rate for one actual settlement event.
- Native intervals do not need to match. Any verified combination such as 1h/2h/4h/6h/8h can be
  paired when both current `next_funding_time` values point to the same timestamp within one second.
  The intervals are used only to find the next possible common timestamp and detect stale schedule
  evidence.
- If the current next timestamps differ, the pair does not enter the opportunity list. Both venue
  feeds may continue updating until their current next timestamps point to the same event, but
  funding paid by either leg before that timestamp is never accumulated into the opportunity.
- The one joint event must cover complete bilateral open/close fees, measured target-size bilateral
  slippage and the configured target buffer. A losing event cannot become executable by multiplying
  it across later funding periods.
- Contract quote identity must be exact. A base-only funding symbol cannot borrow a ticker when the
  venue exposes multiple quote markets for that base. The ticket compares the long buy VWAP and
  short sell VWAP for its requested notional, not display last prices.
- The two VWAP timestamps may differ by at most two seconds. An absolute executable-price gap up to
  10 bps is eligible for pure funding evaluation only when that gap also consumes no more than half
  of projected post-cost funding profit, regardless of which venue is currently dearer. A wider
  relative gap or an absolute 10-30 bps gap is routed to price-spread observation; above 30 bps is
  a hard data, identity or unit sanity failure.
- Discovery retains the complete bounded 8-by-8 funding frontier in memory until BBO, fresh WS
  evidence and official fee snapshots are attached. Only then does the engine retain at most six
  `PerpCross` pairs per symbol, ordering executable, price-aligned, positive post-cost pairs first.
  This cap never triggers another subscription, REST request or order-book read.
- Funding remains mutable until settlement, so a passing `PerpCross` proof is `projected`, never
  `locked`. Preview and Live preflight must refresh the same native timeline, VWAP, fee and market
  evidence immediately before submission.
- Official venue rules establish each leg's native rate, interval, next timestamp and
  holding-at-settlement eligibility. Requiring the two current next timestamps to align is
  CROSSLINE's conservative cross-venue execution policy, not a venue guarantee.

`PerpCross` official venue contract, rechecked 2026-08-04:

| Venue | Native funding and executable-price authority | Product rule |
|---|---|---|
| Binance | [`fundingInfo`](https://developers.binance.com/docs/derivatives/usds-margined-futures/market-data/rest-api/Get-Funding-Rate-Info) supplies per-symbol interval overrides; [`markPrice`](https://developers.binance.com/docs/derivatives/usds-margined-futures/websocket-market-streams/Mark-Price-Stream) supplies current rate and next settlement; book ticker/depth supplies executable price | Full-universe discovery is USDT. Exact USDC requests remain exact and must never be normalized back to USDT. |
| OKX | [`fundingTime` and `nextFundingTime`](https://www.okx.com/docs-v5/en/) define the native interval; official policy permits 1h, 2h, 4h, 6h and 8h cycles; ticker/books supply bid and ask | Use `*-USDT-SWAP` for P0 discovery. Preserve 6h rather than snapping it to 4h or 8h. |
| Bybit | [`instruments-info`](https://bybit-exchange.github.io/docs/v5/market/instrument) supplies quote, settle and interval minutes; [ticker/WS ticker](https://bybit-exchange.github.io/docs/v5/websocket/public/ticker) supplies interval hours, next settlement and BBO | Full-universe discovery is USDT. Exact `*USDC` and `*PERP` native symbols remain exact. |
| Bitget | [UTA current funding](https://www.bitget.com/api-doc/uta/public/Get-Current-Funding-Rate) supplies 1h/2h/4h/8h interval and `nextUpdate`; [instruments](https://www.bitget.com/api-doc/uta/public/Instruments) supplies base, quote, type and sizing | P0 discovery uses `USDT-FUTURES`; exact alternate-quote tickets require exact registry identity. |
| Gate | [Futures contract metadata](https://www.gate.com/docs/developers/apiv4/en/futures/) supplies interval seconds, next apply time, settle path and contract unit; ticker/order book supplies BBO/depth | P0 discovery uses `/futures/usdt`; never infer interval from a global constant. |
| KuCoin | [Active contracts](https://www.kucoin.com/docs-new/rest/futures-trading/market-data/get-all-symbols) supplies base, quote, settle, funding granularity and next funding timestamp; [tickerV2](https://www.kucoin.com/docs-new/3470080w0) supplies real-time BBO | P0 discovery uses `USDTM`; granularity milliseconds are converted to the native event interval. |
| Hyperliquid | [Funding](https://hyperliquid.gitbook.io/hyperliquid-docs/trading/funding) is paid hourly; [`activeAssetCtx`, `bbo` and `l2Book`](https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/websocket/subscriptions) supply current funding and executable market evidence | Perps settle in USDC. A CEX USDT leg remains observation-only until an explicit quote-risk hedge exists. |
| Kraken | [Derivatives `ticker`](https://docs.kraken.com/exchange/api-reference/futures-websocket/ticker) supplies current funding, next-funding timestamp, BBO, mark and index; the sequenced [`book`](https://docs.kraken.com/exchange/api-reference/futures-websocket/book) supplies execution depth | Preserve the exact official product, quote and contract sizing. Native `XBT` becomes canonical `BTC` only through instrument metadata; it is never inferred from free-form text. |
| Gate CrossEx | Dedicated CrossEx [`ticker`, `funding_rate` and order-book channels](https://www.gate.com/docs/developers/crossex/ws/en/) retain the route's underlying exchange and native product identity | CrossEx is an execution route, not independent liquidity. A route cannot be paired with the same underlying direct venue as an arbitrage leg. |

For a bare canonical symbol, discovery selects the sole official contract or the sole
USDT-quoted-and-settled contract when both USDT and USDC variants exist. An exact native symbol
still selects its exact quote. Missing quote/settle evidence, quote-versus-settle disagreement, or
different quote assets across the two legs is an execution blocker, not a stablecoin-equivalence
assumption.

`SpotPerp` execution contract:

- P0 exposes only the forward cash-and-carry direction: buy Spot and short the same venue's linear
  perpetual. Borrow-and-sell Spot and long-perpetual rows are not generated.
- Spot, perpetual ticker and funding evidence retain venue, canonical base and quote identity.
  USDT and USDC same-quote and cross-quote combinations are both discoverable. A cross-quote row
  must normalize the perpetual BBO through a fresh executable same-venue `USDC/USDT` or inverse
  Spot BBO; it is observation-only until the ticket binds the FX conversion/hedge, conversion cost
  and settlement-asset risk. Exact quote, settle asset and venue-native product identity remain
  mandatory in the instrument-registry and ticket gates.
- The opening basis is a projected one-time exit cash flow, not realized profit and not recurring
  yield. It is counted once. Only the current native funding rate, interval and next-settlement event
  may be added to that projection; `rate_8h` is not used for filtering, ranking or display.
- Missing, expired, or implausibly far native funding schedules produce no row. The reported next
  settlement must fall within the contract's reported native interval from the newest input sample.
  A current rate is never multiplied across unproven future events. Complete Spot/Perp open and close fees, target-size bilateral
  slippage, financing cost and the actual close-side residual still belong to the ticket proof.
- Because a perpetual has no maturity that forces basis convergence, `SpotPerp` remains projected
  and Live-blocked until its basis exit and holding cash flows are ticket-bound.

`CrossSpotPerp` uses the same forward cash-and-carry boundary across two venues:

- Candidate profitability follows the mature executor pattern used by Hummingbot's official
  [Arbitrage Executor](https://hummingbot.org/strategies/v2-strategies/executors/arbitrage-executor/):
  compare the target-side executable buy and sell prices, validate balances and subtract complete
  transaction costs before submission. CROSSLINE's scanner therefore divides the opening basis by
  the actual Spot ask paid, never by a midpoint, and the ticket remains the authority for target-size
  VWAP, fees, slippage, balances and bilateral terminal status.
- Hummingbot's documented
  [Spot-Perpetual Arbitrage](https://hummingbot.org/strategies/v1-strategies/spot-perpetual-arbitrage/)
  separately defines an opening-divergence trigger and a closing-convergence trigger. CROSSLINE
  follows that lifecycle boundary: the scanner may discover the opening pair, but it must not label
  the opening gap as realized profit before a ticket proves the paired close cash flow.
- P0 emits only buy-Spot/short-Perp. It does not emit borrow-and-sell Spot rows. Capital is prefunded
  on both venues, so the scanner does not invent a fixed transfer fee; any rebalance, financing or
  conversion cost must come from ticket-bound evidence. A positive cross-venue candidate does not
  enter the deterministic-opportunity Webhook until the cold transfer registry proves a current Base
  route from the Spot venue to the Perp venue and a Quote return route. Same-venue `SpotPerp` does not
  move assets during opening, but its deterministic artifact still requires fresh official Base and
  Quote networks with both deposit and withdrawal enabled. This is an operational availability gate,
  not an invented execution fee; unavailable routes remain `opportunity_monitor` evidence and never
  become deterministic-opportunity alerts.
- Perpetual ticker and funding rows are matched by exact venue, canonical base and native quote.
  The current native funding event is counted once and `rate_8h` never filters, ranks or values the
  opportunity. A missing, expired, or later-than-its-native-interval next-settlement schedule emits
  no row.
- USDT/USDC cross-quote rows require an executable conversion book on the Perp venue. The converted
  observation remains execution-blocked until the ticket binds the conversion hedge, cost and
  settlement-asset risk. The opening basis is one projected exit cash flow and is never multiplied
  by later holding periods.

Hummingbot's sample
[`v2_funding_rate_arb.py`](https://github.com/hummingbot/hummingbot/blob/master/scripts/v2_funding_rate_arb.py)
is a useful execution reference for volume-aware prices, fees, paired positions and realized
funding-payment tracking, but its fixed connector interval map and 24-hour rate normalization are
not adopted. CROSSLINE uses venue-reported native settlement timestamps and events instead.

Price-spread contracts:

- `PerpPriceSpread` compares executable ask/bid across venues for the same canonical base and
  quote asset, or an executable USDT/USDC conversion book on the short venue. Cross-quote rows stay
  ticket-blocked until the conversion hedge and settlement-asset risk are bound. Its opening gap is
  a convergence hypothesis, not immediately realized profit and not recurring funding yield.
  Both exact-quote perpetual legs must also carry a current native funding rate, interval and next
  settlement timestamp inside that native interval; missing, expired or implausibly far funding evidence produces no list row rather than a
  zero-rate placeholder.
  Following Hummingbot's amount-aware
  [Arbitrage Executor](https://hummingbot.org/strategies/v2-strategies/executors/arbitrage-executor/),
  opening uses the long ask and short bid;
  a historical close uses the long bid and short ask. One base-neutral episode is measured from all
  four executable prices: `(long close bid - long open ask) + (short open bid - short close ask)`.
  Subtracting two independently-normalized spread percentages is not an acceptable PnL shortcut.
  Every history point is timestamped by the newest actual exchange event across both perpetual
  legs and any required FX conversion leg. Re-scanning unchanged cached quotes must not append a
  sample or make convergence evidence mature faster.
  The engine must retain an independent bounded history after a candidate disappears. Funding-rate
  history cannot substitute for this price-convergence history. Multiple time-separated historical
  episodes may qualify a statistical observation only; they never prove this trade's future profit
  and never unlock Live execution. Episode evaluation is time-causal: use the first
  quote that reaches the frozen net target after the minimum hold, otherwise settle the episode at
  the maximum hold. Episodes must not overlap: the next entry starts only after the prior episode's
  exit plus one observation period, so one late reversion cannot be counted as several profitable
  paths. Never select the best future residual, omit a mature losing path, or unlock before the
  evidence window spans enough time to observe those losing paths. The evidence-derived
  gross edge is realized once and must never be multiplied by funding periods when ticket fees are
  refreshed. Both perpetual legs retain their venue-native funding rate, interval and next event;
  `rate_8h` is not used and future funding cash flows are not included in a guaranteed profit claim.
  Raw executable prices continue feeding the bounded history before list evaluation, but a row is
  published only after convergence evidence is ready and its projected net yield is finite and
  strictly positive after the modeled round-trip cost. An unproven history row remains internal
  evidence, not a user-facing opportunity.
  The projected hold plus a 60-second bilateral exit buffer must also finish before either leg's
  next native Funding event. Otherwise the price-spread monitor keeps sampling but withholds the row
  until the next settlement window; the strategy never silently omits an intervening Funding cash
  flow from its profit claim.
  The recommended hold is the empirical convergence time and is always positive. During that hold,
  ordinary
  stop-loss must not mistake the known opening/closing cost envelope for adverse market movement;
  take-profit, liquidation protection and an actual market loss beyond the configured limit remain
  immediate. Every exit still uses paired reduce-only close facts.
- `SpotCross` follows the mature two-order flow documented by Hummingbot's
  [AMM Arbitrage strategy](https://hummingbot.org/strategies/v1-strategies/amm-arbitrage/): buy at
  the executable ask on one venue and simultaneously sell prefunded base inventory at the executable
  bid on the other. The scanner keeps the native pair names; the ticket re-quotes the target size,
  verifies both taker fees, buy-side quote balance, sell-side base inventory and both terminal fills.
  It never assumes transfer during execution, margin borrow or naked shorting. A market-only monitor
  may alert before inventory is funded, but Live submission remains blocked until the exact balances
  exist. Monitoring and execution classification both require a sustainable post-trade rebalance
  loop from current official wallet metadata: the buy venue must withdraw Base to a common network
  that the sell venue can deposit, and the sell venue must withdraw Quote back to a common network
  that the buy venue can deposit. Network aliases are normalized, conflicting token contracts fail
  closed, and disabled deposit/withdraw routes are never treated as available.
- The transfer-network registry is cold, candidate-triggered metadata rather than a market-wide
  poll. The scanner first finds a positive `SpotCross` or `CrossSpotPerp` candidate from its existing
  ticker/funding state and verifies the listing, product identity, both live market legs and fee
  floor. Only then does it asynchronously read the two candidate venues; the current row stays
  observation-only while that proof is warming. `SpotCross` includes the proven transfer cost in
  one-cycle profit, while `CrossSpotPerp` uses the proof as Webhook/execution availability evidence
  and leaves actual rebalance cost ticket-bound. Fresh evidence is reused for up to 30 minutes,
  failures use a bounded retry
  cooldown, and credential changes invalidate the affected venue without fetching until another
  candidate needs it. The
  selected route must carry its official fixed/percentage withdrawal fee and minimum withdrawal;
  any official minimum deposit is also enforced. Both route costs are converted at the opportunity's
  bound size and included in one-cycle net profit. Missing status, fee, limit or same-network evidence
  keeps the row observation-only and suppresses deterministic-opportunity Webhooks.
- USDT/USDC rows use the sell venue's executable FX bid before their spread is compared. They remain
  observation-only until a ticket explicitly adds the conversion order, reverse inventory cycle and
  settlement-asset risk; a stablecoin name is not a free one-to-one conversion. Other cross-quote
  rows are rejected. Funding caps stay unknown unless official per-contract cap evidence is carried
  by the market DTO.
- A shared ticker is not sufficient economic-identity evidence. The registry gate requires the
  strategy-specific Spot/Perp product, matching non-unknown asset classes, and compatible executable
  price units. Instrument registry identity is `(venue, product, native symbol)` so a venue's spot
  and perpetual products may share the same native symbol without overwriting each other. A spot
  leg with multiple quote markets binds the exact pair carried by its fresh WS evidence; only a
  base-only request may select the sole USDT spot default. Perpetual selection remains independent
  and may use the sole verified USDT-settled contract.
  Cross-class symbols or a price-scale ratio above 4x remain observation-only as likely
  same-symbol/different-asset or contract-multiplier mismatches; verified index constituents may add
  evidence but cannot override an explicit class or unit mismatch. Venue-native official metadata
  owns this classification: Gate `contract_type`, Bitget `isRwa`/`symbolType`, and Hyperliquid
  `perpCategories` must be preserved per native instrument. A Hyperliquid builder-dex namespace is
  not an asset class; absent or unrecognized non-crypto classifications remain unknown rather than
  being guessed.
- Scanner and Paper execution may be available before Live. Live `SpotCross` remains blocked unless
  both venue adapters prove Spot TradeWrite, official Spot InstrumentSpec, private finality, and the
  exact quote/base balances required by the ticket.
- `StrategyKindInfo.executionSupported` reports scanner/Paper execution support.
  `liveExecutionSupported` is a separate fail-closed capability and is true only for strategy kinds
  whose complete Live write/finality chain is available.

RWA, stocks, indices, metals, oil, and FX are allowed only when a verified exchange contract exists as a real tradable venue leg. Market-hours gaps, leveraged-product hedge ratios, index methodology, funding cadence/caps, settlement schedules, and FX exposure remain explicit independent risks. Reference assets, watchlist rows, registry rows, or unverified contracts must not enter executable strategy tables.

Product extension contracts:

- Automated arbitrage is default-off and reuses the same HedgeTicket, scoped preflight,
  ExecutionRun, finality, compensation, pair-risk and CloseRun path as manual execution. Paper must
  prove the whole loop. Live additionally requires an actual Live runtime, credentials,
  credential-scoped bilateral execution evidence, exit protection and the global risk gate; it has
  no separate process unlock control. A completed place/cancel/finality proof remains valid for the
  current credential epoch instead of expiring on an arbitrary timer. Credential changes remove the
  proof immediately, and a newer remote write failure overrides it fail-closed. Complete proof is
  atomically checkpointed with an opaque credential fingerprint; restart restores it only when the
  fingerprint still matches, while legacy execution-ledger events remain place-ack context only.
  Automation configuration is atomically persisted without credentials. Restart restores the saved
  strategy, environment and thresholds, but any enabled Live configuration starts paused until the
  operator verifies positions, open orders and protected-position fingerprints and explicitly resumes.
  Trading adapter selection, risk limits, venue/symbol scopes, kill switch, exit policy and protected
  position fingerprints are atomically persisted separately from credentials and restored before
  automation can be resumed.
  A fresh installation starts with a conservative $10 capital and $10 bilateral 5 bps depth
  requirement. The frontend draft derives those values from the shared DTO default rather than
  maintaining separate numeric literals, while a valid persisted operator configuration always wins.
  Its `canonicalSymbols` scope filters normalized opportunity base identities before preview;
  trading `allowedSymbols` remains a separate exact allowlist for the venue-native symbols emitted
  by the compiled order intents. An empty canonical scope means all bases, but never bypasses the
  exact trading risk gate.
  Legacy score fields remain read-compatible only. They do not filter, rank, recommend, preview,
  or authorize any strategy. Candidate ordering uses fresh verified one-cycle net profit floor,
  worst-leg market age, executable capacity and stable opportunity ID. An artifact-ready opportunity exists only after the
  snapshot-bound artifact passes every evidence row, including its strategy-specific cash-flow
  proof. Artifact determinism means reproducible bound inputs, not guaranteed market profit. Fee and
  history evidence alone never proves profitability. Funding tickets bind each venue's current
  native funding rate to its native next-settlement timestamp. `PerpCross` does not normalize those
  values to eight hours. Different native periods may pair only when both current next timestamps
  identify the same joint event; a staggered pair remains observation-only and contributes no
  pre-alignment cash flow. The single joint event must exceed complete open/close fees, measured
  bilateral slippage and the configured profit buffer. The proof remains projected because venue
  rates may change before settlement. `PerpPriceSpread` convergence remains statistical even when historical episodes are
  profitable and therefore remains Live-blocked. `SpotCross` may lock a positive floor only with
  `SellInventory`, a fresh bilateral Base/Quote transfer loop, and the resulting official transfer
  fees and limits included in its complete inventory-rebalance cost. Spot-perp variants remain projected until their basis exit, borrow and
  holding cash flows are ticket-bound. Candidate selection rejects bilateral WS
  evidence older than the same 30-second window used by artifact validation; opportunities without
  two fresh verified fee snapshots or a strictly positive one-cycle net floor remain observation-only.
  `estimatedFundingNextSettlementUsd` is the authoritative preview amount. The legacy
  `estimatedFundingPer8hUsd` wire key mirrors that native amount for older clients and must never be
  interpreted as an eight-hour normalization for `PerpCross`.
  A preview that passes every gate produces a short-lived `DeterministicExecutionArtifact` bound to
  the opportunity snapshot, ticket, idempotency key and checksum. It carries bilateral WS market
  age, 5 bps depth, order-plan, risk, cost, strategy profit-floor, net-edge and invalidation evidence. Copying or validating
  the artifact is read-only and cannot submit an order by itself. Both Paper/Shadow and enabled Live
  automation continue through the shared confirm state machine after a ready artifact. Live rechecks
  the current automation state, ticket, bilateral depth, scoped account/order preflight and every
  existing risk gate immediately before automatically submitting the two legs. After the first leg
  reaches filled finality, the second leg must read the already-warmed public WS book again within a
  bounded 500 ms window, resize from the actual first-leg fill, and fail into compensation rather
  than reuse ticket-time executable depth when that refresh is unavailable. Submission order is
  derived from the ticket's current bilateral 5 bps evidence: the shallower leg executes first and
  the deeper leg remains available as the rapid hedge; equal depth retains long-first ordering.
  The pre-submit refresh atomically rebinds one `marketCheckedAtMs` fact across bilateral VWAP,
  target quantity, instrument sizing, complete cost, profit proof, risk decisions, execution order,
  ticket guards and the compact workflow view before the run starts. Paper/Shadow and Live consume
  that same refreshed ticket; only the final venue mutation differs. Native-market intents move to
  the refreshed executable reference, while limit and protected-IOC intents retain the original
  ticket protection price and fail closed when the new VWAP crosses it. After scoped capability and
  margin reads plus this market refresh finish, confirm checks the original 60-second ticket expiry
  again immediately before creating the execution run; slow preflight cannot turn an expired ticket
  into a first-leg write. Order,
  error and unwind records remain stored by long/short business role even when the short leg is the
  first execution leg. Automatic preview is
  stricter than the operator workflow: the returned opportunity snapshot must still equal the snapshot
  selected by the worker, and the ticket's refreshed one-cycle net edge must still meet
  `minOneCycleNetBps` before an artifact can become ready. Snapshot drift cancels that attempt and
  reselects from the next worker snapshot; it must never silently inherit the manual same-opportunity
  rebind behavior. Manual hedge execution remains a separate operator-confirmed workflow. Exhausting a bounded preview batch records the
  actual attempted count plus per-candidate blockers and enters a retry cooldown of at least five
  seconds. The operator's successful-entry cooldown may be configured down to one second. Artifact
  generation errors also cool down before retry; a failed or ambiguous automatic write waits at
  least 60 seconds for reconciliation, even when the normal candidate cooldown is shorter.
  Automation evaluation is event-driven: a new opportunity snapshot, automation control change or
  execution-state change wakes the worker, while a future cooldown owns an exact deadline timer.
  The 30-second recovery interval only repairs a lost wake and must never become a preview or write
  polling loop. Worker-produced status events may coalesce into one additional evaluation, but an
  unchanged decision must not publish again or create a self-sustaining loop.
  The automation frontend seeds and recovers through REST, but compares `updatedAtMs` before
  applying a response: an older REST snapshot or its late transport error cannot replace or degrade
  a newer AppWS runtime. A current control-action failure remains visible against that latest runtime
  until a later successful state transition resolves it.
  Submission acceptance and execution finality remain separate facts. The last `Submitted` or
  `Replayed` decision is retained as the business event, while the visible runtime state follows the
  authoritative active `ExecutionRun`: an unresolved leg is `Submitting`, two terminally filled
  legs are `Hedged`, and an unwind requirement is `Blocked`. Synchronizing that transient state and
  the authoritative active-run count must not fabricate another decision or increment a replayed
  run twice. A `Filled` or `PartiallyFilled` state is not consumable second-leg evidence until the
  same identity also carries a positive finite filled quantity and price. An incomplete terminal ACK
  keeps waiting for the bounded private-WS update and then performs one identity-bound order query;
  it is never treated as a complete fill or cancelled as though it were still open.
  The historical `AutomationLiveUnlock` action kind remains read-compatible for old audit records,
  but no route or current workflow creates it.
  Validation remains available after confirm consumes the ephemeral preview by loading the immutable
  artifact recorded with the automation decision, recomputing its checksum, and reapplying ticket TTL
  plus the 30-second bilateral WS evidence window. The frontend submit gate requires a current
  `READY` response with `valid=true`; local readiness, a copied command, or a stale validation result
  never grants submit permission. An artifact bound to an `ExecutionRun` is consumed and its stable
  idempotency key prevents a second run.
- Automatic exit consumes the same published portfolio truth as AppWS instead of running a second
  two-second portfolio poll. A private-account event can therefore wake the portfolio projection and
  exit evaluation immediately; the ordinary two-second portfolio cycle remains reconciliation, and
  a 30-second worker interval is recovery only. When every protection switch is off, the worker exits
  before reading Portfolio, order or execution-run state. Position pairing and automatic-exit
  eligibility require recorded orders from both `ExecutionRun` legs to match the active product
  environment. DryRun and Testnet both project to Paper; Live accepts only Live records. Missing or
  mixed-environment order evidence fails closed, so an old Paper run cannot pair or close a Live
  position after the operator changes mode. Live eligibility is scoped to the two paired position
  rows rather than the whole account envelope: unrelated NAV, balance or another venue's degradation
  cannot suppress protection for a fresh pair. When the position fanout itself is degraded, both
  exact venue/symbol/side rows must carry a successful sample no older than six seconds; stale or
  missing pair evidence still fails closed. A successful change to any automatic-exit setting asks
  the coalesced Portfolio publisher for one immediate refresh, so enable, disable and threshold
  changes do not wait for the ordinary reconciliation interval.
  Liquidation protection is evaluated before financial valuation. It requires a fresh, actual
  exchange liquidation-distance field for the triggering leg, but it never waits for funding,
  opening-cost or PnL reconciliation. A distance at or below zero means that the mark has already
  crossed the reported liquidation line and exits immediately; a positive approach distance retains
  the configured confirmation filter. Take-profit and stop-loss require the complete valuation and
  retain their configured confirmation samples. Those samples count only when the older of the two
  exact account-position success timestamps advances. Public mark updates may publish newer
  Portfolio versions and update liquidation distance, but they cannot multiply one venue-native PnL
  observation into several take-profit or stop-loss confirmations.
  The exit calculation honors the ticket cost-recovery horizon. A convergence ticket or a recurring
  ticket with more than one recommended hold period does not treat its known opening and estimated
  closing costs as immediate adverse movement. Venue unrealized PnL already uses the actual average
  entry and current mark, as exposed by
  [Binance Position Information V3](https://developers.binance.com/docs/derivatives/usds-margined-futures/trade/rest-api/Position-Information-V3),
  [Bybit Get Position Info](https://bybit-exchange.github.io/docs/v5/position) and the
  [OKX position PnL contract](https://www.okx.com/docs-v5/). Automatic-exit valuation therefore
  removes the actual opening fee from `actualOpenCostUsd` instead of subtracting opening slippage a
  second time; funding, the conservative close/slippage envelope and the configured safety buffer
  remain explicit. Instant-spread tickets never receive the cost-recovery delay.
- Automatic exit idempotency is scoped to one fresh protection-evidence observation and one
  `ExecutionRun`: bilateral account evidence for take-profit/stop-loss, or the triggering
  liquidation-distance evidence for liquidation protection.
  Its post-attempt cooldown blocks only the same paired run, so another independently protected pair
  is not delayed. Every confirmed candidate from the same Portfolio publication remains eligible in
  risk-priority order; the worker submits at most two independent pair exits concurrently instead of
  making lower-priority risk wait for the 30-second recovery cycle. The shared trading rate limiter
  and each run's idempotency contract remain authoritative. An unlinked accepted or
  successful close action, a submitted/partial close, unresolved unwind, failed compensation, manual
  terminal, or successful close blocks every new attempt. A new observation may retry only after a
  close proved that zero orders were submitted or after compensation proved the original pair was
  restored. A `Hedged` run continues consuming automation concurrency whenever portfolio evidence is
  missing, stale, degraded, still pair-linked, or still contains either matching leg; only a fresh,
  complete account snapshot proving both legs absent can release it.
- Pre-existing operator positions can be locked with `protectedPositions` in the trading risk
  configuration. Every fingerprint carries venue, canonical and venue-native symbols, side,
  quantity, entry price, position mode, opening identity, evidence source and capture time. The
  shared RiskEngine blocks both symbols for standard orders, hedge checks and reduce-only unwind;
  close, automation and compensation paths cannot bypass it. Automation also removes a candidate
  before preview when either venue leg matches the protected venue and canonical symbol, while the
  RiskEngine remains the final fail-closed authority. Updating or clearing the list is a high-risk
  audited mutation. The list is restart-scoped: after restart Live execution must remain blocked
  until the current account position is read again and the fingerprint is restored and verified.
- Paper and automated flow verification discovers candidates from execution eligibility without
  requiring full-market depth snapshots; the shared preview performs the bounded bilateral depth
  check on demand before confirmation. A verification run may coexist only with account positions
  that exactly match the active protected fingerprints; any other starting position or open order
  remains a fail-closed precondition failure.
- Review realized PnL must include the terminal prices of the linked pair CloseRun. Opening-leg
  spread alone is not realized profit: without two matched, filled, reduce-only close legs, gross
  and net remain missing and the row is excluded from strategy-performance samples. A complete
  Paper close is estimated evidence; a Live close is actual only when private WS, order-query or
  reconciliation finality proves both fills. Hot and durable CloseRuns feed the executed-detail and
  strategy-performance projections through the same merge path.
  Strategy-performance headline PnL, wins/losses, hit rate, expectancy, drawdown, Sharpe, Sortino
  and Profit Factor use only rows whose Net field is actual. Rows with estimated Net remain visible
  in the computable-sample count and `estimatedNetPnl30dUsd`, but never enter those realized
  performance metrics or `netPnl30dUsd`. An estimated-only strategy reports no complete sample
  instead of manufacturing a profitable or losing track record. The legacy
  `independentPeriods30d` wire key counts unique closed execution-group identities among those
  actual-Net rows. It is an independent realized-loop count, not an eight-hour clock bucket or a
  funding-rate normalization.
- Private-WS operation health separates current transport readiness from business-event evidence.
  An authenticated current session with confirmed subscriptions may report an idle account or order
  stream as ready with `rows=0`; account-stream readiness additionally requires operational balance
  and position cache evidence. `Fresh` cache is authoritative, while a `Stale` snapshot is accepted
  only inside the operation's bounded background-refresh grace; expired, wrong-epoch or out-of-grace
  snapshots remain unproven. This idle state never proves write permission or order finality. Once an
  unresolved Live order exists, a missing fresh order event remains `Warn` and cannot be replaced by
  the idle-session projection.
- Private account projection has one event boundary. Submit ACKs and terminal updates with no
  positive filled quantity update only the order projection; they do not invalidate balances or
  positions. A fill, funding/liquidation event or explicit venue account/position event is required
  before account state changes. Complete private-WS snapshots patch the venue cache and wake the
  portfolio/AppWS projection through a fixed 100 ms coalescing window. Incomplete events invalidate
  only their declared venue/scope. One lifecycle-owned queue merges duplicate venue/scope dirtiness
  across decoded WS batches, and one supervised worker drains it after a fixed 100 ms coalescing
  window. Explicit venue events that require REST recovery proceed immediately after that window.
  Order-driven `fill_event` and `terminal_order_update` dirtiness receive one additional bounded
  250 ms settle window so a following private account event can replace the invalidated scope first;
  if it arrives, the worker observes the fresh cache and skips REST. Otherwise each drain runs at
  most one lock-coalesced position read and balance read in parallel. Order/CloseRun updates publish
  immediately and independently, while a full Portfolio snapshot publishes only after every dirty
  scope in that decoded batch is already fresh or at least one scoped recovery succeeds. A failed recovery does not push
  the known-old account rows as a new snapshot. Only outcomes that pass the private-ledger durability
  gate may enqueue this recovery or reach the order/portfolio projection. The two-second portfolio
  cycle remains only a reconciliation ceiling, not the hot path, and repeated refresh requests must
  reuse the fresh cache instead of clearing it again.
  The frontend records the origin of the last accepted Portfolio snapshot independently from WS
  channel health. A REST bootstrap/fallback response never advances WS freshness, and every accepted
  WS snapshot invalidates older in-flight REST responses before it reaches the positions state.
  A close, paired close, bulk close, compensation or cancel response updates only its typed action
  result; it does not start a second browser-driven Portfolio REST read. The private account/AppWS
  path above owns the hot account update, while REST polling remains active only when that AppWS
  channel is disconnected or stale.
  Portfolio REST, AppWS replay, automatic exit and system health consume that same lifecycle-owned
  snapshot. The REST route must never rebuild account state, history or risk on request: before the
  first publication it returns typed unavailable/warming evidence, and after two missed lifecycle
  intervals it serves the last publication as explicitly stale with its measured freshness. It does
  not write that request-local stale projection back into the authoritative cache. SystemHealth
  similarly consumes the lifecycle-owned Portfolio snapshot and exposes only its own five-second
  lifecycle cache; before the first system publication the REST route returns `Warming` without
  rebuilding diagnostics, reading accounts or querying PnL history in the request path.
- Current Funding has one runtime fact: `MarketDataCache`. Public WS ingest and the bounded funding
  discovery lifecycle may update that cache; Funding REST, Funding AppWS replay, Portfolio and
  strategy projections are read-only consumers. They must not maintain a second current-rates
  snapshot or write a fallback result back during a request. The funding lifecycle publishes the
  cache projection after discovery so a fresher WS row remains authoritative over a REST row.
- AppWS replay is a pure cache projection. A new subscriber may replay only a snapshot already
  published by the owning lifecycle; it must not run SystemHealth computation, synthesize a
  Portfolio envelope from an older raw snapshot, scan the Review ledger or mutate any canonical
  cache. Before the first System, Portfolio or Review publication, replay emits no fabricated frame.
  Review REST returns a typed degraded `REVIEW_RUNTIME_SNAPSHOT_WARMING` envelope during that same
  window instead of becoming a second ledger materializer or cache writer.
- Spot Live requests are product-bound. Every place, cancel and query carries `FeeProduct::Spot`,
  resolves an official venue-native InstrumentSpec, and compiles quantity/price from the same
  ticket-bound sizing evidence before reaching a venue transport. Binance, OKX, Bybit, Bitget,
  Gate, Gate CrossEx, Kraken, KuCoin and Hyperliquid use their official WS trade request methods for
  Spot place/cancel. All nine venue families feed Spot terminal order/fill events from private WS into the same durable order
  ledger. A transport ACK never proves a fill, and health is published only after the mapped event
  has been durably applied. Point queries remain WS where the venue supports them and otherwise use
  an identity-bound signed REST/Info request; an ambiguous write is reconciled and never replayed
  blindly over another transport.
- On-chain/CEX comparison keeps discovery read-only, but a selected profitable direction can enter a
  short-lived execution build. Solana uses a taker-bound [Jupiter Swap V2 order](https://developers.jup.ag/docs/swap/order-and-execute);
  supported EVM chains use a firm 0x Swap API V2 AllowanceHolder quote or the signed
  [OKX DEX Aggregator V6 swap endpoint](https://web3.okx.com/onchainos/dev-docs/trade/dex-swap).
  CoW Protocol `priceQuality=fast` remains observation-only until its EIP-712 order and solver
  finality path are implemented. Quote Provider and EVM node evidence are separate:
  an optional custom public HTTPS RPC normally probes `eth_chainId` and `eth_blockNumber`; an explicit
  token-identity request may additionally call `eth_getCode` and read-only `eth_call` for ERC-20
  `symbol()`, `decimals()` and optional `name()`. Its URL is retained in backend memory and never
  returned or audited. Chain presets choose a default Provider and known token identities, while the
  operator may select another Provider supported by that chain. Contract/Mint-first identity lookup
  requires an exact Jupiter mint row on Solana or chain-id-matched ERC-20 metadata on EVM. Jupiter's
  verification bit is preserved; EVM metadata remains explicitly contract-reported because ERC-20
  metadata methods are optional. The frontend starts identity lookup after a 450ms input debounce,
  discards stale responses and uses the adjacent action only to clear the field. CEX pair choices
  come from the selected venue's official Spot instrument registry; current ticker evidence is
  overlaid as market readiness and must not decide whether an officially listed pair is selectable.
  Changing Base or venue clears the selection, while changing the on-chain Quote preserves the
  operator's explicit CEX choice. The catalog exposes every official Spot quote for the resolved
  Base and never invents symbols from token text. Neither frontend nor API may silently derive or
  overwrite that choice.
  Ambiguous or unreadable identities fail closed. Base identity and orderbook identity must agree.
  The operator may compare any official CEX Spot quote for the same Base with the configured
  on-chain pair. A cross-quote selection compares the two raw displayed prices without currency
  conversion; API and UI must preserve both quote labels and must not claim FX or stablecoin parity.
  The pair field also accepts an explicit `Base/Quote` value when the official catalog is temporarily
  unavailable. This keeps read-only monitoring usable; build and submit still require fresh official
  instrument, orderbook, balance and private-finality evidence.
  User-facing
  on-chain rates use percentages while shared runtime contracts retain basis points.
  Build readiness additionally requires fresh bilateral inventory, gas, an official executable CEX
  Spot instrument, current WS depth, a remote CEX preflight and strictly positive net profit after
  current fees/slippage. For 0x AllowanceHolder, an ERC-20 input may enter firm-quote construction so
  the authoritative [`issues.allowance`](https://docs.0x.org/docs/core-concepts/contracts) response
  can prove or reject the exact spender and amount;
  absence of that issue is required before the build can become submit-ready. OKX DEX and CoW ERC-20
  inputs remain blocked until their approval transaction and finality lifecycle is implemented (the
  OKX boundary starts at its official
  [Approve Transactions](https://web3.okx.com/onchainos/dev-docs/trade/dex-approve-transaction)
  endpoint). Never
  approve the 0x Settler contract; use only the AllowanceHolder target returned by the official
  response. Kraken Spot is a first-class CEX leg and must retain its
  official [WebSocket v2 instrument](https://docs.kraken.com/exchange/api-reference/spot-websocket-v2/instrument)
  native symbol, sizing and private order finality. If that bounded startup snapshot has not arrived,
  one official [AssetPairs](https://docs.kraken.com/api-reference/market-data/get-tradable-asset-pairs)
  read may seed the registry; this is metadata recovery, not periodic market-data polling.
  Wallet private keys are validated against the configured wallet and stored only in the configured
  backend credential store. A submit request contains only the one-time `buildId`; the server claims
  the immutable build, rechecks its five-second evidence window, signs the exact Provider transaction
  and never accepts browser-supplied calldata or serialized transactions at submit time.
  The CEX IOC Spot leg is submitted first and must reach a full terminal fill before chain broadcast.
  An unfilled or partial CEX leg is cancelled and reversed. A definite chain rejection starts the
  opposite CEX compensation order; a transport-ambiguous chain result keeps the CEX hedge, follows
  the same Solana signature or EVM transaction hash for about two minutes and never guesses by
  reversing early. Jupiter retry/poll uses the identical signed transaction and request ID, while
  custom Solana/EVM RPC uses `getSignatureStatuses` or `eth_getTransactionReceipt`. Recent runs are
  backend-owned and the frontend polls only while one run is awaiting chain finality. Completion,
  compensation, unresolved finality and exposed balance all emit their typed Webhook result.
- Webhook delivery is default-off, limited to resolved public HTTPS targets and never returns its
  target credential or signing secret. Saved targets, signing secrets, provider, event scope and
  delivery limits use the configured credential backend and are restored after restart; an invalid
  restored enabled configuration remains visibly configured but disabled instead of being silently
  discarded. Generic delivery signs the exact event body and treats 2xx as
  transport evidence only. Bark delivery posts its current JSON body to the normalized device-key
  endpoint and requires the official response JSON `code=200` before recording application
  acceptance. Versioned events use stable IDs, bounded queues, timeouts and retries. An accepted
  event is written to the SQLite outbox configured by `APP_STORAGE__WEBHOOK_OUTBOX_PATH` before it
  enters the in-memory delivery queue; target URLs and signing secrets never enter that database.
  Restart replays pending rows, restores recent terminal records and durable counters, and retains
  terminal IDs for bounded deduplication. Delivery completion that cannot be committed is retried
  locally without repeating the HTTP request; a crash after remote acceptance but before the local
  commit may still redeliver, so generic receivers must deduplicate the stable
  `x-crossline-event-id`. This is durable at-least-once processing, not a false exactly-once network
  claim. Redirects, private targets, secret logging and unbounded replay are forbidden. Only a ready, snapshot-bound
  execution artifact owns an `opportunity` alert; a positive public ticker/funding candidate never
  emits that event by itself. Candidate-triggered Spot/Perp transfer checks use the separate
  `opportunity_monitor` scope, so their delivery cannot make the UI or automation flow claim a
  deterministic artifact. A `SpotCross` monitor may label its current fee-adjusted spread as
  `locked_spread`; `SpotPerp` and `CrossSpotPerp` must label the basis as `projected_basis` because
  the exit cash flow is not locked. Both available and blocked transfer results remain useful
  monitor evidence, but neither bypasses preview, depth, balance or strategy-profit proof. The
  artifact includes route, cost, net edge, evidence, expiry,
  invalidation conditions and a read-only validation command. Candidate-selection, no-candidate and
  preview-blocked heartbeats remain in the UI decision ledger rather than becoming notification
  spam. Every ticket binds a `transfer_route` guard. `SpotCross` and `CrossSpotPerp` artifacts stay
  blocked while official Base/Quote transfer metadata is warming or unavailable; a ready artifact
  and its Webhook payload retain the proven route detail and Memo/Tag requirement. Same-venue
  `SpotPerp` binds explicit `not_required` evidence instead of pretending a cross-venue route exists.
  `onchain_spread` is emitted only for fresh
  observation-only snapshots whose net spread reaches
  the configured threshold; one stable event ID per cooldown window prevents quote cadence from
  becoming notification spam.
  Lifecycle bridge cursors advance only after a successful enqueue. Queue rejection leaves the
  source terminal state eligible for the next bridge pass, while dispatcher-level event-ID
  deduplication prevents a successful enqueue from becoming duplicate delivery. The first durable
  outbox initialization records existing terminal execution IDs as a no-delivery baseline, so an
  upgrade never turns historical runs into notification spam. Later startups seed the execution
  cursor only from IDs already present in the outbox; a restored `Hedged`, `FailedSafe` or `Closed`
  run that became terminal before the bridge could enqueue it is therefore recovered exactly once
  into the pending queue. Restored automation, compensation and system state retain their
  historical startup baseline. A new canonical opportunity generation
  may wake the bridge to evaluate resolved candidate transfer evidence, but stable event keys make
  unchanged candidates no-ops; system snapshots plus payload-free automation-artifact, execution,
  CloseRun and ActionRun activity pulses trigger only their owning projection. Automation activity
  also checks its artifact because both facts share the same decision snapshot; unrelated activity
  never rescans every execution, compensation, opportunity and system source. Internal activity
  receivers are not
  AppWS subscribers and must not activate periodic Portfolio payload serialization or introduce a
  second business-state cache. A 30-second recovery pass is retained only for lost-wake recovery;
  it is not a terminal-state scanner. Webhook delivery blocks directly on its bounded queue, so an
  arriving event wakes immediately; the same 30-second timeout is only an idle task-health
  heartbeat and replaces the former 500ms empty-queue wakeup.
  Execution-result events use stable `run_id + state` identity. `Hedged` is emitted only after both
  legs are terminally `Filled`; `FailedSafe` and `Closed` each form a separate meaningful result.
  Updating valuation, evidence, timestamps or another field while the run remains in the same state
  does not repeat the notification, while a later transition from `Hedged` to `Closed` does.
  System health heartbeats are compared by
  semantic risk/problem identity rather than timestamps: a severity above the last notified level
  and a new degradation after a fully healthy recovery remain immediate, while same-severity detail
  changes are limited to one notification per 15 minutes and one snapshot selects only one alert
  kind. A risk escalation emits `risk_alert`; an apparent risk drop while the system remains degraded
  does not re-arm that severity because missing portfolio evidence is not a confirmed recovery.
  Changing market/API problems without a new risk escalation emit
  `system_degradation` instead of being mislabeled as portfolio risk. Risk and system notifications
  expose a concise top-level `message` for mobile providers while retaining the complete typed health
  fields for generic webhook consumers; provider bodies must never fall back to a truncated
  serialization of the full health snapshot.
  Bark's JSON `id` becomes the APNs collapse identifier in the official server. It must be the
  deterministic SHA-256 hex digest of the complete CROSSLINE event ID so it remains exactly 64
  ASCII bytes; the unmodified event ID remains the audit, deduplication and request-header identity.

## 3. Performance Defaults

- Execution-sensitive market-data hot paths are WS-only after cold-start discovery. A REST row may
  discover or observe a market, but it cannot substitute for fresh WS price, funding or order-state
  evidence in build, automation or execution readiness.
- Cold start, static metadata, history, and calibration are REST-first.
- Product modules read through shared runtime/cache services, not direct adapter fanout.
- Every market/runtime data surface should carry source, freshness, problem, and retry information when exposed to product UI.
- The five-second full opportunity scan remains a recovery/safety cadence. Fresh public WS changes
  request an event scan immediately; the snapshot worker coalesces them with a start-to-start window
  equal to four times the previous complete scan/publish duration, bounded between 250ms and five
  seconds. This removes the former fixed five-second WS-to-product wait while keeping sustained
  scan/publish duty near 25% and preserving one serialized opportunity truth for REST and AppWS.
  The lifecycle worker is the only scan writer. REST list reads, including compatibility requests
  carrying `fresh=true`, return the current lifecycle publication or typed warming evidence and
  never start a parallel scan, fetch market data or mutate the opportunity cache.
- The strategy-facing market snapshot is a lazy, per-feed versioned projection. Funding, perpetual
  tickers, spot ticks and index compositions publish immutable `Arc` rows through `ArcSwap`; a write
  invalidates only its own feed, so an unchanged feed is not traversed, deep-cloned, regrouped or
  sorted again on the next scan. Projection expiry follows the feed's inclusive freshness/discovery
  boundary. Row evidence and aggregate health are still rebuilt against the current clock on every
  scan, so sharing row storage must never freeze freshness, source, retry or problem semantics.
- `PerpCross`, `PerpPriceSpread`, `SpotPerp`, `CrossSpotPerp` and `SpotCross` are registered in one
  strategy registry and each consume the same immutable market snapshot exactly once per scan.
  There is no separate cross-exchange scanner object, async market-source wrapper or strategy-local
  fetch path. Snapshot acquisition is a synchronous cache projection; the lifecycle worker moves the
  complete CPU scan onto one bounded Tokio blocking task before normalizing and publishing the sole
  REST/AppWS opportunity truth. This keeps public/private WS workers responsive without creating a
  second scan writer or another market-data state. The strategy snapshot excludes execution-only
  order-book health and dead option payloads. With no `arbitrage` AppWS subscribers, the lifecycle
  still refreshes the canonical report and alerts but does not materialize or serialize a frontend
  window delta.
- One scan captures one clock before strategy evaluation. Candidate history, native settlement
  projection, fee snapshots, profitability evidence and every emitted DTO reuse that exact time;
  per-row wall-clock reads must not make evidence inside one publication disagree about freshness or
  settlement. Fee-only frontier ranking may use the numeric official registry rate, but complete fee
  evidence is allocated once only for rows retained by the bounded frontier.
- `OpportunityIndex` publication owns the authoritative `snapshotId` and `cachedAt`. REST lists,
  AppWS deltas and AppWS replay forward that identity unchanged instead of recomputing it from rows.
  An AppWS frame with no changed or removed IDs updates channel health only and must not clone or
  replace the frontend's ID-indexed row projection.
- Opportunity-list scans do not create a synthetic return history. Scan observations are neither
  funding settlements nor fills, and changing the event cadence must never change a displayed risk
  result. The former per-candidate DashMap history and fixed-eight-hour Sharpe/volatility path are
  removed from the hot loop. List risk is structural: joint-event `PerpCross` and prefunded instant
  `SpotCross` are Medium before ticket validation; convergence and projected-basis strategies are
  High until their exit, borrow and holding cash flows are ticket-bound. Realized volatility,
  drawdown and strategy performance come only from matched open/close fills in review.
- The optional futures-table `current Funding` value is the native next-event short-leg rate minus
  the native next-event long-leg rate. It never reuses total strategy edge, opening basis or
  convergence spread; missing funding evidence remains missing instead of becoming numeric zero.
- Private funding-payment ingestion is a five-minute background signed-REST ledger reconciliation,
  not an execution hot path. Each venue receives one bounded 10-second route budget including shared
  rate-limiter queue time, with no in-cycle retry or request-rate increase. Binance uses the official
  [`GET /fapi/v1/income`](https://developers.binance.com/en/docs/catalog/core-trading-derivatives-trading-usd-s-m-futures/api/rest-api/account#get-income-history)
  `USER_DATA` contract and its IP weight 30. Its live private adapter uses the shared conservative
  venue budget of 20 request-weight units per second, half the current official `exchangeInfo`
  limit of 2400 per minute. This changes permit latency only; task cadence and request count stay
  unchanged.
- KuCoin Classic Futures private reads share the official Futures request-weight pool. Account
  overview uses [weight 5](https://www.kucoin.com/docs-new/rest/account-info/account-funding/get-account-futures),
  while [position list](https://www.kucoin.com/docs-new/rest/futures-trading/positions/get-position-list)
  and [order list](https://www.kucoin.com/docs-new/rest/futures-trading/orders/get-order-list) each
  use weight 2, and [private funding history](https://www.kucoin.com/docs-new/rest/futures-trading/funding-fees/get-private-funding-history)
  uses weight 5. The Live private adapter therefore uses the shared conservative KuCoin budget of
  20 weight units per second instead of imposing a second local two-unit bottleneck. This changes
  local permit latency only; account refresh cadence and upstream request count stay unchanged.
- Bitget UTA [account assets](https://www.bitget.com/api-doc/uta/account/Get-Account),
  [current positions](https://www.bitget.com/api-doc/uta/trade/Get-Position) and
  [open orders](https://www.bitget.com/api-doc/uta/trade/Get-Order-Pending) each permit 20
  requests per second per UID. Its Live adapter uses the shared conservative 10-request venue
  budget instead of a second local two-request bottleneck. OKX
  [positions](https://www.okx.com/docs-v5/en/#trading-account-rest-api-get-positions) permit 10
  requests per two seconds per user, while
  [pending orders](https://www.okx.com/docs-v5/en/#order-book-trading-trade-get-order-list) permit
  60 per two seconds; its Live adapter likewise uses the existing shared conservative venue budget.
  These changes reduce local queue time without increasing refresh cadence or request count.
- Hyperliquid REST requests share the official
  [1,200-weight-per-minute budget](https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/rate-limits-and-user-limits):
  `clearinghouseState` costs 2 units and ordinary `info` requests such as `metaAndAssetCtxs` cost
  20. A private position refresh therefore validates the scoped clearinghouse state first, returns
  immediately when it has no position rows, and loads the DEX-scoped single-flight mark context only
  when a position needs a mark price. This preserves strict position evidence while avoiding an
  unnecessary high-weight request for every flat builder DEX at startup. Consolidated spot account
  scope and USD-equity evidence is fetched once before the first combined account snapshot, then
  refreshed through one shared single-flight at 45 seconds while the prior proof remains valid for
  60 seconds. A failed refresh still expires fail-closed; it is never converted into verified scope.
- The top `MarketData` ratio counts execution instrument specs plus live WS funding, perpetual and
  spot snapshot/subscription evidence. REST cold-start, discovery and recovery rows remain visible
  in the detail text but do not dilute or mask the realtime-core ratio.
- Opportunity scans must not fan out orderbook requests across candidates. The opportunity-list
  contract carries no depth field, so rendering, filtering, sorting, selecting or opening a row
  cannot create a depth subscription or fallback request. An explicit hedge build, automation
  candidate confirmation and submission preflight are the only owners of bilateral execution-depth
  reads and strict 0.05% target-notional validation. The shared per-market single-flight cache
  prevents those execution stages from duplicating the same upstream request. If the ranked list advances while
  the operator opens a manual preview, the frontend adopts only the backend-reported current
  snapshot ID for that same opportunity and retries through the unchanged fail-closed preview gate.
  The strategy-facing `OpportunityMarketDataSource` exposes one lifecycle-owned snapshot method and
  no orderbook or fallback-fetch method, so a list scan cannot regain candidate-depth or Funding
  REST fanout through a generic market-data abstraction.
- List-stage `optimalPosition` is a configured planning budget derived from total capital, the
  strictest opportunity/venue/symbol cap, allocation scale and risk tolerance. It is not Kelly and
  never annualizes an instant spread or native funding event. Build/ticket sizing remains authoritative
  after fresh bilateral depth, balances, instrument limits, fees and complete strategy cash flows.
- Opportunity price evidence uses ranked, bounded WS subscriptions: the live planner prioritizes the
  first 50 cross-perpetual rows with at most 16 ticker symbols per venue and market, then considers
  24 other ranked rows with a 4-symbol bound. Only a discovery row with finite, positive
  fee-adjusted `netSingleYield` may claim this exact ticker/Funding confirmation budget; zero,
  negative and non-finite rows remain discovery-only. A cross-quote candidate may add only its exact
  stablecoin Spot market, with at most two shared conversion slots per venue; those slots do not
  trigger a market-wide stablecoin scan and remain inside the global per-venue bound. It detects a new atomic opportunity generation on the
  next 100ms projection tick and clones only the bounded rows that can enter the ranked subscription
  window rather than the complete opportunity index. Candidate legs and conversion markets are
  capacity-checked before they are inserted, so atomicity does not require cloning the accumulated
  venue plan for every row. A new opportunity generation recomputes that bounded projection, but the
  combined opportunity/Portfolio plan is replaced only when its venue-symbol set actually changes;
  a 5s safety comparison remains. The 100ms ingest loop reuses that immutable plan instead of
  rebuilding venue maps and symbol strings. It must not subscribe to candidate orderbooks; REST
  bootstrap or fallback rows retain their source label until a fresh WS row replaces them. Candidate
  pairs and any required conversion market enter the plan atomically so an unavailable leg cannot
  consume the other legs' budget. Perpetual leg evidence is keyed by venue plus exact native
  contract first, then by a unique canonical base/quote identity; a base-only request may use the
  venue's documented default quote, but `USDT` and `USDC` rows never overwrite one another. Once a
  venue publishes per-contract row evidence, a missing requested contract cannot borrow the
  aggregate feed-health row as proof of fresh WS coverage. Exact leg evidence always records the
  row's cache-receive timestamp, not the later strategy-scan timestamp, and aggregate feed health
  can never manufacture evidence for a specific contract. Static first-frame probe symbols are
  forbidden: Kraken, Gate CrossEx and every other venue open public subscriptions only for an
  enabled watchlist, a ranked opportunity or a current position.
  Current account positions independently add only their exact native perpetual contracts, bounded
  to 32 mark/index symbols per venue, to that immutable live plan. Official public WS mark/index
  snapshots feed a separate three-second freshness cache; REST rows and full-universe mark streams
  cannot enter this position hot path. A mark change wakes the coalesced Portfolio projection while
  stable position identity keeps the subscription plan unchanged. The projection may update mark
  price and signed liquidation distance, but it preserves the venue-native unrealized PnL instead
  of manufacturing a second PnL calculation.
  Binance perpetual discovery uses bounded `<symbol>@ticker` rows as the complete statistics
  baseline and overlays bounded `<symbol>@bookTicker` BBO updates; neither the all-market book
  stream nor a periodic full-market REST statistics refresh belongs in the planner. Spot discovery
  is also one row per planned pair: Binance's official
  [`<symbol>@ticker`](https://developers.binance.com/docs/binance-spot-api-docs/web-socket-streams#individual-symbol-ticker-streams)
  carries last, quote volume, BBO and sizes; Gate's official
  [`spot.tickers`](https://www.gate.com/docs/developers/apiv4/ws/en/#tickers-channel) carries last,
  quote volume and BBO while deliberately leaving sizes unknown; KuCoin's official
  [`/market/snapshot:{symbol}`](https://www.kucoin.com/docs-new/3470065w0) carries BBO, sizes, last
  and 24h quote volume. Scanner rows must not add a second BBO stream or periodic all-market volume
  join. Per-symbol execution depth remains owned by selected-candidate preview, automation preflight
  and submission. Hyperliquid HIP-3 WS coin identities retain the official `dex:SYMBOL` prefix.
- Gate contract metadata refreshes are single-flight. Broad funding refreshes reuse the verified
  contract interval/next-apply schedule and combine it with the current official futures-ticker
  funding rate, rather than downloading the full contracts table for every funding cycle.
- Gate perpetual ticker and BBO hot rows use the official production SBE endpoint pinned to schema
  id 1. Subscription control remains JSON text; `futures.tickers` and `futures.book_ticker` data
  frames are decoded from the official little-endian schema. The shared manager publishes
  `Connected` only after its writer is installed; a failed subscribe send releases its in-flight
  claim so the next projection can retry without spawning duplicate tasks. Because Gate's thin
  `futures.book_ticker` rows can remain silent until BBO changes, each active candidate receives one
  `futures.order_book` level-1 SBE snapshot and immediately unsubscribes; a 20-second bounded refresh
  keeps that bootstrap younger than the 30-second cache window without opening a continuous depth
  feed. Sparse `futures.tickers` rows are refreshed by an ordered unsubscribe/subscribe at 15
  seconds, and a row still absent for 30 seconds may trigger one connection-level snapshot recovery
  no more than once per 120 seconds. An invalid schema or frame fails that live WS sample closed as
  missing; the separately labelled REST discovery baseline must not replace it as live evidence. Do
  not apply Bitget's real-time SBE BBO to the whole scanner hot set: its
  unthrottled event cadence can exceed the venue's 300-400ms JSON ticker bandwidth. Reserve that path
  for a bounded build/execution hot pair if introduced later.
- KuCoin futures mark price, index price, open interest, current funding rate, next settlement and
  funding interval use the official public Pro `mark-price` and `funding-fee` channels on one
  connection. Each channel owns an independent 64-topic LRU watchlist: funding requests do not
  activate the one-second mark feed, while funding topics remain eligible long enough to receive
  the official one-minute push. The live funding row must be complete from that WS frame; do not
  stitch settlement metadata from a REST contract row. Subscription commands are deduplicated and
  paced below the official 100 messages per 10 seconds per-connection limit. Pro market JSON
  arrives in binary WS frames and is decoded through the shared strict UTF-8 codec. The separate
  classic futures ticker connection remains because its 24h snapshot carries quote turnover absent
  from the Pro ticker. KuCoin Spot uses the single-symbol snapshot channel rather than joining the
  classic 100ms ticker to a periodic REST `allTickers` volume baseline. Partial current-plan
  snapshots track the first-event age of each missing symbol instead of immediately reporting a
  transport failure. KuCoin spot snapshot, classic
  futures snapshot and Pro funding rows receive bounded 8s, 7s and 65s warm-up windows around the
  official 2s, 5s and 1-minute push cadences; the spot allowance includes live-observed subscription
  control and first-event jitter. Warm-up remains unverified and cannot satisfy build,
  automation or execution evidence; a symbol still missing after its window becomes a normal
  fail-closed `MARKET_DATA_MISSING` problem.
- KuCoin Classic Futures `size`, `filledSize`, `matchSize` and `currentQty` are contract counts.
  The exchange adapter must multiply signed REST order, fill and position results by the exact
  instrument `multiplier` before exposing shared base-quantity DTOs. Classic private WS may retain
  terminal identity/status evidence, but its raw quantities and fills must not enter the journal or
  position cache; those events trigger the official signed order/position query, including
  `GET /api/v1/orders/{orderId}`, for metadata-backed quantities and fee evidence.
  `/contract/positionAll` is a change-notification stream: production close and settlement pushes
  may omit full REST-only risk fields. Validate the native contract identity, contract count and
  event time, invalidate the applicable account scope, and preserve that native symbol through
  multiplier lookup before exposing the normalized base symbol.
- Public and private Binance, OKX and Bybit connections offer RFC 7692 `permessage-deflate` through
  the shared WS supervisor. The allowlist is exact-host and WSS-only because live handshake and wire
  sampling proved real server-side payload compression for those hosts. If a server declines the
  extension, the connection remains an uncompressed WS with identical subscriptions, cadence and
  product semantics. Compression negotiation must never trigger a REST fallback, reduce symbol
  coverage, or slow a stream. KuCoin is excluded because live payload sampling showed no outbound
  byte reduction despite extension negotiation; Bitget, Gate and Hyperliquid are excluded
  because their tested production endpoints did not negotiate RFC 7692. Official protocol:
  <https://www.rfc-editor.org/rfc/rfc7692>.
- OKX private WS order/cancel messages use an independent transport `id` that is unique, exactly 32
  lowercase hexadecimal characters, and never reuses the internal UUID. The order identity remains
  the official `clOrdId`. This follows OKX's 1-32 alphanumeric WS correlation contract; the matching
  response must carry the same transport `id`. An exact signed `GET /api/v5/trade/order` response
  `51603 Order does not exist` maps to an observed missing order, but an ambiguous submit remains
  unknown for the bounded 60-second reconciliation window and is never blindly resubmitted.
  Official protocol: <https://www.okx.com/docs-v5/en/>.
- The pinned `hpx-yawc` compression transport is patched locally under `third_party/hpx-yawc` so
  incrementally read Ping, Pong and Close frames retain their RFC 6455 wire opcodes. Do not replace
  this with an invalid-opcode-to-Binary fallback: that hides the panic by breaking control-frame
  semantics and can silently kill an otherwise healthy compressed WS session.
- AppWS sends one snapshot replay when a channel is first subscribed. Repeating the same subscribe
  command is idempotent and returns only its ACK; it must not resend a full opportunity, portfolio or
  system snapshot. Recovery from a reported broadcast lag stays WS-native by sending the same
  subscribe command with `"replay": true`, which explicitly requests a fresh snapshot without any
  REST fallback.
- Review execution rows and 30-day strategy performance share one lifecycle-owned
  `ReviewRuntimeSnapshot`. Order, close-run and Portfolio-PnL projection activity is coalesced for
  100ms, then reads the realized ledger window once and derives both views from the same materialized
  trades. The lifecycle keeps a 30s recovery pass, publishes the batched `review` AppWS channel only
  when observed, and remains the replay/default-REST authority. A healthy frontend stream suppresses
  the former 10s executed and 30s performance polls; REST remains a freshness fallback and the only
  path for explicit history cursors or non-default day windows. The default frontend projection
  compares `generatedAtMs` before accepting a REST seed or recovery result, so a late older response
  or obsolete recovery error cannot overwrite a newer AppWS review snapshot.
- The arbitrage replay carries the exact first-page window for the combined P0 list and for each of
  the five product strategies, plus the complete deduplicated row union needed to render those
  windows. Normal frames publish only changed/removed rows inside that bounded union while retaining
  each window's exact order, cursor and count metadata. Row change detection covers every
  product-visible field except the projection timestamp, so Funding countdown, evidence, costs and
  blockers cannot remain stale; an otherwise unchanged frame does not advance the frontend table
  projection. The frontend owns one ID-indexed shared live-row projection and derives the selected
  strategy page directly from it; a healthy AppWS first page must never trigger a second REST
  calibration. REST list reads remain only for cursor/search requests and
  explicit WS recovery. During recovery, only the currently mounted product page owns the REST list
  poll; the global AppWS state never issues a duplicate one-row list probe. Opportunity first pages
  are never persisted to browser LocalStorage: a previous session cannot be relabelled as a fresh
  opportunity before the current AppWS replay arrives.
- Browser AppWS requests `encoding=zlib-json`. Data frames at least 8 KiB are zlib-compressed binary
  frames; smaller data frames plus ACK, error, ping and pong control messages remain JSON text. The
  codec is transport-only: it must not change channel cadence, payload fields, replay semantics or
  trigger a REST fallback. Legacy clients without the exact query value continue to receive text JSON.
- On-chain HTTP quote cadence follows provider limits. Solana exposes distinct Jupiter Keyless and
  Jupiter API Key routes; the keyed route fails closed without `JUPITER_API_KEY`, while the keyless
  route never sends the stored key. Jupiter Free-key paired sampling starts about every `2250ms`,
  keyless paired sampling every `4500ms`, 0x `1000ms`, OKX DEX trial-safe paired
  sampling `3000ms`, and CoW paired fast sampling `1000ms` (two requests per selected pair, below
  the official SDK public default of 5 RPS/IP). Jupiter's two directional `/order` requests remain
  spaced by `1000ms` with a Key or `2000ms` keyless, and cadence is measured start-to-start so HTTP
  latency is not added a second time. Custom EVM RPC evidence probes at `5000ms` and never enters the
  CEX 100ms projection hot path. The selected CEX spot-depth adapter is touched and projected from
  its persistent WS cache every `100ms`; an independent `5000ms` recovery worker owns REST
  bootstrap/gap recovery so slow CEX reads cannot hold completed on-chain quotes, and REST must retain its
  source label. AppWS `onchain` publishes source changes at no faster than the same `100ms` product
  batch.
- The on-chain watchlist is bounded to 12 observation-only items. A single provider request budget
  is shared by the focused pair and watchlist rows through fair round-robin scheduling; adding rows
  must increase the displayed estimated DEX sweep rather than create concurrent per-row pollers.
  The focused pair keeps a `100ms` CEX projection, bounded batch rows use `250ms`, and both read the
  same persistent exchange WS caches. Batch rows are included in the existing throttled `onchain`
  AppWS snapshot. Custom RPC URLs remain scoped to the focused configuration and are not copied into
  the watchlist.
- Binance partial depth keeps the official `depth20@100ms` market cadence, while subscribe and
  unsubscribe control frames batch symbols on a `500ms` flush with at most `1024` streams per frame.
  The control plane must stay below the official five incoming messages per second per connection;
  never restore one concurrent subscribe frame per symbol.

## 4. Verification Rule

For every completed item, record at least one of:

- code path evidence
- test command
- repo gate command
- official provider document URL
- runtime smoke output

Do not mark a Markdown item complete from intent alone.

Local verification has two paths only:

- `npm run finish` is the normal change-aware close. It detects the touched backend, frontend and
  documentation surfaces and runs each focused gate once.
- `npm run finish:release` is the single strong close before push or release. It runs workspace
  checks, current architecture/docs contracts, frontend release build and Wasm budget without replaying every
  historical PR completion script.

The exhaustive historical audit matrix has been removed from normal development. Target tests remain
appropriate while coding; do not chain them again with equivalent gates during the same close. UI
work receives one final affected-route browser pass after code and styles stabilize.

## 5. Active Scope

The newest explicit user request defines active scope. Deliver one coherent domain batch with code,
focused tests, affected operator documentation and one final runtime/browser verification when the
surface changed. The former audit, evidence ledger and coverage ledger are not part of the active
checkout or current verification.
