import { expect, test, type Page, type WebSocketRoute } from "@playwright/test";

const API = `http://127.0.0.1:${process.env.CROSSLINE_E2E_API_PORT ?? "18000"}`;
const WEB = `http://127.0.0.1:${process.env.CROSSLINE_E2E_WEB_PORT ?? "18080"}`;
const NOW = 1_790_210_400_000;
const strategies = [
  ["perp_cross", "永续跨所"], ["perp_price_spread", "永续价差"],
  ["spot_perp", "现货-永续"], ["cross_spot_perp", "跨所期现"], ["spot_cross", "现货跨所"],
] as const;

async function setup(page: Page, paginated = false) {
  const seed = await (await page.request.get(`${API}/api/v3/arbitrage/opportunities/list`)).json();
  const sockets = new Set<WebSocketRoute>();
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
    mainP0Counts: { ...seed.mainP0Counts, totalCount: rows.length, strategyCounts: counts },
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
        socket.send(JSON.stringify({ type: "ack", subscribed: msg.channels }));
        if (msg.channels.includes("arbitrage")) {
          sockets.add(socket);
          socket.send(JSON.stringify({ type: "message", channel: "arbitrage", payload: event() }));
        }
      } else if (msg.type === "ping") socket.send(JSON.stringify({ type: "pong" }));
    });
    socket.onClose(() => sockets.delete(socket));
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
  return { errors, writes, rows, sockets, searches,
    failSearch: (symbol?: string) => { failSymbol = symbol; },
    holdSearch: (symbol: string) => { heldSymbol = symbol; },
    releaseSearch: () => { heldSymbol = undefined; releaseSearch?.(); },
    stale: (value: boolean) => { streamStale = value; emit(); },
    partial: (value: boolean) => { streamPartial = value; emit(); },
    tick: () => { rows.forEach((row) => { row.longLeg.price += 0.5; }); emit(); },
  };
}

