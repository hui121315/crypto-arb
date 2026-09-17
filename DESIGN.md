# CROSSLINE Omni Design System

> Status: active design contract.
> Category: Fintech & Crypto / expert trading workstation.
> Method: continuous study of current expert trading products, direct browser evidence, production implementation, and repeated runtime review.
> Scope: frontend information architecture, visual language, interaction behavior, responsive behavior, and visual verification.

This is CROSSLINE Omni's own design system. Current expert trading products may be studied as
interaction and information-architecture references, but their branding, tokens, components,
wording, and product semantics are not production dependencies and must not be copied by default.

Authority is split deliberately:

- `AGENTS.md` and `docs/CROSSLINE_IMPLEMENTATION_GUIDE.md` own code shape and product semantics.
- This file owns visual intent, information hierarchy, interaction behavior, and design review.
- `frontend/styles/src/tokens.css` is the executable source for token values.

The completed whole-repository audit has been removed from the active checkout. Historical audit
evidence remains available through Git history and does not define current scope or completion.

When this file and `tokens.css` disagree, do not silently choose one. Reconcile both in the same
change before treating the design as complete.

## 1. Product and Operator Context

CROSSLINE Omni is a private, Chinese-first, low-latency arbitrage workstation for one expert
operator. The interface exists to help that operator understand current market and system state,
compare evidence, make a bounded decision, execute safely, and review the result.

Every screen should optimize for these outcomes, in order:

1. Detect what changed and whether the data is trustworthy.
2. Identify the best available action and the evidence behind it.
3. Understand risk, blockers, and irreversible consequences before acting.
4. Confirm what the system actually did, including partial or failed outcomes.

This is not a public exchange landing page, a consumer portfolio app, or a crypto casino. Avoid
marketing chrome, gamification, decorative token imagery, exaggerated profit cues, and visual
effects that compete with operational facts.

## 2. Visual Theme and Atmosphere

The visual language is a cold steel, blue-black control surface: calm, precise, dense, and quietly
authoritative. Content comes first and chrome comes second.

- Build hierarchy with a consistent cool luminance ladder, thin borders, spacing, and typography.
- Keep the canvas dark so long monitoring sessions remain comfortable and status colors stay clear.
- Use compact geometry with small radii; components should feel engineered rather than playful.
- Prefer flat surfaces and inset separators. Reserve elevation for overlays, menus, and transient UI.
- Avoid ornamental gradients, glassmorphism, neon glow, oversized shadows, 3D decoration, and
  continuous ambient animation.
- A dense screen must still have one obvious reading order. Density is not permission to flatten
  every fact to the same visual weight.

## 3. Color and Semantic Roles

Never invent a color in a component or skin file. Use semantic variables from
`frontend/styles/src/tokens.css`; new values belong there and must be named by role rather than by
the screen that first needs them.

| Role | Canonical token family | Intended use |
|---|---|---|
| Root and shell | `--color-bg-root`, `--color-bg-shell` | Application canvas and persistent shell |
| Tables and surfaces | `--color-bg-table`, `--color-bg-surface-*` | Data regions and task groupings |
| Separation | `--color-border-*` | Hierarchy, focus boundaries, and dividers |
| Primary text | `--color-text`, `--color-text-strong`, `--color-text-row` | Facts and active content |
| Secondary text | `--color-text-muted`, `--color-text-faint`, `--color-text-subtle` | Metadata and supporting context |
| Mint | `--color-accent` | Profit, healthy state, fresh real-time data, and safe primary selection |
| Blue | `--color-blue` | Neutral information, reference context, and non-risk emphasis |
| Amber and yellow | `--color-warning`, `--color-yellow` | Waiting, stale data, degraded state, and caution |
| Orange | `--color-orange` | Live execution mode and high-attention operational state |
| Coral red | `--color-danger*` | Loss, risk, failure, destructive action, and hard blocker |

Color is never the only carrier of meaning. Pair every status color with explicit Chinese text and,
where space permits, a stable symbol or shape. In particular, Paper and Live, fresh and stale, and
warning and blocked must remain distinguishable without color perception.

Mint means a positive or healthy fact; it must not make an unverified opportunity appear safe.
Orange identifies Live trading attention, while red is reserved for risk, failure, loss, and
destructive consequences.

## 4. Typography and Numeric Data

- Use Inter through `--font-sans` for navigation, labels, explanations, forms, and controls.
- Use JetBrains Mono through `--font-mono` for prices, amounts, rates, timestamps, IDs, and other
  values whose digits must stay aligned.
- Use tabular numerals and right alignment for comparable numeric columns.
- Use the existing compact scale: 11px metadata, 12px controls, 13px body/table content, 15px
  emphasis, 18–20px section headings, and roughly 22px KPI values.
- Keep Chinese copy primary. Preserve industry abbreviations only when they are standard for the
  expert operator; explain ambiguous abbreviations in adjacent copy or a tooltip.
- Prefer short, concrete labels and explicit verbs. Avoid slogans and vague labels such as “智能”,
  “增强”, or “优化” when a precise operational term exists.
