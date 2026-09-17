# CROSSLINE Exchange Transport Matrix

Checked against official documentation on **2026-08-06**. This matrix covers the
market, account, order and recovery operations exposed by CROSSLINE's
`ExchangeAdapter` and `LiveTradingAdapter` contracts. It is the operator-facing
transport contract; it is not a claim that every API offered by every venue is
part of this product.

## Transport Rules

- **WS stream** is the hot path for changing market/account/order state.
- **WS request** is preferred for supported point queries and trade commands.
- **REST seed/recovery** remains required when a WS channel has no initial
  snapshot, pagination, historical range, fee detail or authoritative metadata.
- A subscription ACK is not data evidence. A cache is usable only after a fresh,
  parseable venue event.
- A trade ACK is not finality. Order/fill streams and an identity-bound query
  decide the terminal state.
- A failed non-idempotent WS order request is never replayed automatically over
  REST. The same client order identity is reconciled first.

## Current API Families

| Venue | Current family used | Version finding |
|---|---|---|
| Binance | USD-M current Streams + WS API; Spot current Streams | Uses the 2026 `/public`, `/market`, `/private` split, `ws-fapi` request API, account V2 WS methods and REST account/position V3 fallbacks. No runtime `/fapi/v2` path remains. |
| OKX | API V5 | Public/private/business V5 WS and `/api/v5` REST only. |
| Bybit | V5 | V5 public/private/trade WS and `/v5` REST only. SBE/full-depth additions are optional, not replacements for supported JSON V5. |
| Bitget | UTA V3 | V3 public/private/trade WS and `/api/v3` REST. `/api/v2/public/time` is the current standalone clock endpoint, not a legacy trade API. |
| Gate | API V4.106 + Futures WS V4.0.0 | Current production V4 endpoints, current USDT testnet hosts and decimal-size WS handshake header. |
| KuCoin | Classic Futures/Spot plus Pro private WS | Perpetual production writes remain Classic REST. Spot place/cancel uses the official Pro private WS challenge flow, with Classic private WS terminal events and HF REST point queries. |
| Hyperliquid | Current `/ws`, WS `post`, `/info`, `/exchange` | Market/account subscriptions and WS post requests share the current production socket. |
| Kraken | Spot WebSocket v2 + Derivatives WebSocket v1 and REST v3 | Spot uses the recommended v2 socket for market/account/order commands. The official Derivatives socket is streaming-only, so futures writes and identity-bound point queries remain signed REST v3. |
| Gate CrossEx | CrossEx WebSocket v1.0.0 + CrossEx REST v1.0.2 | Uses the dedicated CrossEx public/private sockets and `/api/v4/crossex/*`; ordinary Gate Spot/Futures V4 channels are not CrossEx evidence. |

## Public Market Data

| Venue | Perpetual hot path | Spot hot path | Metadata / non-stream facts |
|---|---|---|---|
| Binance | `/public`: book ticker/depth; `/market`: ticker and `markPrice@1s` carrying mark, index, funding and next settlement | Spot ticker, book ticker and depth streams | REST `exchangeInfo`, funding interval, fee, index composition and clock; WS cache is first choice after its first event |
| OKX | `tickers`, `books`, `funding-rate`, `mark-price`, `index-tickers` | `tickers` + `books` | REST seed; `instruments` WS updates refresh the active instrument registry |
| Bybit | V5 linear `tickers` + `orderbook` | V5 spot `tickers` + `orderbook` | REST instruments, clock and index components; ticker carries funding/mark/index/OI fields where present |
| Bitget | UTA V3 `ticker` + `orderbook` | UTA V3 `ticker` + `orderbook` | REST instruments supplies funding interval and executable precision; index components remain REST |
| Gate | `futures.tickers`, `futures.book_ticker`, `futures.order_book_update` | `spot.tickers`, `spot.book_ticker`, `spot.order_book` | REST contracts, fee, index components and clock; WS uses `X-Gate-Size-Decimal: 1` where authenticated |
| KuCoin | Negotiated public WS ticker/orderbook/contract updates | Negotiated spot ticker/orderbook WS | REST bullet token, contract registry, fee and history; Classic WS is the production stream |
| Hyperliquid | `l2Book`, `activeAssetCtx` and WS `post/info` | Persistent `l2Book` plus WS `post/info` spot context | HTTP `/info` is a bounded fallback and metadata/history source, not the hot quote path |
| Kraken | Derivatives `ticker` carries BBO, mark, index, funding and next funding time; `book` carries sequenced snapshot/deltas | Spot v2 `ticker` with `event_trigger=bbo` and checksummed `book` snapshot/deltas | Spot v2 `instrument` is the live listing/precision authority; Derivatives REST `instruments` seeds contract size and static rules |
| Gate CrossEx | Dedicated `ticker`, `mark_price`, `funding_rate` and `order_book_update` channels with `*_FUTURE_*` route identity | Dedicated `ticker` and `order_book_update` channels with `*_SPOT_*` route identity | REST `/crossex/rule/symbols` supplies route, product and executable specification; signed `GET /crossex/market/funding_info` supplies the native funding interval; public subscriptions require explicit symbols and never subscribe all |

