import { type Page, type WebSocketRoute } from "@playwright/test";

export const API = `http://127.0.0.1:${process.env.CROSSLINE_E2E_API_PORT ?? "18000"}`;
export const WEB = `http://127.0.0.1:${process.env.CROSSLINE_E2E_WEB_PORT ?? "18080"}`;
export const NOW = 1_790_210_400_000;
export const strategies = [
  ["perp_cross", "永续跨所"], ["perp_price_spread", "永续价差"],
  ["spot_perp", "现货-永续"], ["cross_spot_perp", "跨所期现"], ["spot_cross", "现货跨所"],
] as const;

export async function setup(page: Page, paginated = false) {
  const seed = await (await page.request.get(`${API}/api/v3/arbitrage/opportunities/list`)).json();
  const sockets = new Set<WebSocketRoute>();
  const channelSockets = new Map<string, Set<WebSocketRoute>>();
  const errors: string[] = [];
  const writes: string[] = [];
  let failSymbol: string | undefined;
  let heldSymbol: string | undefined;
  let releaseSearch: (() => void) | undefined;
  let streamStale = false;
  let streamPartial = false;
  const searches: string[] = [];
  let revision = 1;
  const rows = strategies.flatMap(([kind, label]) => ["BTC", "ETH"].map((symbol, index) => {
    const row = structuredClone(seed.rows[0]);
    Object.assign(row, { id: `fixture-${kind}-${symbol}`, symbol, strategyKind: kind, typeLabel: label,
      updatedAt: new Date(NOW).toISOString() });
    row.longLeg.venue = "binance";
    row.longLeg.action = `binance ${kind.includes("spot") ? "买入现货" : "做多永续"}`;
    row.shortLeg.venue = kind === "spot_perp" ? "binance" : "bitget";
    row.shortLeg.action = `${row.shortLeg.venue} ${kind === "spot_cross" ? "卖出现货" : "做空永续"}`;
    row.longLeg.price = symbol === "BTC" ? 60000 : 3000;
    row.shortLeg.price = row.longLeg.price + 1;
    for (const leg of [row.longLeg, row.shortLeg]) {
      Object.assign(leg.marketEvidence, { venue: leg.venue, symbol, price: leg.price });
      Object.assign(leg.marketEvidence.health, { source: "ws_push", observedAtMs: NOW, quality: "fresh" });
    }
    row.execution.eligible = index === 0;
    row.execution.blockers = index === 0 ? [] : ["fixture: 双腿结算窗口尚未对齐，仅观察"];
    return row;
  }));
  const counts = Object.fromEntries(strategies.map(([kind]) => [kind, 2]));
  const envelope = (selected: any[]) => ({ ...structuredClone(seed), rows: selected,
    page: { ...seed.page, returnedCount: selected.length, totalRows: selected.length, pageSize: 50, snapshotId: `futures-${revision}` },
    scopeMeta: { ...seed.scopeMeta, globalTotalCount: rows.length, strategyScopeCount: selected.length, symbolScopeCount: selected.length, filteredCount: selected.length, emittedCount: selected.length },
    mainP0Counts: { ...seed.mainP0Counts, totalCount: rows.length, executableCount: rows.filter((row) => row.execution.eligible).length,
      strategyCounts: counts, executableStrategyCounts: Object.fromEntries(strategies.map(([kind]) => [kind, 1])) },
    observedAtMs: NOW + revision, cachedAt: new Date(NOW + revision).toISOString(),
  });
  const event = () => ({ ...envelope(rows), event: "snapshot_invalidated", snapshotId: `futures-${revision}`,
    status: streamStale ? "stale" : streamPartial ? "degraded" : "fresh",
    partialFailures: streamPartial ? [{ code: "VENUE_MISSING", message: "fixture: unrelated venue unavailable" }] : [],
    error: streamStale ? { code: "OPPORTUNITY_SNAPSHOT_STALE", message: "fixture: snapshot stale" } : null,
    windows: [null, ...strategies.map(([kind]) => kind)].map((kind) => {
      const selected = rows.filter((row) => !kind || row.strategyKind === kind);
      const window = envelope(selected);
      return { strategyKind: kind, ids: selected.map((row) => row.id), page: window.page, scopeMeta: window.scopeMeta, queryKey: `scope=main_p0;strategy=${kind ?? "*"}` };
    }),
    changedIds: rows.map((row) => row.id), changedRows: rows, removedIds: [], topIds: rows.map((row) => row.id) });
  await page.clock.setFixedTime(NOW);
  await page.addInitScript((api) => {
    (Error as ErrorConstructor & { stackTraceLimit: number }).stackTraceLimit = 60;
    localStorage.setItem("api_base", JSON.stringify(api));
    localStorage.setItem("api_auth_token", JSON.stringify("isolated-fixture-token"));
  }, API);
  page.on("pageerror", (error) => errors.push(error.stack ?? error.message));
  await page.routeWebSocket(/.*/, (socket) => {
    if (!socket.url().startsWith(API.replace("http:", "ws:"))) return socket.close();
    socket.onMessage((raw) => {
      const msg = JSON.parse(raw.toString());
      if (msg.type === "subscribe") {
        for (const channel of msg.channels) {
          if (!channelSockets.has(channel)) channelSockets.set(channel, new Set());
          channelSockets.get(channel)!.add(socket);
        }
        socket.send(JSON.stringify({ type: "ack", subscribed: msg.channels }));
        if (msg.channels.includes("arbitrage")) {
          sockets.add(socket);
          socket.send(JSON.stringify({ type: "message", channel: "arbitrage", payload: event() }));
        }
      } else if (msg.type === "ping") socket.send(JSON.stringify({ type: "pong" }));
    });
    socket.onClose(() => { sockets.delete(socket); channelSockets.forEach((set) => set.delete(socket)); });
  });
  await page.route("**/*", async (route) => {
    const url = new URL(route.request().url());
    if (![API, WEB].includes(url.origin)) return route.abort();
    if (!["GET", "HEAD"].includes(route.request().method()) && url.pathname !== "/api/auth/ws-ticket") {
      writes.push(url.pathname);
      return route.fulfill({ status: 409, json: { code: "ISOLATED_TEST", message: "writes disabled" } });
    }
    if (url.pathname === "/api/v3/arbitrage/opportunities/list") {
      const symbol = url.searchParams.get("symbol");
      if (symbol) searches.push(symbol);
      if (symbol && symbol === heldSymbol) await new Promise<void>((resolve) => { releaseSearch = resolve; });
      if (symbol === failSymbol) return route.fulfill({ status: 503, json: { code: "SEARCH_UNAVAILABLE", message: `fixture: ${symbol} search unavailable` } });
      const canonical = symbol === "BTCUSDT" ? "BTC" : symbol;
      const kinds = (url.searchParams.get("strategy") ?? "").split(",");
      const selected = rows.filter((row) => (!canonical || row.symbol === canonical) && (!kinds[0] || kinds.includes(row.strategyKind)));
      const response = envelope(selected);
      response.requestMeta = { fast: false, fresh: false,
        filter: { scope: "main_p0", strategyKinds: kinds.filter(Boolean), symbol: canonical, minYield: null },
        sortKey: seed.page.sortKey, requestedPageSize: 50, appliedPageSize: 50, maxPageSize: 50 };
      if (paginated && canonical === "BTC") {
        const offset = url.searchParams.get("cursor") === "page-2" ? 1 : 0;
        response.rows = [structuredClone(selected[0])];
        if (offset) {
          response.rows[0].id = "fixture-perp_cross-BTC-page2";
          response.rows[0].longLeg.price = 62000;
        }
        Object.assign(response.page, { startOffset: offset, returnedCount: 1, totalRows: 2,
          hasNextPage: !offset, nextCursor: offset ? null : "page-2", previousCursor: null,
          lastCursor: offset ? null : "page-2" });
      }
      return route.fulfill({ json: response });
    }
    if (["/api/v1/strategy/kinds", "/api/strategy/main-kinds"].includes(url.pathname)) {
      const kinds = await (await route.fetch()).json();
      const source = kinds.find((row: any) => row.kind === "perp_cross");
      return route.fulfill({ json: strategies.map(([kind, label]) => ({ ...source, kind, labelZh: label, labelEn: kind })) });
    }
    return route.continue();
  });
  const emit = () => { revision++; sockets.forEach((socket) => socket.send(JSON.stringify({ type: "message", channel: "arbitrage", payload: event() }))); };
  return { errors, writes, rows, sockets, channelSockets, searches,
    failSearch: (symbol?: string) => { failSymbol = symbol; },
    holdSearch: (symbol: string) => { heldSymbol = symbol; },
    releaseSearch: () => { heldSymbol = undefined; releaseSearch?.(); },
    stale: (value: boolean) => { streamStale = value; emit(); },
    partial: (value: boolean) => { streamPartial = value; emit(); },
    tick: () => { rows.forEach((row) => { row.longLeg.price += 0.5; }); emit(); },
  };
}