- Never truncate a risk blocker, failed preflight reason, or execution finality message without an
  accessible way to read the full text.

Numeric formatting must preserve the precision needed for the decision. Do not trade away material
digits merely to make a column narrower, and do not present unknown values as zero.

## 5. Layout, Density, and Reading Order

The workstation is desktop-first, but every critical read and action must remain usable on narrow
screens.

The default reading order is:

1. Persistent navigation and system/data status.
2. Module title and concise operational context.
3. Critical KPIs or blockers.
4. Primary table, workflow, or decision surface.
5. Selected-row detail, evidence, and scoped actions.
6. Secondary history or explanatory material.

Use the existing 4/8/16px spacing rhythm and compact 4/6/8px radii. Group content by operator task,
not by backend object. Avoid “card soup”: a border is warranted only when the region has its own
heading, state, selection, or action boundary.

Execution-critical comparison tables keep the full task width whenever their row relationships
drive a decision. Portfolio risk summaries sit above the positions table as a compact command band;
they do not occupy a permanent side rail that compresses prices, liquidation data, or close actions.
Detailed evidence and destructive controls remain in their scoped panels below the primary table.

Current responsive contracts are part of the design system:

- Above 720px, keep the eight modules in one 56px horizontal application bar. No desktop width uses
  a permanent left navigation rail; the full content width belongs to the active trading task.
- At 1200px and below, allow the horizontal navigation to scroll locally when labels cannot fit and
  collapse multi-column task regions without changing their semantic order.
- Below 1100px, move persistent detail or evidence rails below the primary task surface.
- At 720px and below, show all eight modules in a compact two-row navigation grid and keep tables
  internally scrollable with sticky headers and sticky action columns.
- At 480px and below, remove secondary copy before removing status or action meaning.

The application page itself never scrolls horizontally. Wide comparisons, evidence ledgers, and
tables own their local scrolling. Every grid or flex child that contains variable data uses
`min-width: 0`, and every flexible track uses `minmax(0, ...)` or an equivalent constraint.

Do not convert a comparison table into unrelated cards when row and column relationships are the
core decision aid. Tables over 100 rows require self-rolled virtualization.

## 6. Components and State Coverage

### Surfaces and cards

Use surfaces to express task grouping and state, not decoration. Default cards are flat, bordered,
and compact. A raised surface is appropriate only for a menu, modal, popover, or temporary alert.

### Tables and lists

- Keep headers sticky, units explicit, and comparable values aligned.
- Make row selection visibly different from hover.
- Only use a pointer cursor when the row is actually interactive.
- Keep the primary fact on the first line and provenance, freshness, or qualification on the second.
- Place the reason for a disabled action beside the action instead of hiding it in a remote panel.

### Buttons and forms

- Button labels state the operation: “构建对冲”, “暂停自动化”, or “关闭全部仓位”, not “确定”.
- Safe primary selection may use mint. Live-mode controls use orange. Destructive actions use red.
- Disabled controls remain legible and include the blocking reason.
- Focus, validation, submitting, success, partial success, and failure are visually distinct.
- A Live write path must retain the repository's explicit unlock, preflight, finality, and
  compensation semantics; styling cannot weaken those gates.

### Charts and icons

Hand-roll focused SVG charts and icons. Do not add a chart or icon dependency. Charts must include
units, time range, source/freshness, and honest empty/error states. Decorative charts and anonymous
sparklines without a decision purpose do not belong in the workstation.

### Required data states

Every data-bearing surface must deliberately cover loading, empty, fresh, stale, degraded, error,
blocked, and unavailable states when those states can occur. Empty, unavailable, and zero are three
different meanings. Observation-only and executable are also different meanings.

## 7. Interaction, Feedback, and Motion

- Feedback should be immediate, but trading success is shown only after the established finality
  contract confirms it. Never use optimistic success for an external write.
- Keep values and controls spatially stable during refresh. Tabular numerals prevent digit jitter;
  reserved regions prevent status text from moving adjacent actions.
- Use the existing 120ms color, border, and opacity transitions. Avoid layout, size, blur, shadow,
  and continuous looping animation.
- Honor `prefers-reduced-motion` and keep all workflows complete when motion is absent.
- Preserve a visible keyboard focus ring and a logical focus order.
- Use toasts for transient confirmation, not for the only copy of a blocker or failure. Persistent
  consequences stay near the affected task.
- Module navigation is idempotent: one real route change produces one active-module transition.
  Hash and history synchronization must not remount the current task or duplicate its requests.
- Route-owned timers and request gates stop reading reactive state after unmount. A normal module
  switch must complete without disposed-signal panics or background callbacks mutating the old task.

## 8. Responsive and Accessible Behavior

- Target WCAG AA contrast for text and essential controls, then verify it against the actual token
  pair rather than assuming that a semantic name is sufficient.