All ten adapter families implement the applicable product-facing WS snapshot surfaces:
perpetual ticker, funding, spot ticker and on-demand spot orderbook. Missing
symbols are backfilled individually; one late symbol cannot relabel a partial
WS cache as complete or force unrelated fresh symbols back to REST.

WS-only venue discovery is bootstrapped with a bounded BTC/ETH/SOL first-frame
probe. The probe carries Gate CrossEx's underlying-route identity, does not
subscribe depth, and cannot expand into an all-market subscription. Gate CrossEx
may stream the current rate and next settlement before credentials are present,
but its native interval remains unknown until the authenticated funding-info
metadata request succeeds; the runtime must not invent an interval.

Kraken's global funding scan is a separate path from that first-frame probe. It
uses one deduplicated full `ticker` subscription across execution-ready perpetuals
because the lighter `ticker_lite` feed omits funding, mark and index fields. This
can be a material byte stream at the venue's one-second cadence, but it does not
subscribe any order book; Kraken books remain candidate/preflight-only and expire
when no longer touched.

## Perpetual Accounts, Orders And Trades

| Venue | Account/position hot path | Order/fill hot path | Place / cancel | Point query and cold snapshot |
|---|---|---|---|---|
| Binance | `/private` `ACCOUNT_UPDATE`; WS API `v2.account.status`, `v2.account.balance`, `v2.account.position` | `/private` `ORDER_TRADE_UPDATE` | WS API `order.place` / `order.cancel` | WS `order.status`; `userDataStream.start/ping/stop` is WS-first; signed REST retains all-open-orders, history and recovery |
| OKX | private `account`, `positions` | private `orders` | private WS `order` / `cancel-order` | REST seeds/reconciles because the order channel has no cold snapshot; history remains REST |
| Bybit | private `wallet`, `position` | private `order`, `execution` | V5 trade WS `order.create` / `order.cancel` | REST wallet/position/order snapshots and history; WS streams remain authoritative for hot updates |
| Bitget | UTA V3 private `account`, `position` | private `order`, `fill` | V3 private WS place/cancel topics | REST assets/current-position/order-info/unfilled-orders for seed, page and recovery |
| Gate | `futures.balances`, `futures.positions` | `futures.orders`, `futures.usertrades` | `futures.order_place` / `futures.order_cancel` | WS `futures.order_status` and `futures.order_list`; REST account seed, fill history and recovery |
| KuCoin | Classic private wallet/position topics | Classic private `tradeOrders`; REST supplies fee detail | Signed Classic REST production writer | REST snapshots/query/history. Pro WS order/cancel is beta and remains fail-closed |
| Hyperliquid | account subscriptions plus WS `post/info` | `orderUpdates`, `userFills` | WS `post/order` / `post/cancel` | WS `post/info orderStatus` and account/spec reads; HTTP `/info` remains bounded fallback/history |
| Kraken | Derivatives private `balances` and `open_positions` | `open_orders` plus `fills`, authenticated by signed challenge | Signed Derivatives REST v3 writer; the official futures WS is streaming-only | Signed REST seeds/reconciles account, positions and orders and supplies history/funding payments |
| Gate CrossEx | private `asset` and `position` | private `order` and `usertrades` | `place_order` / `cancel_order` on the dedicated CrossEx private socket | CrossEx REST account/order endpoints seed, page and reconcile because private pushes are change streams |

## Spot Instruments, Orders And Trades

Instrument precision and listing state are static/slow-changing facts, so the official public
HTTP registry is authoritative even when place/cancel and terminal state use WS. Every write is
compiled from that ticket-bound Spot spec; a ticker symbol alone is never sufficient.

| Venue | Official Spot InstrumentSpec | Place / cancel | Private terminal hot path | Point query / recovery |
|---|---|---|---|---|
| Binance | REST `/api/v3/exchangeInfo` | WS API `order.place` / `order.cancel` | signed Spot user stream `executionReport` | WS `order.status`; REST history/pagination/recovery |
| OKX | REST `/api/v5/public/instruments?instType=SPOT` | private WS `order` / `cancel-order`, `tdMode=cash` | private `orders`, `instType=ANY` | REST `/api/v5/trade/order` plus cold/recovery reads |
| Bybit | REST `/v5/market/instruments-info?category=spot` | V5 trade WS `order.create` / `order.cancel` | private all-in-one `order` + `execution` | REST `/v5/order/realtime?category=spot` plus history/recovery |
| Bitget | REST `/api/v3/market/instruments?category=SPOT` | UTA V3 private WS place/cancel, `category=spot` | private UTA `order` + `fill` | UTA REST order-info/pending/history |
| Gate | REST `/api/v4/spot/currency_pairs` | Spot account-trade WS `spot.order_place/order_cancel` | private `spot.orders` | WS `spot.order_status`; REST history/recovery |
| KuCoin | REST `/api/v2/symbols` | Pro private WS `spot.order` / `spot.cancel` | Classic private `/spotMarket/tradeOrdersV2` | HF REST by order ID/clientOid plus fee/history recovery |
| Hyperliquid | `/info` `spotMeta` | WS `post/order` / `post/cancel` with Spot asset ID | `orderUpdates` + `userFills` | WS `post/info orderStatus`; HTTP `/info` bounded fallback |
| Kraken | Spot v2 `instrument` snapshot/update, with REST AssetPairs as bounded recovery | Spot v2 `add_order` / `cancel_order` | v2 `executions` plus `balances`, authenticated by REST-issued WebSockets token | Signed REST OpenOrders/QueryOrders/TradesHistory supplies cold snapshot, point query and history |
| Gate CrossEx | REST `/api/v4/crossex/rule/symbols` | CrossEx WS `place_order` / `cancel_order` | private `order` + `usertrades` | CrossEx REST order/account endpoints provide cold snapshot, query and recovery |