test("five strategy screens keep evidence expansion and focus across live ticks", async ({ page }) => {
  const f = await setup(page);
  await page.goto("/#futures");
  const table = page.getByRole("table", { name: "期货套利候选", exact: true });
  await expect(table.locator("tr.futures-data-row")).toHaveCount(2);
  for (const [, label] of strategies) {
    await page.getByRole("tab", { name: label, exact: true }).click();
    await expect(table.locator("tr.futures-data-row")).toHaveCount(2);
  }
  await page.getByRole("tab", { name: "永续跨所", exact: true }).click();
  await table.getByRole("button", { name: "查看证据", exact: true }).first().click();
  const full = table.locator(".futures-evidence-more");
  await full.locator("summary").click();
  const collapse = table.getByRole("button", { name: "收起证据", exact: true });
  await collapse.focus();
  for (const width of [1440, 1024, 390]) {
    await page.setViewportSize({ width, height: 900 });
    if (width >= 1024) {
      const lastTab = page.getByRole("tab", { name: "现货跨所", exact: true });
      const tabs = await lastTab.locator("..").boundingBox();
      const tab = await lastTab.boundingBox();
      expect(tab!.x + tab!.width).toBeLessThanOrEqual(tabs!.x + tabs!.width);
    }
    const panel = table.locator(".futures-evidence-panel");
    const bounds = await panel.boundingBox();
    expect(bounds?.x).toBeGreaterThanOrEqual(0);
    expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(width);
    const wrap = table.locator("..");
    const visibleRight = await wrap.evaluate((el) => el.getBoundingClientRect().left + el.clientWidth);
    const close = await panel.getByRole("button", { name: "收起", exact: true }).boundingBox();
    expect(close!.x + close!.width).toBeLessThanOrEqual(visibleRight);
    const action = table.locator(".futures-data-row.is-selected .futures-cell-action");
    const background = await action.evaluate((el) => getComputedStyle(el).backgroundColor);
    expect(background).not.toMatch(/rgba\([^)]*,\s*0\./);
    await page.screenshot({ path: test.info().outputPath(`futures-evidence-${width}.png`), fullPage: true });
  }
  await page.setViewportSize({ width: 1440, height: 900 });
  await collapse.focus();
  await expect.poll(() => f.sockets.size).toBeGreaterThan(0);
  f.tick();
  await expect(table.locator(".futures-data-row").first()).toContainText("60000.5");
  await expect(full).toHaveAttribute("open", "");
  await expect(collapse).toBeFocused();
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("late symbol results cannot roll back a newer query or venue filter", async ({ page }) => {
  const f = await setup(page);
  await page.goto("/#futures");
  const search = page.getByPlaceholder("BTC / BINANCE", { exact: true });
  const rows = page.locator(".futures-data-row");
  await expect(rows).toHaveCount(2);
  f.holdSearch("BTCUSDT");
  const oldResponse = page.waitForResponse((response) => new URL(response.url()).searchParams.get("symbol") === "BTCUSDT");
  await search.fill("BTCUSDT");
  await expect.poll(() => f.searches.includes("BTCUSDT")).toBe(true);
  await search.fill("ETH");
  await expect(page.locator(".futures-search-status")).toContainText("ETH · 搜索快照");
  await expect(rows).toHaveCount(1);
  await expect(rows.first()).toContainText("ETH");
  f.releaseSearch();
  await (await oldResponse).finished();
  await expect(rows.first()).not.toContainText("BTC");
  await search.fill("kraken");
  await expect(rows).toHaveCount(0);
  await expect(page.locator(".futures-search-status")).toHaveCount(0);
  expect(f.searches).not.toContain("KRAKEN");
  await search.fill("BTCUSDT");
  await expect(rows).toHaveCount(1);
  await expect(rows.first()).toContainText("BTC");
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("stale stream preserves quotes but disables building until recovery", async ({ page }) => {
  const f = await setup(page);
  await page.goto("/#futures");
  const build = page.getByRole("button", { name: "构建新双腿", exact: true });
  await expect(build).toBeEnabled();
  f.stale(true);
  await expect(build).toBeDisabled();
  await expect(page.locator(".futures-data-row")).toHaveCount(2);
  f.stale(false);
  await expect(build).toBeEnabled();
  f.partial(true);
  await expect(page.locator(".futures-feed-status summary")).toContainText("降级");
  await expect(build).toBeEnabled();
  f.partial(false);
  await build.click();
  await expect(page.locator(".execution-page")).toBeVisible();
  await expect(page.locator(".execution-ticket")).toContainText("BTC");
  await expect(page.locator(".execution-ticket")).toContainText("期货套利");
  for (const [, label] of strategies.slice(1)) {
    await page.goto("/#futures");
    await page.getByRole("tab", { name: label, exact: true }).click();
    await page.getByRole("button", { name: "构建新双腿", exact: true }).click();
    await expect(page.locator(".execution-ticket h3")).toHaveText(`BTC · ${label}`);
  }
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("a failed symbol search is visible and cannot offer an earlier symbol as current", async ({ page }) => {
  const f = await setup(page);
  await page.goto("/#futures");
  const search = page.getByPlaceholder("BTC / BINANCE", { exact: true });
  const table = page.getByRole("table", { name: "期货套利候选", exact: true });
  await expect(table.locator("tr.futures-data-row")).toHaveCount(2);
  await search.fill("BTC");
  await expect(table.locator("tr.futures-data-row")).toHaveCount(1);
  f.failSearch("SOL");
  await search.fill("SOL");
  await expect(page.locator(".futures-search-status")).toContainText("SOL · 搜索失败");
  await expect(table.locator("tr.futures-data-row")).toHaveCount(0);
  await page.screenshot({ path: test.info().outputPath("futures-search-failure.png"), fullPage: true });
  f.failSearch();
  await page.getByRole("button", { name: "重新搜索", exact: true }).click();
  await expect(page.locator(".futures-search-status")).toContainText("SOL · 搜索快照");
  await expect(table.locator("tr.futures-data-row")).toHaveCount(0);
  await search.clear();
  await expect(table.locator("tr.futures-data-row")).toHaveCount(2);
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("symbol pagination never inserts live first-page rows into a later page", async ({ page }) => {
  const f = await setup(page, true);
  await page.goto("/#futures");
  await page.getByPlaceholder("BTC / BINANCE", { exact: true }).fill("BTC");
  const rows = page.locator(".futures-data-row");
  await expect(rows).toHaveCount(1);
  await page.getByRole("button", { name: "查看证据", exact: true }).click();
  await page.getByRole("button", { name: "下一页", exact: true }).click();
  await expect(rows).toHaveCount(1);
  await expect(rows).toContainText("62000");
  await expect(page.locator(".futures-evidence-panel")).toHaveCount(0);
  f.tick();
  await expect(rows).toHaveCount(1);
  await expect(rows).toContainText("62000");
  await page.getByRole("button", { name: "首页", exact: true }).click();
  await expect(rows).toHaveCount(1);
  await expect(rows).toContainText("60000.5");
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});