- Do not encode status, trend, or venue identity through color alone.
- Use semantic tables, headings, labels, and button elements before adding ARIA.
- Give meaningful SVGs accessible names; mark decorative SVGs as hidden from assistive technology.
- Keep desktop controls compact, but ensure critical mobile actions have a touch target of at least
  44px in one dimension or equivalent surrounding hit area.
- Horizontal scrolling is acceptable for a dense comparison table when the header remains visible
  and the first identifying column stays understandable.
- Keyboard access is mandatory for tabs, row actions, menus, dialogs, and execution controls.

## 9. Data Honesty and Operational Safety

The interface must expose uncertainty instead of smoothing it away.

- Show source, freshness, problem, and retry information where the underlying contract provides it.
- Show unknown as unknown; do not substitute zero, a dash with ambiguous meaning, or a fabricated
  estimate.
- Mark observation-only data explicitly and omit execution affordances that are not supported.
- Do not invent legs, prices, funding, fees, APR, breakeven, or support status for visual
  completeness.
- Differentiate Paper and Live with text, color, and guard behavior on every consequential surface.
- A mixed order history keeps each record's own Paper or Live environment in the primary scan line;
  the current shell environment must never relabel or recolor historical orders.
- Review surfaces use “confirmed” only for terminally evidenced results. A readable ledger proves
  data availability, not realized PnL; confirmed, estimated, and unavailable totals remain separate.
- Review result height follows the amount of evidence. Empty and short task results use their real
  content height; larger execution and venue ledgers retain the bounded full-height workspace and
  local scrolling. Switching tasks must not turn one empty row into a viewport-sized blank frame.
- An execution workspace without a current ticket uses one full-width outcome surface for the next
  action and read-only history; it does not reserve a second empty ticket surface beside old results.
- Long-lived automation and execution history shows a complete local date and time. Clock-only
  labels are reserved for live freshness and heartbeat context where the date is already implicit.
- A desktop task surface with its own bounded vertical scroll must not also create a few pixels of
  page-level tail scroll. Narrow layouts may return to document scrolling when the inner bound is removed.
- Stable configuration catalogs remain visible during a same-scope background refresh and dedupe
  concurrent identical reads. A failed refresh marks the retained catalog stale and blocks apply;
  it never keeps a previous fresh presentation.
- Keep risk and preflight blockers adjacent to the action they govern.
- Prototype fixtures must be visibly labeled as fixtures and must not be copied into production data
  paths.

Product and external-system wording must come from shared DTOs, existing repository specs/tests,
explicit user instruction, inspected live responses, or official provider documentation.

## 10. Frontend Implementation Mapping

Design artifacts are directions, not a second production architecture. Production implementation
uses the repository's established paths:

- Shared semantic values: `frontend/styles/src/tokens.css`.
- Cross-module component recipes: `frontend/styles/src/skin/surfaces.css` and other shared skin
  files already listed in `frontend/styles/src/manifest.txt`.
- Module-specific presentation: `frontend/styles/src/skin/<module>.css`, scoped under the module root.
- Data acquisition and state: `frontend/src/panels/modules/<module>/data.rs`.
- Top-level assembly: `view.rs`; focused rendering: `components/*.rs`.
- Backend-shaped types: `shared-types/src/`; never duplicate them under `frontend/src/`.

New CSS uses `var(--*)`; hex literals stay in `tokens.css`. New `#[component]` code does not fetch,
open sockets, or own timers directly. Keep `view!` depth and child counts within the project limits,
and preserve the repository's polling and WS cadence contracts.

## 11. Continuous Product Refinement Workflow

Use this loop for every new module, substantial workflow, or material visual change. The active
research and execution ledger is `docs/HYPERLIQUID_PRODUCT_DESIGN_REFINEMENT.md`.

1. **Observe** — inspect the current official Hyperliquid product for the nearest comparable task.
   Record date, route, viewport, visible hierarchy, control geometry, interaction sequence, and
   screenshot evidence because the external product can change.
2. **Distill** — state what operator problem the observed behavior solves, what principle transfers
   to CROSSLINE, and what must not transfer because the product semantics differ.
3. **Audit** — inspect the real CROSSLINE route with realistic runtime states. Record information
   duplication, action distance, ambiguous status, unstable layout, overflow, empty space, and
   responsive failures in the active refinement ledger.
4. **Reshape** — define the target reading order, primary action, state matrix, density, and
   responsive behavior. Large information-architecture and component changes are allowed when they
   shorten the operator's decision path without weakening product contracts.
5. **Implement** — change the real Leptos modules, shared DTOs when semantics require it, data hooks,
   tokens, and scoped skin files. A separate prototype is optional and is never an authority input.
6. **Verify** — scale verification to the changed surface. Layout, CSS, responsive, or interaction-
   geometry changes exercise the built route at 1536, 1100, 720, and 480 widths. A data-contract,
   copy, or semantic-state change that leaves DOM structure and geometry intact uses one
   representative affected-route pass, plus one narrow pass only when the new content can change
   wrapping. In either case inspect the exact changed state, browser console, and local overflow;
   do not replay unrelated routes or unchanged viewports.