## Deliberate REST Boundaries

REST is not treated as a timer-driven substitute for live data. It remains for:

1. Initial snapshots when the official WS sends only deltas.
2. Paginated open-order or account reads that have no complete WS request.
3. Instrument precision, fee tiers, account mode, clock calibration and index
   composition when no equivalent complete production WS exists.
4. Funding-payment, fill and order history.
5. Bounded gap recovery after disconnect, sequence discontinuity or stale cache.
6. Credential permission probes that must return an explicit authenticated result.
7. Kraken Derivatives place/cancel/query, because Kraken explicitly documents its
   Derivatives WebSocket as streaming-only.

## Unsupported Or Evidence-Gated

- Spot Live remains fail-closed per ticket until the exact venue's official InstrumentSpec,
  credentials, balance/inventory, TradeWrite capability and private terminal stream are all ready.
  Implemented transport coverage does not turn missing runtime evidence into permission.
- **KuCoin Pro WS trade** is enabled only for Spot. Classic Futures REST remains
  the production perpetual writer; product identity prevents either route from crossing over.
- **SBE/FIX feeds** are optional low-latency transports with separate schemas,
  permissions or venue tiers. The current JSON WS APIs remain supported and are
  the production contract; SBE/FIX is not silently advertised as active.
- Private-stream stability cannot be proven without each user's enabled account
  permissions. Fixtures and parser tests prove schema handling, not live account
  authorization.
- Gate CrossEx is an execution route over an explicit underlying exchange, not an
  independent source of liquidity. Its native symbol and runtime evidence must retain
  that underlying exchange; CROSSLINE must not create an arbitrage pair between a direct
  venue leg and the same venue reached through CrossEx.
- Kraken uses `XBT` in several native products while CROSSLINE displays canonical `BTC`.
  Only official instrument metadata may perform that mapping; free-form suffix stripping
  cannot establish product identity or executable sizing.

## Official Sources

- Binance: [2026 stream split](https://developers.binance.com/en/docs/products/derivatives-trading-usds-futures/websocket-market-streams/Important-WebSocket-Change-Notice), [current USD-M trade WS catalog](https://developers.binance.com/en/docs/catalog/core-trading-derivatives-trading-usd-s-m-futures/api/ws-api/trade), [Spot WS trading requests](https://developers.binance.com/docs/binance-spot-api-docs/websocket-api/trading-requests), [Spot user data stream](https://developers.binance.com/docs/binance-spot-api-docs/user-data-stream)
- OKX: [API V5](https://www.okx.com/docs-v5/en/)
- Bybit: [V5 WebSocket](https://bybit-exchange.github.io/docs/v5/ws/connect), [V5 changelog](https://bybit-exchange.github.io/docs/changelog/v5)
- Bitget: [UTA WebSocket](https://www.bitget.com/api-doc/uta/websocket/Intro), [UTA changelog](https://www.bitget.com/api-doc/uta/changelog)
- Gate: [Futures WS V4](https://www.gate.com/docs/developers/futures/ws/en/), [Spot account-trade WS](https://www.gate.com/docs/developers/apiv4/ws/en/#spot-account-trade), [API V4](https://www.gate.com/docs/developers/apiv4/en/)
- KuCoin: [API changelog](https://www.kucoin.com/docs-new/change-log), [Pro Spot order](https://www.kucoin.com/docs-new/3470133w0), [Classic Spot order stream](https://www.kucoin.com/docs-new/3470073w0)
- Hyperliquid: [WebSocket](https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/websocket), [WS post requests](https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/websocket/post-requests), [Info endpoint](https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/info-endpoint)
- Kraken: [Spot WebSocket v2](https://docs.kraken.com/exchange/api-reference/spot-websocket-v2), [Spot instruments](https://docs.kraken.com/exchange/api-reference/spot-websocket-v2/instrument), [Spot book](https://docs.kraken.com/exchange/api-reference/spot-websocket-v2/book), [Derivatives WebSocket](https://docs.kraken.com/exchange/api-reference/futures-websocket), [Derivatives ticker](https://docs.kraken.com/exchange/api-reference/futures-websocket/ticker), [Derivatives REST](https://docs.kraken.com/exchange/api-reference/futures-rest-api)
- Gate CrossEx: [WebSocket v1.0.0](https://www.gate.com/docs/developers/crossex/ws/en/), [REST v1.0.2](https://www.gate.com/docs/developers/crossex/en/)