7. **Record and repeat** — update the ledger with concrete evidence and remaining issues, then begin
   the next highest-value iteration. A visual direction remains open to further refinement after a
   passing screenshot round.

For a small local visual defect, before/after screenshots may be sufficient evidence. For a
semantic-only correction, the typed contract, one focused test when the distinction is non-trivial,
and one real affected-route observation are sufficient. Reading this contract, updating the
relevant ledger item, and verifying the built result are still required.

### 11.1 Deterministic Opportunity Workflow

This section preserves the accepted product contract for opportunity, automation, Webhook, and
hedge execution. It remains subject to the continuous refinement loop above, but visual changes may
not weaken these behavioral guarantees.

- Opportunity scanning keeps the comparison table primary. The current selected-candidate judgment
  sits between the KPI strip and candidate surface; the Webhook delivery disclosure follows the
  candidate result so queue and acknowledgement evidence remain available without delaying the
  first comparison row.
- Futures and opportunity KPI facts stay in compact comparison strips instead of stacked cards.
  On narrow screens, execution-eligibility categories remain one locally scrollable row with the
  current result count fixed beside it; neither filter density nor long blocker labels may create
  document-level horizontal scrolling.
- On narrow opportunity layouts, strategy scope remains one locally scrollable row, search and the
  percentage net-profit threshold share one row, and the current judgment remains a single compact
  line. Full candidate evidence, workflow stages and Webhook delivery history stay available on
  demand below the primary comparison.
- Automation uses one compact control rail and one dominant runtime column. Low-frequency entry and
  exit configuration is disclosed on demand; runtime state, the latest deterministic artifact and
  the decision ledger remain visible.
- Automation keeps control and current runtime ahead of Webhook delivery details. When automation is
  disabled and has neither a current artifact nor a current decision, one compact waiting state
  replaces repeated artifact and decision empty blocks; guard configuration and historical tasks
  remain available without implying that monitoring is active.
- Automation keeps the configured cooldown duration separate from its current timer. Disabled,
  waiting, actively counting down, expired, and missing-timer states use distinct labels; the
  absence of an active countdown must not be presented as if cooldown protection were unconfigured.
- Automation lifecycle state is not a candidate decision. While disabled, the current runtime does
  not present a disabled heartbeat as “no eligible candidate” or surface a historical artifact as
  current; the disabled heartbeat is labelled as a closed lifecycle event in the separate,
  read-only decision history.
- Automation uses three mutually exclusive local tasks: current runtime, read-only decision
  history, and the current seven-stage loop. The current loop may reuse only the current decision's
  artifact and the Webhook delivery whose event ID matches that artifact; prior artifacts and
  deliveries remain history and cannot advance current-stage presentation.
- When automation is disabled, current runtime, sparse decision history and the idle seven-stage
  loop use their real content height instead of stretching empty frames to the viewport. Running,
  paused and active-run states retain the bounded full-height workspace needed for live updates.
- Decision history grows with its rows and becomes a local scroll surface only after it reaches the
  workspace height cap; one or two lifecycle rows must not create a large empty ledger frame.
- Hedge execution places the snapshot-bound artifact between preview evidence and the final action.
  Copy, revalidate and submit are separate controls. Copy emits a read-only validation command;
  revalidation can expire or block the artifact; submit remains disabled until the operator reviews
  a currently ready artifact.
- When hedge execution has no current ticket, its deterministic path exposes the only actionable
  candidate-source row. The order/result surface remains reserved for current or previous outcomes
  and must not repeat the same empty instruction or a second three-stage placeholder; historical
  outcomes stay explicitly read-only. Sparse history uses its real content height, while a long
  order ledger becomes a bounded local scroll surface instead of extending the document.
- Paper/Shadow is the visible default. Live automation is labelled as automatic, has no separate
  unlock control, and submits both legs only after artifact revalidation and every shared risk gate
  pass. Manual hedge execution remains a separate operator-confirmed workflow. No visual treatment
  may imply that a qualified Live artifact was submitted before order finality proves it.
- Thresholds exposed to the operator use percentages. Basis points remain an internal DTO and
  calculation unit only.
- The three workflow surfaces reuse one seven-stage rail: qualification, Webhook, artifact
  revalidation, dual-leg submission, ACK/finality, protected exit, and review. Each stage supports
  idle, current, complete, warning, and blocked semantics without implying progress that its page
  cannot prove.
- Webhook records distinguish queued, retrying, application-accepted, failed, and dropped outcomes.
  HTTP success alone is not a green Bark state. Artifact states distinguish ready, revalidating,
  expired, tampered, missing, and consumed; an existing `ExecutionRun` is the consumed evidence.
- A configured and subscribed Webhook is labelled as a ready delivery channel, not as proof that
  automation is running or that a current event was delivered. Narrow layouts keep this readiness
  text visible below the channel title instead of hiding it.
- The rail wraps to four and then two columns at narrower widths. It must never create a horizontal
  scroll dependency or displace the primary comparison, runtime, or execution controls.

### 11.2 Hyperliquid-Informed CROSSLINE Workstation V3

The current shell, control geometry, and cross-module hierarchy are maintained directly in this
contract and `docs/HYPERLIQUID_PRODUCT_DESIGN_REFINEMENT.md`. Hyperliquid is studied only as a
current interaction, density, and task-flow reference; CROSSLINE retains its own name, semantic
colors, data states, workflows, and trading safeguards. Visual resemblance is not an acceptance
criterion; faster comprehension, shorter safe operation paths, and honest state feedback are.

- The application shell is a 56px horizontal module bar followed by one 34px expandable health
  summary. It has no permanent left rail at any desktop width.
- The expanded health summary keeps the compact Connectivity, Activity and Risk facts first, then
  triages every existing `SystemHealth.problems` item into action blockers, market data,
  configuration and permission, transport performance, pending evidence, and remaining runtime
  issues. The compact summary exposes the live count for each non-empty category; the native
  keyboard-readable ledger retains every original row. A hover `title` may preview evidence but
  must never be its only readable surface; long messages scroll inside the ledger without adding
  requests or document-level overflow.
- Buttons and fields are 32–34px high on desktop with 6–8px radii. At 480px, task controls use at
  least 44px touch targets while dense table actions remain locally constrained.
- The canvas is a continuous, flat trading workspace. Dividers, task headers and local scroll
  regions establish hierarchy; nested floating panels, decorative shadows and oversized headings
  are not part of the system.
- A module begins with one compact task title, not a hero or static feature description. Current
  mode, state, evidence and next action belong in the first real task surface. A nested surface must
  use its own task name instead of repeating the module title.
- Comparison tables retain sticky headers and local horizontal scrolling. Identity and action
  columns remain visible where the row requires an operator decision; the document itself never
  scrolls horizontally.
- Compact row metadata remains self-describing. A domain risk value is rendered as `Risk Low`,
  `Risk Medium`, `Risk High`, or `Risk Unknown`, never as an isolated `Low` that can be mistaken for
  a profitability score or execution confidence. Profitability and execution eligibility remain
  separate evidence-backed fields.
- Arbitrage candidate freshness, opportunity-stream transport, and each leg's market-data
  freshness are separate scopes. A stale candidate scan must not imply that every WS quote is
  stale, and a fresh leg quote must not disguise an old candidate set.
- Candidate snapshot freshness follows the backend's bounded measured refresh window: the previous
  scan duration, the scheduled refresh interval and a capped jitter allowance. A normal long scan is
  labelled as refreshing with its retained snapshot age; only data beyond that measured window is
  stale. Sparse Futures result sets collapse to their real content height instead of stretching a
  single row or empty message into a full-canvas table frame.
- Dense candidate rows keep price, native Funding, settlement timing and the compact source/age of
  each leg in the primary comparison. Fully received healthy coverage is not repeated as `1/1
  (100%)` on every row; the exact quality, source, freshness and coverage remain in the row's
  evidence disclosure. Cached, partial, missing or failed evidence stays explicit in the row.
- Opportunity Scan's current judgment binds the selected market, route, realizability and first
  blocker in one neutral statement. Only an execution-eligible candidate may call its fee-after
  result net profit; an observation-only candidate labels the same mathematics as an estimated
  edge and says that it is not realizable. The table and selected detail preserve that distinction.
  Its primary evidence action focuses the same stable detail panel directly; anchored targets
  reserve the fixed shell height so the destination is not obscured on narrow screens.
- Opportunity strategy tabs are server-backed result scopes, not client-only filters over the
  currently loaded page. Changing strategy resets pagination and requests that strategy's first
  page; symbol search preserves the same scope, and late responses from an older strategy or cursor
  cannot replace the current result. Local strategy matching remains a defensive presentation
  check, never the source of pagination truth.
- A candidate's realization label follows its actual strategy contract. Funding strategies may use
  a real settlement countdown; spot cross waits for both order finalities, and perpetual price
  spread waits for spread convergence and finality. A strategy without a timed settlement event
  must never display `0s` as if value had already been realized.
- At 1100px and above, Opportunity Scan is one viewport-bounded result workspace. Its three market facts
  share one compact status row with the current decision, while the candidate table and the 300px
  evidence rail own independent local scrolling. Webhook delivery follows the selected evidence in
  that context rail instead of creating a second page tail. Below 1100px, evidence and
  delivery return to document order after the comparison without changing selection or actions.
- At 720px and below, the sticky futures action cell repeats the fee-after net edge beside the
  action state so profitability remains visible without horizontal scrolling. Observation-only or
  unverified values remain neutral even when mathematically positive.
- A perpetual candidate keeps each leg's native Funding rate, native interval, and next-settlement
  countdown together with that leg's venue and price on the first comparison layer. A combined
  edge never replaces these leg facts, one-hour rates are never presented as eight-hour normalized
  rates, and missing or elapsed settlement evidence remains explicit rather than becoming `0m`.
  Because long and short positions reverse the cash-flow meaning of the same signed Funding rate,
  each leg also states its expected receive, pay, flat, or unknown outcome without replacing the
  native signed rate. This cash-flow label is evidence, not execution eligibility.
- Expanded Futures evidence repeats the candidate's current qualification and first backend blocker
  beside its identity. The panel must remain understandable after the originating row scrolls away;
  the first layer keeps strategy, execution condition, gross/cost/net breakdown, cost evidence,
  snapshot freshness and both leg evidence. Depth, timing, composition and other strategy details
  remain in one keyboard-readable complete-evidence disclosure; a positive edge never replaces an
  observation-only decision.
- Futures compact blocker labels derive from the exact first backend blocker and stay identical in
  the eligibility filter and row action. Funding evidence, spread-convergence evidence, exit or
  holding rules, market data, instrument registry, identity, cost, depth, and account readiness are
  distinct categories; a generic blocker label is reserved for genuinely unknown reasons. The full
  backend text remains available in the row title and expanded evidence.
- At 1100px and above, Futures is one viewport-bounded comparison workspace. Strategy, scoped filters and
  execution-eligibility controls stay stable while the candidate table and an expanded evidence row
  share one local scroll region; pagination remains visible. Sparse, empty and full pages keep the
  same result-plane height so live row-count changes do not collapse the workspace. Below 1100px,
  the same tasks return to natural document flow without changing the selected strategy, evidence
  or build action.
- At 720px and below, Futures keeps its five strategy tasks in one locally scrollable tab row with
  roving keyboard focus. Search and the active strategy's profit threshold share the next row; an
  inactive Reset command does not reserve another row, but appears when a local constraint exists.
  The first candidate row should enter the initial narrow viewport without removing profitability,
  execution eligibility, feed freshness, or blocker meaning.
- Positions uses one full-width result plane with six mutually exclusive tasks: Positions, Assets,
  Risk, Close, Access and Controls. Positions is the default task and owns the comparison table plus
  its direct reduce-only close action; the other tasks replace that result body instead of squeezing
  it beside or below a permanently mounted table. Execution may use a 300–320px contextual rail when
  a current ticket needs persistent evidence; On-chain uses a responsive 320–356px configuration
  rail so contract identities remain readable, and automation uses a 292px rail.
- Above 1100px, Positions is one viewport-bounded account workspace. Its compact live header,
  account summary and inline risk boundary stay above the selected task; only the selected task owns
  the remaining result plane and local scrolling. At 1100px and below, the same hierarchy returns to
  natural document flow. At 480px the six short task labels remain directly visible in one row, the
  close action remains visible without document-level horizontal scrolling, and an empty task uses
  an intentional centered state rather than an orphaned label or decorative filler.
- The Positions Balance/NAV task pairs account-equity history with one selected venue account.
  Venue accounts use one compact selector inside that task; the selected account and NAV history
  occupy adjacent result regions on wide screens and stack in the same order on narrow screens.
  Account selection must not stretch sparse details or the neighboring chart.
- Positions margin utilization is venue account initial margin divided by venue account equity.
  Maintenance margin remains a separate liquidation-risk fact and must never be used as the
  utilization numerator. When account-level evidence is unavailable, a position-scope fallback is
  labelled as estimated; an uncomputable ratio remains unknown rather than zero.
- Account NAV and its historical change remain visually neutral. A NAV interval change is labelled
  explicitly as account-value movement, never as PnL, because deposits and withdrawals can move
  NAV without representing trading profit or loss.
- A 24-hour NAV change is present only when a real sample at least 24 hours old exists. Until then,
  the summary says that the 24-hour baseline is pending evidence; current NAV must never be reused
  as the historical baseline to manufacture a zero-percent change. A shorter NAV-history interval
  may still show its own explicitly labelled change without being promoted to 24 hours.
- The Positions NAV history request covers the full retained account-equity window needed by the
  24-hour summary. A client row limit must not hide the older baseline sample while the summary
  still reports a valid 24-hour change; chart range, sample count and summary evidence must describe
  the same retained window.
- The Positions summary wrapper always spans the full task width. Its four facts own their
  responsive four-to-two-column grid; the wrapper itself must never become a two-column KPI grid
  that leaves an empty half-canvas at narrow widths.
- Positions keeps the current task ahead of repeated diagnostics on narrow screens. Transport and
  account coverage share one compact context row; runtime degradation and a stale summary snapshot
  each use a one-line native disclosure. Their complete typed problem, source, path, retry, and
  previous-snapshot explanation remain readable when expanded, but neither becomes an extra KPI
  card or a scrolling chip rail that pushes the position table out of the first viewport.
- A position comparison row keeps identity, size, valuation, PnL, liquidation, Funding, pairing and
  its primary action geometrically stable. Row evidence opens as one full-width, keyboard-linked
  subtask with visible source/problem/time context; it is mutually exclusive with destructive close
  confirmation, and hover text is never the only evidence surface.
- A direct position close opens one row-linked confirmation before any request is dispatched. The
  confirmation names Live or Paper, the exact single-leg or paired scope, mark price, current or
  matched notional, unrealized PnL, liquidation distance, reduce-only market behavior and the
  exchange-terminal completion boundary. Cancel receives initial focus; Cancel and Escape both
  return focus to the originating close command. At 720px and 480px the complete decision facts and
  both actions remain inside the visible table width without document-level horizontal scrolling.
- Position Funding combines the next real settlement window, the position-side cash-flow direction,
  and the original signed rate in one compact fact. A verified positive rate means a long will pay
  and a short will receive; a verified negative rate reverses that direction. Zero is flat and an
  unverified rate remains unknown. The cash-flow label predicts the next funding transfer only; it
  must not be presented as an already realized payment. At 480px the hidden desktop Funding column
  returns as one full-width compact fact below the two-column position risk summary; it must not
  disappear merely because the table drops secondary desktop columns.
- The compact risk summary exposes Risk Evidence and Advanced Controls as sibling contextual
  destinations. Each destination selects, reveals, scrolls to, and focuses its stable task panel;
  narrow tab rails keep the selected task label visible. Kill switch and bulk-close commands remain
  inside Advanced Controls behind their existing explicit confirmation contracts.
- Positions Risk uses four stable evidence domains in this order: risk limits, Funding settlement
  window, Delta concentration and margin utilization. They form four equal desktop columns, a 2x2
  grid at 720px, and one natural column at 480px; a fifth orphan card or implicit empty grid track is
  not allowed. Unknown account evidence remains unknown or explicitly estimated rather than zero.
- Positions Controls presents the kill switch, bulk close and paired-exit protection as three equal
  decision units in that order. Every unit keeps current state, affected scope, operational effect
  and its single command together; it must not collapse multiple destructive controls into one
  unlabeled strip or hide consequences in hover text. At 1100px the three units remain aligned for
  comparison, while 720px and 480px preserve the same order in natural vertical flow.
- Pair protection is active only when current rows contain complete mutual pair evidence. A single
  leg or incomplete pair is labelled as unpaired and unprotected even when future exit thresholds
  are configured; missing liquidation evidence remains distinct from missing pair evidence.
- A position-to-opportunity link must distinguish discovering an independent two-leg strategy from
  adopting or completing the current position. Route context preserves the source symbol, the
  destination states that a build opens a new complete pair, and no discovery action may imply that
  the original exposure has been hedged.
- A bulk close control names the current loaded position scope before the input. No-position,
  incomplete-phrase, ready and pending states remain distinct, and the destructive submit stays
  disabled until the exact confirmation phrase is present. Its ready label states the exact number
  of loaded positions to close, and the visible terminal boundary remains reduce-only market close
  confirmed by the exchange; the backend confirmation contract is still authoritative.
- The execution ticket contains only the selected opportunity and its matching current run.
  Previous orders and run evidence stay together in the order rail as a labelled read-only result;
  a settled historical run must not reappear below the page or auto-expand inside a new ticket.
- The order rail leads with business scope: market, long venue, short venue, and update time. Raw
  run and ticket identifiers stay in a keyboard-readable disclosure. Current execution, previous
  execution, and recent orders without an `ExecutionRun` use distinct headings and empty states;
  an independent route load must never promote the latest order state into a current run.
- Historical execution runs, order rows, fill confirmations and timeline events always show local
  calendar date and time. Current transport heartbeats may remain `HH:mm`; a time-only historical
  label must never make an older record look like a future event on the current day.
- The on-chain rail exposes one local task at a time through Market, Node and Alert tabs with roving
  keyboard focus. Its permanent read-only boundary and apply/refresh/batch command footer remain
  visible while the selected task owns the only internal scroll region; below 901px the same
  controls return to normal document flow without overlap.
- When on-chain monitoring is disabled, the command rail, current comparison, source evidence,
  empty batch state and quote-evidence disclosure use their real content height. The inactive page
  must not reserve a full-height live-result canvas. Enabling monitoring restores the bounded live
  workspace so source cadence and batch rows remain geometrically stable while they update.
- On-chain user-facing chain names come from the shared chain preset catalog, and CEX source states
  use the product source vocabulary. Internal IDs such as `solana` and `not_started` remain contract
  values and must not leak into the visible command rail, decision board, telemetry, batch table or
  evidence summary.
- The on-chain command footer follows the actual dependency order: apply the draft first, then
  enable or disable monitoring. Refresh and batch commands appear only while monitoring is enabled;
  the disabled state does not reserve space for actions that cannot run. The applied comparison
  names one next step and does not repeat the disabled explanation in a second status strip.
- The on-chain batch summary treats queue occupancy, sample availability, runtime anomalies and the
  schedule estimate as separate facts. An empty queue may truthfully show `0/12`, but its result and
  timing states read as no sample and not running. Its setup guidance shares the same compact summary
  row rather than creating another empty-result surface; items waiting for their first sample remain
  pending. A numeric zero anomaly or duration is shown only when the corresponding runtime fact has
  actually been observed or calculated.
- Enabling an on-chain comparison enters an explicit first-snapshot pending state until the first
  quote pair and CEX order book can be compared. Pending is neutral and must not render as upstream
  failure, zero edge, or a connected HTTP source. Source telemetry names the quote and order book it
  is still waiting for, then changes to observed source and freshness only after real evidence arrives.
- On-chain source telemetry contains only sources required by the applied chain. Solana and other
  non-EVM routes do not reserve an EVM RPC card or cadence; EVM routes retain quote, RPC and CEX WS
  evidence as separate facts. Removing an irrelevant source never removes its configuration task
  from a chain where that source is actually required.
- Review uses one continuous 30-day result workspace. Its Executed, Missed, Strategy and Venue
  Quality task tabs expose each source's current row/sample count and compact evidence state before
  selection. A loaded empty task remains selected and truthful, but offers one count-backed action
  to the highest-priority non-empty result; it never switches automatically, and an explicit switch
  returns focus to the target task tab. Full source, degradation and error evidence stays in the
  selected task disclosure rather than being flattened into the compact tab.
- On a loaded empty Review task, the selected source disclosure and count-backed path to another
  available result share one compact context row. The honest business empty state remains the only
  result body; evidence, keyboard focus, and explicit user-directed switching are preserved without
  stacking a second action surface above it.
- An Executed Review timeline keeps event type, venue, side, amount or price, evidence quality and
  source on its first layer. Repeated event, run and ticket identifiers belong to the existing
  technical disclosure; adapter acknowledgement is labelled as an estimate, never as confirmed fill
  finality. Moving identifiers must not remove or rewrite the underlying ledger evidence.
- Above 1100px, Review is one viewport-bounded result workspace: task tabs, source context and the
  active result pager remain stable while the active result table owns the only vertical scrolling.
  At 1100px and below, Review returns to natural document flow so narrow layouts do not trap scrolling.
- Settings keeps the current execution mode, its consequence, and the switch action ahead of
  reference material. The Paper/Live comparison remains directly readable, while the full venue
  capability matrix defaults to a secondary disclosure whose summary exposes coverage and current
  problems. Static capability never implies ticket-level submit readiness.
- Exchange credential editing keeps one selected venue bound to one save result. Venue switching,
  evidence refresh, save, clear, and migration cannot overlap while a mutation is pending; refreshing
  evidence must not erase an in-flight or terminal action state. The first result layer names the
  venue and distinguishes not submitted, pending, accepted, succeeded, and failed. Request, action-run,
  and idempotency identifiers stay in a keyboard-readable technical-evidence disclosure.
- A credential result that has not been submitted remains a compact venue-bound status row rather
  than a full result panel. Pending, accepted, succeeded and failed states expand to the full stable
  result with explanatory copy and technical evidence; compactness must never erase the action state.
- Wide Settings ledgers may retain local horizontal scrolling for complete operational evidence, but
  the terminal row action stays sticky at the trailing edge and remains directly reachable without
  dragging the table. Opening that action returns to a natural single-column detail flow on narrow
  screens; the document itself must never acquire horizontal overflow.
- Credential inputs never reveal saved Secret values. An empty draft means “keep the stored value”,
  not a pending update: the save action stays neutral and disabled until at least one non-empty field
  changes, names the number of fields to update, and clears the local Secret draft after success.
- At 720px the module navigation becomes a two-row, four-column grid beneath the brand row. Status,
  blocker and action meaning remain visible; provenance copy may compact before a required control.
- Fresh, loading, unknown, stale, degraded, error, blocked, observation-only, Paper, Live and
  destructive states remain distinct in text and color. Unknown never renders as zero, and
  observation-only rows never expose an executable action.
- Signed monetary values choose display precision before applying direction. Exact zero carries no
  positive or negative sign, real sub-dollar values retain enough decimals to remain non-zero, and
  non-finite input renders as unknown; `-$0` and `+$0` are not valid operator-facing facts.
- Production verification requires screenshots of all eight modules at 1536, 1100 and 480 widths,
  plus affected-route verification at 720 width, no document-level horizontal overflow, no topbar
  overlap, a clean independent route load, and a completed interaction pass for changed workflows.

## 12. Do and Do Not

Do:

- Design around the operator's decision and the evidence required for it.
- Reuse semantic tokens and existing component recipes before adding variants.
- Put blockers, freshness, and finality next to the facts or actions they qualify.
- Preserve stable alignment and predictable interaction under real-time updates.
- Remove secondary decoration before reducing operational clarity.

Do not:

- Copy a bundled brand system into production without an explicit user decision.
- Use generic SaaS card grids as a substitute for information architecture.
- Use casino aesthetics, profit celebration, neon glow, or motion to manufacture urgency.
- Hide unsupported actions, missing evidence, or failed finality behind polished empty states.
- Add one-off hex colors, chart/icon libraries, unscoped CSS, or duplicated frontend DTOs.
- Treat an unreviewed prototype as production-ready merely because it renders.
