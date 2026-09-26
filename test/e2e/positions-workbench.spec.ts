import { expect, test, type Page, type WebSocketRoute } from "@playwright/test";
import { riskFixture } from "./fixtures/settings-risk";

const API = "http://127.0.0.1:18000";
const NOW = 1_790_210_400_000;

async function switchPortfolioConnection(page: Page, kind: "token" | "base", changed: boolean) {
  await page.getByRole("button", { name: "切换到设置", exact: true }).click();
  await page.getByRole("tab", { name: "诊断", exact: true }).click();
  await page.getByRole("tab", { name: "连接", exact: true }).click();
  if (kind === "token") {
    await page.locator(".settings-api-token-task input").fill(changed ? "positions-other-token" : "isolated-fixture-token");
    await page.getByRole("button", { name: "保存 Token", exact: true }).click();
  } else {
    await page.getByLabel("API Base", { exact: true }).fill(changed ? `${API}/other-backend` : API);
    await page.getByPlaceholder("apply", { exact: true }).fill("apply");
    await page.getByRole("button", { name: "保存并应用", exact: true }).click();
  }
  await page.getByRole("button", { name: /^切换到持仓\/风控(?:，|$)/ }).click();
}

for (const kind of ["token", "base"] as const) {
  test(`positions ${kind} switch clears retained accounts and rejects late A-B-A reads`, async ({ page }) => {
    await page.clock.install({ time: NOW });
    const f = await setup(page);
    const historyBody = (navUsd: number) => ({ count: 1, rows: [{ occurredAtMs: NOW, navUsd }],
      source: "isolated-connection", observedAtMs: NOW, latestAtMs: NOW, freshnessMs: 0 });
    await page.route(`${API}/api/trading/portfolio/nav-history?*`, route => route.fulfill({ json: historyBody(111) }));
    await page.goto("/#positions");
    await expect(page.locator(".positions-table")).toContainText("BTCUSDT");
    const baseline = f.snapshot();
    const envelope = await (await page.request.get(`${API}/api/trading/portfolio/snapshot`)).json();
    await page.getByRole("tab", { name: "资产", exact: true }).click();
    const history = page.locator(".positions-nav-region");
    await expect(history).toContainText("$111");
    const reads: string[] = [];
    const cancelled = new Set<string>();
    page.on("requestfailed", request => {
      const path = new URL(request.url()).pathname;
      const original = kind === "token" ? request.headers().authorization === "Bearer isolated-fixture-token"
        : !path.startsWith("/other-backend/");
      if (original && path.includes("/api/trading/portfolio/")) {
        cancelled.add(path.endsWith("snapshot") ? "snapshot" : "history");
      }
    });
    const pending = new Map<string, () => void>();
    const hold = new Set(["original", "other"]);
    let generation = 1;
    const makeSnapshot = (symbol: string, nav: number) => {
      const snapshot = structuredClone(baseline);
      snapshot.positions = [position("binance", symbol, 0.1, 100)];
      snapshot.summary.totalNavUsd = nav;
      snapshot.summary.netDeltaUsd = nav;
      snapshot.recentCloseRuns = [closeRun(`close-${symbol}`, "succeeded", "filled")];
      return snapshot;
    };
    await page.route("**/api/**", async route => {
      const url = new URL(route.request().url());
      if (url.origin !== API) return route.fallback();
      const changed = kind === "token" ? route.request().headers().authorization === "Bearer positions-other-token"
        : url.pathname.startsWith("/other-backend/");
      const path = url.pathname.replace("/other-backend", "");
      const source = changed ? "other" : "original";
      const nav = changed ? 222 : generation === 1 ? 111 : 333;
      const symbol = changed ? "CURRENT-B" : generation === 1 ? "OLD-LATE-A" : "CURRENT-A";
      if (!["/api/trading/portfolio/snapshot", "/api/trading/portfolio/nav-history"].includes(path)) {
        if (url.pathname.startsWith("/other-backend/")) return route.fallback({ url: `${API}${path}${url.search}` });
        return route.fallback();
      }
      const operation = path.endsWith("snapshot") ? "snapshot" : "history";
      const id = `${source}:${operation}`;
      reads.push(id);
      const body = operation === "snapshot" ? { ...envelope, snapshot: makeSnapshot(symbol, nav), observedAtMs: NOW }
        : historyBody(nav);
      if (hold.has(source) && !pending.has(id)) await new Promise<void>(resolve => pending.set(id, resolve));
      await route.fulfill({ json: body });
    });
    await history.getByRole("button", { name: "刷新净值历史", exact: true }).click();
    await page.clock.fastForward(8_100);
    await expect.poll(() => pending.has("original:snapshot") && pending.has("original:history")).toBe(true);
    await switchPortfolioConnection(page, kind, true);
    await expect.poll(() => [...cancelled].sort()).toEqual(["history", "snapshot"]);
    await expect(page.locator(".positions-layout")).not.toContainText("BTCUSDT");
    await expect(page.locator(".positions-layout")).not.toContainText("close-filled");
    await expect.poll(() => pending.has("other:snapshot") && pending.has("other:history")).toBe(true);
    pending.get("other:snapshot")!();
    await expect(page.locator(".positions-table")).toContainText("CURRENT-B");
    await page.getByRole("tab", { name: "资产", exact: true }).click();
    await expect(history).toContainText("读取账户净值历史中");
    await expect(history).not.toContainText("$111");
    hold.delete("other");
    pending.get("other:history")!();
    await expect(history).toContainText("$222");
    await expect(history.getByRole("button", { name: "刷新净值历史", exact: true })).toBeEnabled();
    generation = 2;
    hold.delete("original");
    await switchPortfolioConnection(page, kind, false);
    await expect(page.locator(".positions-table")).toContainText("CURRENT-A");
    pending.get("original:snapshot")!();
    pending.get("original:history")!();
    await page.getByRole("tab", { name: "资产", exact: true }).click();
    await expect(history).toContainText("$333");
    await expect(history).not.toContainText("$111");
    await expect(page.locator(".positions-layout")).not.toContainText("OLD-LATE-A");
    await page.getByRole("tab", { name: "持仓", exact: true }).click();
    await expect.poll(() => f.sockets.size).toBeGreaterThan(0);
    const current = makeSnapshot("CURRENT-WS", 444);
    f.send(current);
    await expect(page.locator(".positions-table")).toContainText("CURRENT-WS");
    await page.getByRole("tab", { name: "平仓", exact: true }).click();
    await expect(page.getByRole("region", { name: "平仓记录", exact: true })).toContainText("close-CURRENT-WS");
    await expect(page.locator(".positions-layout")).not.toContainText("close-CURRENT-B");
    expect(reads.filter(read => read === "other:history")).toHaveLength(1);
    expect(f.errors).toEqual([]);
    expect(f.writes).toEqual([]);
  });
}

async function setup(page: Page, configure: (snapshot: any) => void = () => {}) {
  const errors: string[] = [];
  const writes: string[] = [];
  const sockets = new Set<WebSocketRoute>();
  let latest: any;
  await page.clock.setFixedTime(NOW);
  await page.addInitScript((api) => {
    localStorage.setItem("api_base", JSON.stringify(api));
    localStorage.setItem("api_auth_token", JSON.stringify("isolated-fixture-token"));
  }, API);
  page.on("pageerror", (e) => errors.push(e.message));
  await page.routeWebSocket(/.*/, (socket) => {
    if (!socket.url().startsWith(API.replace("http:", "ws:"))) return socket.close();
    socket.onMessage((raw) => {
      const message = JSON.parse(raw.toString());
      if (message.type === "subscribe") {
        socket.send(JSON.stringify({ type: "ack", subscribed: message.channels }));
        if (message.channels.includes("portfolio")) sockets.add(socket);
      } else if (message.type === "ping") socket.send(JSON.stringify({ type: "pong" }));
    });
    socket.onClose(() => sockets.delete(socket));
  });
  // Only the isolated fixture server and static build can be reached.
  await page.route("**/*", async (route) => {
    const url = new URL(route.request().url());
    if (!url.origin.match(/^http:\/\/127\.0\.0\.1:(18000|18080)$/)) return route.abort();
    if (route.request().method() === "POST" && url.pathname !== "/api/auth/ws-ticket") {
      writes.push(url.pathname);
      return route.fulfill({ status: 409, json: { code: "ISOLATED_TEST", message: "writes disabled" } });
    }
    if (url.pathname === "/api/trading/portfolio/nav-history") {
      return route.fulfill({ json: { count: 0, rows: [], source: "isolated-fixture", observedAtMs: NOW } });
    }
    if (url.pathname === "/api/trading/status") {
      const body = await (await route.fetch()).json();
      body.environment = "live";
      body.risk.liveTradingEnabled = true;
      return route.fulfill({ json: body });
    }
    if (url.pathname !== "/api/trading/portfolio/snapshot") return route.continue();
    const response = await route.fetch();
    const body = await response.json();
    const snapshot = body.snapshot;
    snapshot.snapshotVersion = "positions-workbench-fixture";
    snapshot.serverNowMs = NOW;
    snapshot.degraded = false;
    snapshot.problems = [];
    snapshot.operationHealth = ["binance", "bitget"].map((venue) => ({
      venue, operation: "positions", status: "ok", source: "isolated-fixture",
      message: "fixture", supported: true, configured: true, observedAtMs: NOW,
    }));
    snapshot.positions = [position("binance", "BTCUSDT", 0.2, 60000), position("bitget", "SOLUSDT", 0.1, 150)];
    snapshot.recentCloseRuns = [closeRun("close-filled", "succeeded", "filled"), closeRun("close-pending", "submitted", "accepted")];
    Object.assign(snapshot.accountState.positions, { status: "fresh", problems: [], rows: [], fieldQuality: [], rowHealth: [], observedAtMs: NOW });
    snapshot.accountState.fieldQuality = [];
    Object.assign(snapshot.summary, {
      navEvidence: { status: "actual", source: "isolated-fixture", observedAtMs: NOW,
        coveredVenues: ["binance", "bitget"], missingVenues: [] },
      netDeltaUsd: 12015, netDeltaPctOfNav: 120.15, nakedExposureUsd: 12015, nakedPositionCount: 2,
    });
    body.observedAtMs = NOW;
    body.operationHealth = snapshot.operationHealth;
    configure(snapshot);
    latest = structuredClone(snapshot);
    return route.fulfill({ json: body });
  });
  const send = (payload: any) => sockets.forEach((socket) => socket.send(JSON.stringify({ type: "message", channel: "portfolio", payload })));
  return { errors, writes, send, snapshot: () => structuredClone(latest), sockets };
}

function position(venue: string, symbol: string, quantity: number, markPrice: number) {
  return { venue, symbol, side: "long", quantity, entryPrice: markPrice - 1, markPrice,
    origin: "execution_ledger", leverage: 3, unrealizedPnlUsd: 5, liquidationPrice: markPrice * 0.3,
    liquidationDistancePct: 70, marginUsd: markPrice * quantity / 3,
    nextFundingMs: NOW + 3_600_000, fundingRate8h: 0.0001, fundingRateVerified: true,
    maintenanceMarginRatio: 0.01, severity: "ok", secondsUntilFunding: 3600 };
}

function closeRun(id: string, status: string, legStatus: string) {
  return { id, scope: "single", status, snapshotVersion: "close-fixture", expectedLegCount: 1,
    submittedOrderCount: 1, failedLegCount: 0, nakedExposureUsd: status === "succeeded" ? 0 : 15,
    message: status === "succeeded" ? "1 条订单已确认成交" : "已受理，等待交易所最终结果",
    legs: [{ venue: "bitget", symbol: "SOLUSDT", side: "long", status: legStatus,
      quantity: 0.1, markPrice: 150, notionalUsd: 15 }],
    startedAtMs: NOW - 60_000, updatedAtMs: NOW - (status === "succeeded" ? 30_000 : 1000) };
}

function pairPositions(rows: any[]) {
  rows.forEach((row, index) => {
    const partner = rows[1 - index];
    row.pairEvidence = { source: "execution_run", runId: "pair-fixture", ticketId: "ticket-fixture",
      opportunityId: "opp-fixture", venue: row.venue, symbol: row.symbol, side: row.side,
      partnerVenue: partner.venue, partnerSymbol: partner.symbol, partnerSide: partner.side,
      legFilledQuantity: row.quantity, partnerFilledQuantity: partner.quantity,
      matchedNotionalUsd: 15, updatedAtMs: NOW };
  });
}

function incident(id: string, status = "unwind_required") {
  const candidate = { venue: "bitget", symbol: "SOLUSDT", side: "long", status: "filled",
    targetQuantity: 0.1, confirmedQuantity: 0.1, confirmedPrice: 150, markPrice: 150,
    notionalUsd: 15, notionalQuality: "actual", notionalSource: "fixture-fill", compensationOrderSide: "buy" };
  return { ...closeRun(id, status, "filled"), unwindPlan: {
    status: status === "compensation_failed" ? "compensation_failed" : "blocked_pending_manual_recheck",
    filledLegs: [candidate], failedLegs: [], compensationCandidates: [candidate], remainingPositions: [],
    compensationAttempts: [], requiredEvidence: [], nextActions: [{
      kind: status === "compensation_failed" ? "manual_incident_review" : "submit_compensation_order",
      label: status === "compensation_failed" ? "人工复核" : "提交补买", candidateIndex: 0, requiresConfirmation: true,
      requiredEvidence: [],
    }],
  }};
}

test("positions receipt history survives reload, WS updates and older portfolio snapshots", async ({ page }) => {
  const fixture = await setup(page);
  await page.goto("/#positions");
  await expect(page.locator(".positions-table")).toContainText("BTCUSDT");
  for (const width of [1440, 1024, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await page.screenshot({ path: test.info().outputPath(`positions-${width}.png`), fullPage: true });
    const close = page.getByRole("button", { name: "平仓", exact: true }).first();
    const bounds = await close.boundingBox();
    expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(width);
  }
  await page.getByRole("tab", { name: "平仓", exact: true }).click();
  const history = page.getByRole("region", { name: "平仓记录", exact: true });
  await expect(history).toContainText("最近 2 条");
  const pending = history.locator("details").filter({ hasText: "close-pending" });
  await expect(pending.locator("summary")).toContainText("等待成交");
  await expect(pending.locator("summary")).toContainText("0/1");
  await pending.locator("summary").click();
  await expect.poll(() => fixture.sockets.size).toBeGreaterThan(0);
  const filled = { ...closeRun("close-pending", "succeeded", "filled"), updatedAtMs: NOW + 100 };
  fixture.send({ event: "close_run_updated", closeRun: filled, timestampMs: NOW + 100 });
  await expect(pending.locator("summary")).toContainText("已完成");
  await expect(pending.locator("summary")).toContainText("1/1");
  await expect(pending).toHaveAttribute("open", "");
  fixture.send(fixture.snapshot());
  await expect(pending.locator("summary")).toContainText("已完成");
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    expect(await history.evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
    await page.screenshot({ path: test.info().outputPath(`positions-history-${width}.png`), fullPage: true });
  }
  fixture.send({ status: "error", source: "isolated-fixture", observedAtMs: NOW + 200,
    problem: { code: "TIMEOUT", message: "isolated snapshot timeout", status: 504 } });
  await expect(history).toContainText("刷新失败，显示上次记录");
  await expect(pending.locator("summary")).toContainText("已完成");
  await page.reload();
  await page.getByRole("tab", { name: "平仓", exact: true }).click();
  await expect(history).toContainText("最近 2 条");
  await expect(history.locator("details").filter({ hasText: "close-filled" }).locator("summary")).toContainText("已完成");
  expect(fixture.errors).toEqual([]);
  expect(fixture.writes).toEqual([]);
});

test("unknown account configuration is not claimed as complete coverage", async ({ page }) => {
  await setup(page, (snapshot) => { snapshot.operationHealth = []; });
  await page.goto("/#positions");
  const coverage = page.locator(".positions-command-coverage");
  await expect(coverage).toContainText("状态待确认");
  await expect(coverage).not.toContainText("覆盖完整");
  await expect(coverage).not.toContainText("0/0");
});

test("close controls require current execution state and invalidate old confirmations", async ({ page }) => {
  await page.clock.install({ time: NOW });
  await page.setViewportSize({ width: 1440, height: 900 });
  const f = await setup(page, snapshot => {
    snapshot.positions[0].origin = "account_private";
    snapshot.recentCloseRuns = [];
  });
  const baseline = await (await page.request.get(`${API}/api/trading/status`)).json();
  let current: any = null;
  let reads = 0;
  await page.route(`${API}/api/trading/status`, route => {
    reads++;
    return current ? route.fulfill({ json: structuredClone(current) }) : route.fulfill({
      status: 504, json: { error: { code: "TIMEOUT", message: "fixture execution status unavailable", status: 504 } },
    });
  });
  const refresh = async (environment: "paper" | "live" | null, enabled = false, adapter = "fixture") => {
    current = environment ? { ...baseline, adapter, environment,
      risk: { ...baseline.risk, liveTradingEnabled: enabled } } : null;
    const before = reads;
    const response = page.waitForResponse(`${API}/api/trading/status`);
    await page.clock.fastForward(5100);
    await (await response).finished();
    await page.clock.runFor(32);
    await expect.poll(() => reads).toBeGreaterThan(before);
  };
  await page.goto("/#positions");
  const table = page.locator(".positions-table");
  const accountRow = table.locator("tbody > tr").filter({ has: page.locator(".row-close-button"), hasText: "BTCUSDT" });
  const ledgerRow = table.locator("tbody > tr").filter({ has: page.locator(".row-close-button"), hasText: "SOLUSDT" });
  const confirmation = page.getByRole("group", { name: /^确认平仓：bitget SOLUSDT/ });
  const all = page.locator(".close-all-control");
  const phrase = all.locator("input");
  await expect(table).toContainText("SOLUSDT");
  await expect(table.getByRole("button", { name: "环境待确认", exact: true })).toHaveCount(2);
  await page.getByRole("tab", { name: "控制", exact: true }).click();
  await phrase.fill("CLOSE_ALL_POSITIONS");
  await expect(all.getByRole("button", { name: "环境待确认", exact: true })).toBeDisabled();

  await refresh("paper");
  await expect(phrase).toHaveValue("");
  await expect(all.getByRole("button", { name: "需实盘", exact: true })).toBeDisabled();
  await page.getByRole("tab", { name: "持仓", exact: true }).click();
  await expect(accountRow.getByRole("button", { name: "需实盘", exact: true })).toBeDisabled();
  await ledgerRow.getByRole("button", { name: "平仓", exact: true }).click();
  await expect(confirmation.getByRole("button", { name: "模拟平仓", exact: true })).toBeEnabled();

  await refresh("live", true);
  await expect(confirmation).not.toBeVisible();
  await ledgerRow.getByRole("button", { name: "平仓", exact: true }).click();
  await expect(confirmation.getByRole("button", { name: "实盘平仓", exact: true })).toBeEnabled();
  await refresh("live", true);
  // An unchanged successful read must not collapse an in-progress confirmation.
  await expect(confirmation).toBeVisible();
  await refresh(null);
  await expect(confirmation).not.toBeVisible();
  await expect(table.getByRole("button", { name: "环境待确认", exact: true })).toHaveCount(2);
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await table.scrollIntoViewIfNeeded();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    await page.screenshot({ path: test.info().outputPath(`positions-environment-unknown-${width}.png`) });
  }

  await refresh("live", false);
  await expect(table.getByRole("button", { name: "实盘未启用", exact: true })).toHaveCount(2);
  await page.getByRole("tab", { name: "控制", exact: true }).click();
  await phrase.fill("CLOSE_ALL_POSITIONS");
  await expect(all.getByRole("button", { name: "实盘未启用", exact: true })).toBeDisabled();
  await refresh("live", true);
  await expect(phrase).toHaveValue("");
  await phrase.fill("CLOSE_ALL_POSITIONS");
  await expect(all.getByRole("button", { name: "关闭全部 2 个持仓" })).toBeEnabled();
  await refresh("live", true, "changed-adapter");
  await expect(phrase).toHaveValue("");
  await expect(all.getByRole("button", { name: "输入确认短语" })).toBeDisabled();
  await phrase.fill("CLOSE_ALL_POSITIONS");
  await expect.poll(() => f.sockets.size).toBeGreaterThan(0);
  f.send({ status: "error", source: "isolated-fixture", observedAtMs: NOW + 100,
    problem: { code: "TIMEOUT", message: "fixture position refresh unavailable", status: 504 } });
  await expect(all.getByRole("button", { name: "持仓待确认", exact: true })).toBeDisabled();
  // Even a synthetic submit event must respect the same request-boundary guard.
  await all.dispatchEvent("submit");
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await all.scrollIntoViewIfNeeded();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    await page.screenshot({ path: test.info().outputPath(`positions-environment-controls-${width}.png`) });
  }
  expect(f.writes).toEqual([]);
  expect(f.errors).toEqual([]);
});

test("pair close checks both sources and preserves confirmation only through market repricing", async ({ page }) => {
  await page.clock.install({ time: NOW });
  const f = await setup(page, snapshot => {
    pairPositions(snapshot.positions);
    snapshot.positions[0].origin = "account_private";
    snapshot.recentCloseRuns = [];
  });
  const baseline = await (await page.request.get(`${API}/api/trading/status`)).json();
  let live = false;
  await page.route(`${API}/api/trading/status`, route => route.fulfill({ json: {
    ...baseline, adapter: "fixture", environment: live ? "live" : "paper",
    risk: { ...baseline.risk, liveTradingEnabled: live },
  } }));
  await page.goto("/#positions");
  const table = page.locator(".positions-table");
  const ledgerRow = table.locator("tbody > tr").filter({ has: page.locator(".position-identity-cell strong", { hasText: "SOLUSDT" }) });
  const confirmation = page.getByRole("group", { name: /^确认平配对：bitget SOLUSDT/ });
  await expect(table.getByRole("button", { name: "需实盘", exact: true })).toHaveCount(2);
  await expect(ledgerRow.locator(".row-close-button")).toBeDisabled();
  await expect.poll(() => f.sockets.size).toBeGreaterThan(0);
  const original = f.snapshot();
  let tick = 0;
  const send = (edit: (snapshot: any) => void) => {
    const snapshot = structuredClone(original);
    snapshot.serverNowMs = NOW + ++tick;
    edit(snapshot);
    f.send(snapshot);
  };
  send(snapshot => snapshot.positions.shift());
  await expect(ledgerRow.locator(".row-close-button")).toHaveText("配对待确认");
  await expect(ledgerRow.locator(".row-close-button")).toHaveAttribute("title", /另一条配对腿尚未读取/);
  send(snapshot => snapshot.positions[0].pairEvidence.runId = "different-run");
  await expect(ledgerRow.locator(".row-close-button")).toHaveAttribute("title", /双腿配对证据不一致/);
  send(snapshot => snapshot.positions[0].origin = "execution_ledger");
  await ledgerRow.getByRole("button", { name: "平配对", exact: true }).click();
  await expect(confirmation.getByRole("button", { name: "模拟平配对", exact: true })).toBeEnabled();
  send(snapshot => {
    snapshot.positions[0].origin = "execution_ledger";
    snapshot.positions[1].markPrice += 1;
  });
  await expect(confirmation).toBeVisible();
  await expect(confirmation).toContainText("$151");
  send(() => {});
  await expect(confirmation).not.toBeVisible();
  await expect(ledgerRow.getByRole("button", { name: "需实盘", exact: true })).toBeDisabled();
  live = true;
  const response = page.waitForResponse(`${API}/api/trading/status`);
  await page.clock.fastForward(5100);
  await (await response).finished();
  await ledgerRow.getByRole("button", { name: "平配对", exact: true }).click();
  await expect(confirmation.getByRole("button", { name: "实盘平配对", exact: true })).toBeEnabled();
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await confirmation.scrollIntoViewIfNeeded();
    await confirmation.getByRole("button", { name: "实盘平配对", exact: true }).click({ trial: true });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    await page.screenshot({ path: test.info().outputPath(`pair-close-context-${width}.png`) });
  }
  send(snapshot => snapshot.positions[0].quantity += 0.1);
  await expect(confirmation).not.toBeVisible();
  expect(f.writes).toEqual([]);
  expect(f.errors).toEqual([]);
});

test("close receipts survive navigation and a late acknowledgement cannot undo WS finality", async ({ page }) => {
  const f = await setup(page, snapshot => { snapshot.recentCloseRuns = []; });
  const requests: { idempotency: string; requestId: string; body: any }[] = [];
  let release: (() => void) | undefined;
  const path = `${API}/api/trading/portfolio/positions/bitget/SOLUSDT/close`;
  await page.route(path, async route => {
    requests.push({ idempotency: route.request().headers()["idempotency-key"],
      requestId: route.request().headers()["x-request-id"], body: route.request().postDataJSON() });
    const attempt = requests.length;
    await new Promise<void>(resolve => { release = resolve; });
    const run = closeRun(`close-http-${attempt}`, attempt === 1 ? "succeeded" : "submitted", attempt === 1 ? "filled" : "accepted");
    run.updatedAtMs = NOW + attempt * 100;
    return route.fulfill({ json: { ...run, idempotencyKey: requests[attempt - 1].idempotency,
      requestId: requests[attempt - 1].requestId, actionRunId: `action-http-${attempt}` } });
  });
  await page.goto("/#positions");
  const submit = async () => {
    await page.getByRole("tab", { name: "持仓", exact: true }).click();
    await page.locator(".positions-table tbody > tr").filter({ hasText: "SOLUSDT" })
      .getByRole("button", { name: "平仓", exact: true }).click();
    await page.getByRole("group", { name: /^确认平仓：bitget SOLUSDT/ })
      .getByRole("button", { name: "实盘平仓", exact: true }).click();
  };
  await submit();
  await expect.poll(() => requests.length).toBe(1);
  await page.getByRole("button", { name: "切换到设置", exact: true }).click();
  await page.getByRole("button", { name: "切换到持仓/风控", exact: true }).click();
  await expect(page.getByRole("alert", { name: "平仓操作待核对" })).toContainText("平仓处理中");
  await expect(page.locator(".positions-table").getByRole("button", { name: "平仓", exact: true }).first()).toBeDisabled();
  await page.getByRole("button", { name: "切换到设置", exact: true }).click();
  const firstReply = page.waitForResponse(path);
  release!();
  await (await firstReply).finished();
  await page.getByRole("button", { name: "切换到持仓/风控", exact: true }).click();
  await page.getByRole("tab", { name: "平仓", exact: true }).click();
  const history = page.getByRole("region", { name: "平仓记录", exact: true });
  await expect(history.locator("details").filter({ hasText: "close-http-1" }).locator("summary")).toContainText("已完成");

  await submit();
  await expect.poll(() => requests.length).toBe(2);
  await page.getByRole("tab", { name: "平仓", exact: true }).click();
  await expect.poll(() => f.sockets.size).toBeGreaterThan(0);
  f.send({ event: "close_run_updated", timestampMs: NOW + 300,
    closeRun: { ...closeRun("close-http-2", "succeeded", "filled"), updatedAtMs: NOW + 300,
      idempotencyKey: requests[1].idempotency, requestId: requests[1].requestId, actionRunId: "action-http-2" } });
  const record = history.locator("details").filter({ hasText: "close-http-2" });
  await expect(record.locator("summary")).toContainText("已完成");
  const secondReply = page.waitForResponse(path);
  release!();
  await (await secondReply).finished();
  await expect(page.locator(".positions-action-message").filter({ hasText: "正在提交" })).toHaveCount(0);
  await expect(page.locator(".positions-main")).not.toContainText("等待交易所最终结果");
  await expect(record.locator("summary")).toContainText("已完成");
  await expect(page.locator(".positions-table")).toContainText("SOLUSDT");
  // A receipt does not prove a new account snapshot or silently erase the position.
  f.send({ status: "error", source: "isolated-fixture", observedAtMs: NOW + 400,
    problem: { code: "TIMEOUT", message: "isolated snapshot timeout", status: 504 } });
  await expect(history).toContainText("刷新失败，显示上次记录");
  await expect(record.locator("summary")).toContainText("已完成");
  await page.setViewportSize({ width: 390, height: 900 });
  await history.scrollIntoViewIfNeeded();
  expect(await history.evaluate(el => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
  await page.screenshot({ path: test.info().outputPath("positions-close-receipts-390.png") });
  expect(requests.map(r => r.body)).toEqual([
    expect.objectContaining({ side: "long", expectedLegCount: 1 }),
    expect.objectContaining({ side: "long", expectedLegCount: 1 }),
  ]);
  expect(requests[1].idempotency).not.toBe(requests[0].idempotency);
  expect(f.writes).toEqual([]);
  expect(f.errors).toEqual([]);
});

test("close unknown results survive reload and only exact terminal receipts unlock single pair and all", async ({ page }) => {
  let mode = "single", unavailable = false;
  const actions: any[] = [], requests: any[] = [];
  const f = await setup(page, snapshot => {
    if (mode === "pair") {
      pairPositions(snapshot.positions);
    }
  });
  const envelope = await (await page.request.get(`${API}/api/trading/action-runs`)).json();
  await page.route(`${API}/api/trading/portfolio/snapshot`, route => unavailable
    ? route.fulfill({ status: 504, json: { error: { code: "TIMEOUT", message: "fixture account unavailable" } } })
    : route.fallback());
  await page.route(/\/api\/trading\/action-runs(?:\/[^/]+)?$/, route => {
    const id = new URL(route.request().url()).pathname.split("/").at(-1);
    return route.fulfill({ json: id === "action-runs" ? { ...envelope, data: actions } : actions.find(a => a.id === id) });
  });
  await page.route(/\/api\/trading\/portfolio\/(?:positions\/bitget\/SOLUSDT\/close(?:-pair)?|close-all)$/, route => {
    const headers = route.request().headers(), body = route.request().postDataJSON();
    requests.push({ path: new URL(route.request().url()).pathname, body });
    actions.unshift({ id: `action-recovery-${requests.length}`, kind: mode === "single" ? "portfolio_close_position"
      : mode === "pair" ? "portfolio_close_pair" : "portfolio_close_all", target: mode === "all" ? "all-positions" : "bitget:SOLUSDT",
      requestId: headers["x-request-id"], idempotencyKey: headers["idempotency-key"], status: "accepted",
      actor: "fixture", message: "accepted", result: null, startedAtMs: NOW, updatedAtMs: NOW });
    return route.fulfill({ status: 504, json: { error: { code: "TIMEOUT", message: "fixture response lost", status: 504 } } });
  });
  const recovery = page.getByRole("alert", { name: "平仓操作待核对" });
  const recheck = recovery.getByRole("button", { name: "核对原平仓" });
  const pendingRecords = () => page.evaluate(() => Object.keys(sessionStorage)
    .filter(key => key.startsWith("crossline.settings.pending.v1:position-close:"))
    .map(key => JSON.parse(sessionStorage.getItem(key)!)));

  for (const scope of ["single", "pair", "all"]) {
    mode = scope;
    await page.goto(`/?closeCase=${scope}#positions`);
    await expect(page.locator(".positions-table")).toContainText("SOLUSDT");
    if (scope === "all") {
      await page.getByRole("tab", { name: "控制", exact: true }).click();
      await page.locator("#positions-close-all-confirmation").fill("CLOSE_ALL_POSITIONS");
      await page.getByRole("button", { name: "关闭全部 2 个持仓" }).click();
    } else {
      await page.locator(".positions-table tbody > tr").filter({ hasText: "SOLUSDT" })
        .getByRole("button", { name: scope === "pair" ? "平配对" : "平仓", exact: true }).click();
      await page.getByRole("button", { name: scope === "pair" ? "实盘平配对" : "实盘平仓", exact: true }).click();
    }
    await expect(recheck).toBeEnabled();
    const before = await pendingRecords();
    expect(before).toHaveLength(1);
    expect(Object.keys(before[0]).sort()).toEqual(["context", "kind", "run_id", "target", "version"]);
    await page.reload();
    await expect(recovery).toBeVisible();
    expect(await pendingRecords()).toEqual(before);
    await expect(page.locator(".positions-table").getByRole("button", { name: /^(平仓|平配对)$/ }).first()).toBeDisabled();
    await page.getByRole("tab", { name: "控制", exact: true }).click();
    await page.locator("#positions-close-all-confirmation").fill("CLOSE_ALL_POSITIONS");
    await expect(page.getByRole("button", { name: "关闭全部 2 个持仓" })).toBeDisabled();
    await recheck.click();
    await expect(recovery).toContainText("后端已受理");
    const action = actions[0];
    const result: any = { ...closeRun(`close-recovery-${scope}`, "succeeded", "filled"), scope,
      requestId: action.requestId, actionRunId: action.id, idempotencyKey: action.idempotencyKey, updatedAtMs: NOW + requests.length * 1000 };
    if (scope !== "single") {
      result.expectedLegCount = result.submittedOrderCount = 2;
      result.legs.push({ ...result.legs[0], venue: "binance", symbol: "BTCUSDT", quantity: 0.2 });
    }
    action.status = "succeeded";
    action.result = { ...result, requestId: "wrong-request" };
    await recheck.click();
    await expect(recovery).toContainText("请求返回异常");
    expect(await pendingRecords()).toHaveLength(1);
    // Even a failed action may contain one filled leg and require compensation.
    if (scope === "pair") {
      action.status = "failed";
      action.result = { ...result, status: "failed", submittedOrderCount: 0, failedLegCount: 2,
        legs: result.legs.map((leg: any, index: number) => ({ ...leg, status: "failed",
          problem: { code: index === 0 ? "CLOSE_RUN_PRE_TRADE_REJECTED" : "TIMEOUT", message: "fixture unknown leg" } })) };
      await recheck.click();
      await expect(recheck).toBeEnabled();
      expect(await pendingRecords()).toHaveLength(1);
    }
    action.status = "failed";
    action.result = { ...result, status: "unwind_required", nakedExposureUsd: 15 };
    await recheck.click();
    await expect(recheck).toBeEnabled();
    await expect(recovery).toBeVisible();
    expect(await pendingRecords()).toHaveLength(1);
    if (scope === "single") {
      unavailable = true;
      await page.reload();
      await expect(recheck).toBeEnabled();
      await expect(page.locator(".positions-main")).toContainText("fixture account unavailable");
      await expect(page.locator(".positions-risk-summary")).toContainText("风险快照读取失败");
      for (const width of [1440, 390]) {
        await page.setViewportSize({ width, height: 900 });
        await recheck.click({ trial: true });
        const box = await recovery.boundingBox(), table = await page.locator(".positions-detail-workspace").boundingBox();
        expect(box!.y + box!.height).toBeLessThanOrEqual(table!.y);
        expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
        await page.screenshot({ path: test.info().outputPath(`positions-close-recovery-${width}.png`), fullPage: true });
      }
      await page.setViewportSize({ width: 1440, height: 900 });
    }
    action.status = "succeeded";
    action.result = result;
    await recheck.click();
    await expect(recovery).toHaveCount(0);
    expect(await pendingRecords()).toEqual([]);
    if (scope === "single") {
      await expect(page.locator(".positions-action-status")).toContainText("1 条订单已确认成交");
      unavailable = false;
      const snapshot = f.snapshot();
      snapshot.serverNowMs = NOW + 10_000;
      f.send(snapshot);
    }
    await page.getByRole("tab", { name: "平仓", exact: true }).click();
    const record = page.getByRole("region", { name: "平仓记录", exact: true }).locator("details").filter({ hasText: result.id });
    await expect(record.locator("summary")).toContainText("已完成");
    await expect(page.locator(".positions-table")).toContainText("SOLUSDT");
    expect(requests).toHaveLength(["single", "pair", "all"].indexOf(scope) + 1);
  }
  expect(requests.map(r => r.body.expectedLegCount)).toEqual([1, 2, 2]);
  await page.getByRole("tab", { name: "控制", exact: true }).click();
  await page.locator("#positions-close-all-confirmation").fill("CLOSE_ALL_POSITIONS");
  await page.getByRole("button", { name: "关闭全部 2 个持仓" }).click();
  await expect(recheck).toBeEnabled();
  actions[0].status = "failed";
  actions[0].problem = { code: "CLOSE_RUN_STALE_SNAPSHOT", message: "fixture rejected before submission", status: 409 };
  await recheck.click();
  await expect(recovery).toHaveCount(0);
  await expect(page.locator(".close-all-control")).toContainText("平仓未提交");
  expect(await pendingRecords()).toEqual([]);
  expect(requests).toHaveLength(4);
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("close confirmation preserves missing mark evidence and can be cancelled without submitting", async ({ page }) => {
  const fixture = await setup(page, (snapshot) => {
    snapshot.positions[0].origin = "account_private";
    snapshot.accountState.fieldQuality = [{
      subject: { kind: "position", venue: "binance", symbol: "BTCUSDT", side: "long" },
      field: "markPrice", status: "missing", source: "account_position_runtime", observedAtMs: NOW,
      problem: { code: "ACCOUNT_FIELD_UNKNOWN", message: "mark price unavailable" },
    }];
  });
  await page.goto("/#positions");
  await expect(page.locator(".positions-table")).toContainText("BTCUSDT");
  const close = page.getByRole("button", { name: "平仓", exact: true }).first();
  await close.click();
  const confirmation = page.getByRole("group", { name: /^确认平仓：binance BTCUSDT/ });
  await expect(confirmation).toBeVisible();
  const facts = confirmation.locator(".position-close-confirmation-facts");
  await expect(facts.locator("strong").nth(0)).toHaveText("数据待确认");
  await expect(facts.locator("strong").nth(1)).toHaveText("数据待确认");
  await expect(facts.locator("strong").nth(2)).toHaveText("数据待确认");
  await confirmation.getByRole("button", { name: "取消", exact: true }).click();
  await expect(confirmation).toBeHidden();
  await expect(close).toBeFocused();
  expect(fixture.writes).toEqual([]);
  expect(fixture.errors).toEqual([]);
});

function assetSnapshot(snapshot: any) {
  const balance = (venue: string, currency: string, total: number) => ({ venue, currency, total,
    available: total, frozen: 0, unrealizedPnl: 0 });
  const rows = [balance("binance", "USDT", 3215.25), balance("binance", "BTC", 0.04),
    balance("binance", "SMALL", 0.5), balance("bitget", "USDC", 251.34),
    balance("bitget", "UNPRICED", 123.456)];
  const valuation = (venue: string, currency: string, usdValue: number) => ({ venue, currency,
    usdValue, source: "isolated-fixture", observedAtMs: NOW });
  Object.assign(snapshot.accountState.balances, { rows, rowCount: rows.length, status: "fresh", problems: [], observedAtMs: NOW,
    accountSummaries: [], assetValuations: [valuation("binance", "USDT", 3215.25),
      valuation("binance", "BTC", 2400), valuation("binance", "SMALL", 0.5),
      valuation("bitget", "USDC", 251.34)] });
  snapshot.balances = rows;
}

test("assets stay aligned and keep account selection and expanded balances through WS updates", async ({ page }) => {
  const f = await setup(page, assetSnapshot);
  await page.goto("/#positions");
  await page.getByRole("tab", { name: "资产", exact: true }).click();
  await expect(page.getByLabel("选择交易所账户")).toBeVisible();
  for (const width of [1440, 1024, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await page.evaluate(() => window.scrollTo(0, 0));
    const panel = page.locator("#positions-detail-balances");
    expect(await panel.evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
    expect(await page.locator(".balance-table").first().evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
    await page.screenshot({ path: test.info().outputPath(`assets-${width}.png`), fullPage: true });
  }
  const selector = page.getByLabel("选择交易所账户");
  await selector.selectOption("bitget");
  await page.getByText("待估值资产", { exact: true }).click();
  const unvalued = page.locator(".balance-unvalued-disclosure");
  await expect(unvalued).toHaveAttribute("open", "");
  await selector.focus();
  const update = f.snapshot();
  update.serverNowMs = NOW + 100;
  update.snapshotVersion = "assets-updated";
  update.accountState.balances.rows.find((r: any) => r.currency === "UNPRICED").available = 120;
  await expect.poll(() => f.sockets.size).toBeGreaterThan(0);
  f.send(update);
  await expect(unvalued.locator(".balance-amount-cell").nth(1)).toContainText("120");
  await expect(selector).toHaveValue("bitget");
  await expect(selector).toBeFocused();
  await expect(unvalued).toHaveAttribute("open", "");
  f.send({ status: "error", source: "isolated-fixture", observedAtMs: NOW + 200,
    problem: { code: "TIMEOUT", message: "fixture refresh timeout", status: 504 } });
  const assets = page.locator("#positions-detail-balances");
  await expect(assets).toContainText("余额刷新失败");
  await expect(assets.locator(".balance-evidence-chip").first()).toContainText("DEGRADED");
  await expect(unvalued).toHaveAttribute("open", "");
  f.send({ ...update, serverNowMs: NOW + 300, snapshotVersion: "assets-recovered" });
  await expect(assets).not.toContainText("余额刷新失败");
  await expect(assets.locator(".balance-evidence-chip").first()).toContainText("FRESH");
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("asset evidence stays open and focused while balances and quality update", async ({ page }) => {
  const f = await setup(page, (snapshot) => {
    assetSnapshot(snapshot);
    snapshot.accountState.fieldQuality = [{
      subject: { kind: "balance", venue: "binance", currency: "USDT" },
      field: "available", status: "estimated", source: "fixture", observedAtMs: NOW,
    }];
  });
  await page.goto("/#positions");
  await page.getByRole("tab", { name: "资产", exact: true }).click();
  const row = page.locator(".balance-row").filter({ has: page.locator(".balance-asset-cell strong", { hasText: /^USDT$/ }) });
  const detail = row.locator(".balance-row-diagnostics");
  const summary = detail.locator("summary");
  await summary.click();
  await summary.focus();
  await row.evaluate((el) => { (window as any).__assetRow = el; });
  const update = f.snapshot();
  update.serverNowMs = NOW + 100;
  update.snapshotVersion = "asset-evidence-update";
  update.accountState.balances.rows[0].available = 3000;
  update.accountState.balances.assetValuations.find((r: any) => r.currency === "BTC").usdValue = 5000;
  update.accountState.fieldQuality[0].status = "missing";
  await expect.poll(() => f.sockets.size).toBeGreaterThan(0);
  f.send(update);
  await expect(row.locator(".balance-amount-cell").nth(1).locator("strong")).toHaveText("未知");
  await expect(detail).toHaveAttribute("open", "");
  await expect(summary).toBeFocused();
  expect(await row.evaluate((el) => (window as any).__assetRow === el)).toBe(true);
  await expect(detail).toContainText("MISSING");
  await expect(page.locator(".balance-row").first()).toContainText("USDT");
  await expect(page.locator(".balance-row").filter({ hasText: "BTC" }).locator(".balance-value-cell")).toContainText("$5000.00");
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    expect(await page.locator(".balance-account-detail").evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
    await page.screenshot({ path: test.info().outputPath(`asset-evidence-${width}.png`), fullPage: true });
  }
  update.serverNowMs += 100;
  update.snapshotVersion = "asset-evidence-recovered";
  update.accountState.fieldQuality = [];
  f.send(update);
  await expect(row.locator(".balance-amount-cell").nth(1).locator("strong")).toHaveText("3000");
  await expect(detail).toBeHidden();
  update.serverNowMs += 100;
  update.snapshotVersion = "asset-hidden";
  Object.assign(update.accountState.balances.rows[0], { available: 0, total: 0 });
  update.accountState.balances.assetValuations[0].usdValue = 0;
  f.send(update);
  await expect(row).toHaveCount(0);
  update.serverNowMs += 100;
  update.snapshotVersion = "asset-returned";
  Object.assign(update.accountState.balances.rows[0], { available: 2000, total: 2000 });
  update.accountState.balances.assetValuations[0].usdValue = 2000;
  f.send(update);
  await expect(row.locator(".balance-amount-cell").nth(0).locator("strong")).toHaveText("2000");
  await page.getByLabel("选择交易所账户").selectOption("bitget");
  await expect(page.locator(".balance-venue-header")).toContainText("BITGET");
  await expect(page.locator(".balance-account-detail")).not.toContainText("USDT");
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("missing account positions are not treated as a risk-free empty account after a previous close", async ({ page }) => {
  const f = await setup(page, (snapshot) => {
    assetSnapshot(snapshot);
    snapshot.positions = [];
    snapshot.degraded = true;
    snapshot.accountState.positions.status = "degraded";
    snapshot.accountState.positions.problems = [{ code: "TIMEOUT", message: "fixture position read failed" }];
    snapshot.summary.navEvidence.status = "missing";
    snapshot.summary.totalNavUsd = 0;
    snapshot.summary.netDeltaUsd = 0;
    snapshot.summary.nakedExposureUsd = 0;
    snapshot.summary.pnlBreakdown.evidence = { quality: "missing", source: "fixture", observedAtMs: NOW,
      realizedGroupCount: 0, closeRunCount: 0, unwindRunCount: 0 };
    snapshot.risk.fundingClustering = [{ settlesInMinutes: 60, positionCount: 0, totalOutflowUsd: 0, symbols: [] }];
    snapshot.accountState.fieldQuality = [{
      subject: { kind: "balance", venue: "binance", currency: "USDT" },
      field: "available", status: "missing", source: "fixture", observedAtMs: NOW,
    }];
    snapshot.accountState.balances.rows[0].available = 0;
  });
  await page.goto("/#positions");
  await expect(page.locator(".positions-main")).toContainText("fixture position read failed");
  const cards = page.locator(".summary-card");
  for (let i = 0; i < 4; i++) await expect(cards.nth(i).locator(":scope > strong")).toHaveText("未知");
  await expect(page.locator(".compact-risk-list")).toContainText("持仓数据待确认");
  await expect(page.locator(".compact-risk-list")).not.toContainText("无结算");
  await page.getByRole("tab", { name: "资产", exact: true }).click();
  const usdt = page.locator(".balance-row").filter({ has: page.locator(".balance-asset-cell strong", { hasText: /^USDT$/ }) });
  await expect(usdt.locator(".balance-amount-cell").nth(1).locator("strong")).toHaveText("未知");
  await expect(usdt.locator(".balance-amount-cell").nth(0)).toContainText("3215.25");
  await page.screenshot({ path: test.info().outputPath("assets-missing.png"), fullPage: true });
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("funding windows stay unconfirmed when an otherwise current position lacks settlement evidence", async ({ page }) => {
  const f = await setup(page, (snapshot) => {
    snapshot.positions[0].nextFundingMs = null;
    snapshot.positions[0].secondsUntilFunding = null;
    snapshot.risk.fundingClustering = [{ settlesInMinutes: 60, positionCount: 0, totalOutflowUsd: 0 }];
  });
  await page.goto("/#positions");
  const funding = page.locator(".compact-risk-row").filter({ hasText: "临近 资金费" });
  await expect(funding).toContainText("1 个仓位待补结算数据依据");
  await expect(funding).toContainText("待确认");
  await expect(funding).not.toContainText("无结算");
  await page.getByRole("tab", { name: "风险", exact: true }).click();
  await expect(page.locator("#positions-detail-risk")).toContainText("暂不汇总 资金费");
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("long positions remain usable while incidents have independent stable drafts", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const f = await setup(page, (snapshot) => {
    snapshot.positions = Array.from({ length: 20 }, (_, i) => position("bitget", `ASSET${i}USDT`, 0.1, 100 + i));
    snapshot.recentCloseRuns = [incident("incident-a"), incident("incident-b", "compensation_failed")];
  });
  await page.goto("/#positions");
  await expect(page.locator(".positions-table")).toContainText("ASSET0USDT");
  const table = page.locator(".positions-table-wrap");
  expect((await table.boundingBox())!.height).toBeGreaterThan(200);
  await expect(page.locator(".positions-incident-notice")).toContainText("2 笔平仓待处理");
  await page.locator(".positions-table .row-close-button").last().scrollIntoViewIfNeeded();
  const lastClose = await page.locator(".positions-table .row-close-button").last().boundingBox();
  expect(lastClose!.x + lastClose!.width).toBeLessThanOrEqual(1440);
  expect(lastClose!.y + lastClose!.height).toBeLessThanOrEqual(900);
  await table.evaluate((el) => { el.scrollTop = 0; });
  await page.screenshot({ path: test.info().outputPath("positions-long-1440.png"), fullPage: true });
  await page.getByRole("button", { name: "查看处理", exact: true }).click();
  const a = page.locator('.close-incident[data-run-id="incident-a"]');
  const b = page.locator('.close-incident[data-run-id="incident-b"]');
  await a.locator("summary").click();
  await a.getByRole("textbox", { name: "补偿确认短语", exact: true }).fill("COMPENSATE_CLOSE_RUN");
  await expect(a.getByRole("button", { name: "补买 #1", exact: true })).toBeEnabled();
  await b.locator("summary").click();
  await b.getByRole("textbox", { name: "处理原因", exact: true }).fill("checked account, fixture only");
  await b.getByRole("textbox", { name: "人工终结确认短语", exact: true }).fill("MANUAL_TERMINATE_CLOSE_RUN");
  await b.getByRole("spinbutton").fill("-1");
  await expect(b.getByRole("button", { name: "记录人工终结" })).toBeDisabled();
  await b.getByRole("spinbutton").fill("0.25");
  await expect(b.getByRole("button", { name: "记录人工终结" })).toBeEnabled();
  await expect.poll(() => f.sockets.size).toBeGreaterThan(0);
  const updated = { ...incident("incident-a"), updatedAtMs: NOW + 100, message: "new unrelated receipt detail" };
  f.send({ event: "close_run_updated", closeRun: updated, timestampMs: NOW + 100 });
  await expect(a.locator("summary")).toContainText("new unrelated receipt detail");
  await expect(a.getByRole("textbox", { name: "补偿确认短语", exact: true })).toHaveValue("COMPENSATE_CLOSE_RUN");
  await expect(b.getByRole("textbox", { name: "处理原因", exact: true })).toHaveValue("checked account, fixture only");
  await expect(b.getByRole("spinbutton")).toHaveValue("0.25");
  await a.locator("summary").click();
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    const region = page.getByRole("region", { name: "待处理平仓", exact: true });
    expect(await region.evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
    await page.locator("#positions-detail-activity").evaluate((el) => { el.scrollTop = 0; });
    if (width === 1440) {
      const action = await b.getByRole("button", { name: "记录人工终结" }).boundingBox();
      const panel = await page.locator("#positions-detail-activity").boundingBox();
      expect(action!.y + action!.height).toBeLessThanOrEqual(panel!.y + panel!.height);
    }
    await page.screenshot({ path: test.info().outputPath(`incidents-${width}.png`), fullPage: true });
  }
  f.send({ event: "close_run_updated", closeRun: { ...updated, snapshotVersion: "changed-plan", updatedAtMs: NOW + 200 }, timestampMs: NOW + 200 });
  await a.locator("summary").click();
  await expect(a.getByRole("textbox", { name: "补偿确认短语", exact: true })).toHaveValue("");
  await expect(a.getByRole("button", { name: "补买 #1", exact: true })).toBeDisabled();
  await expect(b.getByRole("textbox", { name: "人工终结确认短语", exact: true })).toHaveValue("MANUAL_TERMINATE_CLOSE_RUN");
  f.send({ status: "error", source: "isolated-fixture", observedAtMs: NOW + 300,
    problem: { code: "TIMEOUT", message: "isolated snapshot timeout", status: 504 } });
  await expect(b.getByRole("button", { name: "记录人工终结" })).toBeDisabled();
  await expect(page.getByRole("region", { name: "待处理平仓" })).toContainText("记录已过期，暂停提交");
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});


test("compensation cancel and manual recovery keep exact requests across navigation and reload", async ({ page }) => {
  let current: any = incident("incident-recovery");
  let unavailable = false;
  const f = await setup(page, snapshot => { snapshot.recentCloseRuns = [structuredClone(current)]; });
  const envelope = await (await page.request.get(`${API}/api/trading/action-runs`)).json();
  const actions: any[] = [], mutations: any[] = [];
  let currentOrder: any;
  let releaseSubmit!: () => void;
  const hold = new Promise<void>(resolve => { releaseSubmit = resolve; });
  await page.route(/\/api\/trading\/action-runs(?:\/[^/]+)?$/, route => {
    const id = new URL(route.request().url()).pathname.split("/").at(-1);
    return route.fulfill({ json: id === "action-runs" ? { ...envelope, data: actions } : actions.find(a => a.id === id) });
  });
  await page.route(`${API}/api/trading/portfolio/snapshot*`, route => unavailable
    ? route.fulfill({ status: 503, json: { error: { code: "TIMEOUT", message: "fixture account unavailable" } } })
    : route.fallback());
  await page.route(`${API}/api/trading/portfolio/close-runs/incident-recovery/compensation-orders`, async route => {
    mutations.push(route.request().postDataJSON());
    current = compensationReceipt("incident-recovery", "act-remedy-comp");
    currentOrder = structuredClone(current.unwindPlan.compensationAttempts[0].order);
    actions.push(remedyAction(route.request(), "portfolio_close_compensation", current.id, "act-remedy-comp", structuredClone(current)));
    await hold;
    return route.fulfill({ status: 504, json: { error: { code: "TIMEOUT", message: "fixture lost submit receipt" } } });
  });
  await page.route(`${API}/api/trading/orders/comp-order-1/cancel`, route => {
    mutations.push({ cancel: "comp-order-1" });
    currentOrder.state = "cancel_requested";
    currentOrder.updatedAtMs = NOW + 300;
    actions.push({ ...remedyAction(route.request(), "trading_order_cancel", "comp-order-1", "act-remedy-cancel", structuredClone(currentOrder)), status: "succeeded" });
    return route.fulfill({ status: 504, json: { error: { code: "TIMEOUT", message: "fixture lost cancel receipt" } } });
  });
  await page.route(`${API}/api/trading/orders/comp-order-1`, route => route.fulfill({ json: currentOrder }));
  await page.route(`${API}/api/trading/portfolio/close-runs/incident-recovery/manual-terminal`, route => {
    const body = route.request().postDataJSON();
    mutations.push(body);
    current = structuredClone(current);
    current.status = "manually_resolved";
    current.updatedAtMs = NOW + 500;
    current.message = "人工处理结果已记录";
    current.unwindPlan.status = "manual_terminal_recorded";
    current.unwindPlan.nextActions = [];
    current.unwindPlan.manualTerminalEvidence = { actionRunId: "act-remedy-manual", actor: "fixture",
      reason: body.reason, snapshotVersion: body.snapshotVersion, recordedAtMs: NOW + 500,
      evidence: body.evidence, manualHandlingCostUsd: body.manualHandlingCostUsd };
    actions.push({ ...remedyAction(route.request(), "portfolio_close_manual_terminal", current.id, "act-remedy-manual", structuredClone(current)), status: "succeeded" });
    return route.fulfill({ status: 504, json: { error: { code: "TIMEOUT", message: "fixture lost manual receipt" } } });
  });
  const primary = page.getByRole("alert", { name: "补偿 / 人工终结", exact: true });
  const cancel = page.getByRole("alert", { name: "补偿撤单", exact: true });
  const originalRecords = () => page.evaluate(() => Object.keys(sessionStorage)
    .filter(key => key.includes(":position-remedy"))
    .map(key => JSON.parse(sessionStorage.getItem(key)!)));
  const openIncident = async () => {
    await page.getByRole("tab", { name: "平仓", exact: true }).click();
    const row = page.locator('.close-incident[data-run-id="incident-recovery"]');
    if (await row.getAttribute("open") === null) await row.locator("summary").click();
    return row;
  };
  await page.goto("/#positions");
  let row = await openIncident();
  await row.getByRole("textbox", { name: "补偿确认短语", exact: true }).fill("COMPENSATE_CLOSE_RUN");
  await row.getByRole("button", { name: "补买 #1", exact: true }).click();
  await expect(primary).toContainText("处理中");
  const original = await originalRecords();
  await page.evaluate(() => { location.hash = "settings"; });
  await expect(page.locator(".settings-workspace")).toBeVisible();
  await page.evaluate(() => { location.hash = "positions"; });
  await expect(primary).toContainText("处理中");
  row = await openIncident();
  await expect(row.getByRole("button", { name: "补买 #1", exact: true })).toHaveCount(0);
  expect(mutations).toHaveLength(1);
  releaseSubmit();
  await expect(primary).toContainText("结果待核对");
  await page.reload();
  await expect(primary.getByRole("button")).toBeEnabled();
  expect(await originalRecords()).toEqual(original);
  const expectedRequest = actions[0].requestId;
  actions[0].requestId = "another-request";
  await primary.getByRole("button").click();
  await expect(primary.getByRole("button")).toBeEnabled();
  expect(await originalRecords()).toHaveLength(1);
  actions[0].requestId = expectedRequest;
  actions[0].result.unwindPlan.compensationAttempts[0].actionRunId = "unrelated-compensation";
  await primary.getByRole("button").click();
  await expect(primary).toContainText("REMEDY_RECEIPT_MISMATCH");
  actions[0].result = structuredClone(current);
  actions[0].result.unwindPlan.compensationAttempts[0].status = "filled";
  await primary.getByRole("button").click();
  await expect(primary).toContainText("REMEDY_RECEIPT_MISMATCH");
  expect(await originalRecords()).toHaveLength(1);
  actions[0].result = structuredClone(current);
  await primary.getByRole("button").click();
  await expect(primary).toContainText("等待订单最终结果");
  row = await openIncident();
  await expect(row.getByRole("button", { name: "撤补买", exact: true })).toBeEnabled();
  await row.getByRole("button", { name: "撤补买", exact: true }).click();
  await expect(cancel).toContainText("结果待核对");
  await page.reload();
  await cancel.getByRole("button").click();
  await expect(cancel).toContainText("等待交易所最终结果");
  expect(await originalRecords()).toHaveLength(2);
  row = await openIncident();
  await expect(row.getByRole("button", { name: "撤补买", exact: true })).toBeDisabled();
  currentOrder.intent.id = "unrelated-order";
  await cancel.getByRole("button").click();
  await expect(cancel).toContainText("REMEDY_RECEIPT_MISMATCH");
  currentOrder.intent.id = "comp-order-1";
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await cancel.getByRole("button").click({ trial: true });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    await page.screenshot({ path: test.info().outputPath(`positions-remedy-${width}.png`), fullPage: true });
  }
  await page.setViewportSize({ width: 1440, height: 900 });
  currentOrder.state = "cancelled";
  currentOrder.updatedAtMs = NOW + 400;
  await cancel.getByRole("button").click();
  await expect(cancel).toHaveCount(0);
  await expect(row.getByRole("button", { name: "撤补买", exact: true })).toBeDisabled();
  current = { ...current, status: "compensation_failed", updatedAtMs: NOW + 400 };
  current.unwindPlan.status = "compensation_failed";
  Object.assign(current.unwindPlan.compensationAttempts[0], {
    status: "cancelled", order: structuredClone(currentOrder), updatedAtMs: NOW + 400,
  });
  current.unwindPlan.nextActions = [{ kind: "manual_incident_review", label: "人工复核", requiresConfirmation: true, requiredEvidence: [] }];
  actions[0].result = structuredClone(current);
  f.send({ event: "close_run_updated", closeRun: current, timestampMs: NOW + 400 });
  await expect(primary).toHaveCount(0);
  expect(await originalRecords()).toHaveLength(0);
  row = await openIncident();
  await row.getByRole("textbox", { name: "处理原因", exact: true }).fill("已在隔离夹具确认风险处理");
  await row.getByRole("textbox", { name: "数据依据编号", exact: true }).fill("fixture-evidence-only");
  await row.getByRole("spinbutton", { name: "人工成本 USD（选填）", exact: true }).fill("0.25");
  await row.getByRole("textbox", { name: "人工终结确认短语", exact: true }).fill("MANUAL_TERMINATE_CLOSE_RUN");
  await row.getByRole("button", { name: "记录人工终结", exact: true }).click();
  await expect(primary).toContainText("结果待核对");
  unavailable = true;
  await page.reload();
  await expect(primary.getByRole("button")).toBeEnabled();
  actions[2].result.unwindPlan.manualTerminalEvidence.actionRunId = "different-manual-action";
  await primary.getByRole("button").click();
  await expect(primary).toContainText("REMEDY_RECEIPT_MISMATCH");
  actions[2].result = structuredClone(current);
  await primary.getByRole("button").click();
  await expect(primary).toHaveCount(0);
  expect(await originalRecords()).toEqual([]);
  unavailable = false;
  const snapshot = f.snapshot();
  snapshot.serverNowMs = NOW + 1000;
  snapshot.recentCloseRuns = [current];
  f.send(snapshot);
  await page.getByRole("tab", { name: "平仓", exact: true }).click();
  await expect(page.getByRole("region", { name: "平仓记录", exact: true })).toContainText("人工");
  await expect(page.locator(".positions-table")).toContainText("SOLUSDT");
  expect(mutations).toHaveLength(3);
  expect(mutations[2]).toMatchObject({ manualHandlingCostUsd: 0.25, evidence: ["fixture-evidence-only"] });
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

function compensationReceipt(id: string, actionId: string): any {
  const run: any = incident(id, "compensation_submitted");
  run.updatedAtMs = NOW + 200;
  run.unwindPlan.status = "compensation_submitted";
  run.unwindPlan.nextActions = [];
  run.unwindPlan.compensationAttempts = [{
    actionRunId: actionId, venue: "bitget", symbol: "SOLUSDT", side: "long",
    compensationOrderSide: "buy", targetQuantity: 0.1, status: "accepted",
    submittedAtMs: NOW + 100, updatedAtMs: NOW + 200,
    order: { intent: { id: "comp-order-1", source: "close_run_compensation", mode: "dry_run",
      exchange: "bitget", symbol: "SOLUSDT", side: "buy", orderType: "limit", quantity: 0.1,
      price: 150, clientOrderId: "comp-client-1", createdAtMs: NOW + 100 },
      state: "accepted", updatedAtMs: NOW + 200 },
  }];
  return run;
}

function remedyAction(request: any, kind: string, target: string, id: string, result: any) {
  return { id, kind, target, status: "accepted", actor: "isolated-fixture",
    requestId: request.headers()["x-request-id"], idempotencyKey: request.headers()["idempotency-key"],
    message: "isolated receipt", startedAtMs: NOW, updatedAtMs: NOW + 200, result };
}

test("compensation HTTP receipt updates controls without waiting for a WS event", async ({ page }) => {
  const f = await setup(page, (snapshot) => { snapshot.recentCloseRuns = [incident("incident-http")]; });
  const requests: any[] = [];
  const envelope = await (await page.request.get(`${API}/api/trading/action-runs`)).json();
  let action: any;
  await page.route(/\/api\/trading\/action-runs(?:\/[^/]+)?$/, route => route.fulfill({
    json: route.request().url().endsWith("/action-runs") ? { ...envelope, data: action ? [action] : [] } : action,
  }));
  await page.route(`${API}/api/trading/portfolio/close-runs/incident-http/compensation-orders`, async (route) => {
    requests.push(route.request().postDataJSON());
    const receipt = compensationReceipt("incident-http", "act-comp-http");
    action = remedyAction(route.request(), "portfolio_close_compensation", "incident-http", "act-comp-http", receipt);
    return route.fulfill({ json: receipt });
  });
  await page.goto("/#positions");
  await page.getByRole("button", { name: "查看处理" }).click();
  const record = page.locator('.close-incident[data-run-id="incident-http"]');
  await record.locator("summary").click();
  await record.getByRole("textbox", { name: "补偿确认短语", exact: true }).fill("COMPENSATE_CLOSE_RUN");
  await record.getByRole("button", { name: "补买 #1", exact: true }).click();
  await expect(record.locator("summary")).toContainText("补偿中");
  await expect(record.getByRole("button", { name: "补买 #1", exact: true })).toHaveCount(0);
  expect(requests).toHaveLength(1);
  expect(requests[0]).toMatchObject({ confirmationPhrase: "COMPENSATE_CLOSE_RUN", snapshotVersion: "close-fixture", candidateIndex: 0, targetQuantity: 0.1, limitPrice: 150 });
  await expect.poll(() => f.sockets.size).toBeGreaterThan(0);
  f.send(f.snapshot());
  await expect(record.locator("summary")).toContainText("补偿中");
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("protection states distinguish configured rules from stale positions", async ({ page }) => {
  const f = await setup(page);
  let failStatus = false;
  await page.route(`${API}/api/trading/status`, async (route) => {
    if (failStatus) return route.fulfill({ status: 504, json: { code: "TIMEOUT", message: "isolated protection config timeout" } });
    const body = await (await route.fetch()).json();
    body.risk.autoProfitClose = { enabled: true, minNetProfitUsd: 0.25, minRoiBps: 10, maxNetLossUsd: 1.5, maxLossRoiBps: 20, exitBufferBps: 5, confirmationSamples: 3, cooldownSecs: 60 };
    return route.fulfill({ json: body });
  });
  await page.goto("/#positions");
  await page.getByRole("tab", { name: "控制", exact: true }).click();
  const protection = page.locator(".pair-protection-bar");
  await expect(protection).toContainText("等待配对");
  await expect(protection).toContainText("净收益 >= $0.25 且收益率 >= 0.1%");
  await expect(protection).toContainText("亏损 >= $1.5 或亏损率 >= 0.2%");
  await expect(protection).not.toContainText("监控中");
  await expect.poll(() => f.sockets.size).toBeGreaterThan(0);
  f.send({ status: "error", source: "isolated-fixture", observedAtMs: NOW + 300,
    problem: { code: "TIMEOUT", message: "isolated snapshot timeout", status: 504 } });
  await expect(protection).toContainText("持仓待确认");
  await page.screenshot({ path: test.info().outputPath("controls-1440.png"), fullPage: true });
  await page.setViewportSize({ width: 390, height: 900 });
  expect(await protection.evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
  await page.screenshot({ path: test.info().outputPath("controls-390.png"), fullPage: true });
  failStatus = true;
  await page.reload();
  await page.getByRole("tab", { name: "控制", exact: true }).click();
  await expect(protection).toContainText("配置读取失败");
  await expect(protection).toContainText("isolated protection config timeout");
  await expect(protection).not.toContainText("正在读取");
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("positions and settings share risk locks, recover lost receipts and retain decimal protection", async ({ page }) => {
  const f = await riskFixture(page);
  let portfolioFails = false;
  await page.route(`${API}/api/trading/portfolio/nav-history*`, (route) => route.fulfill({
    json: { count: 0, rows: [], source: "isolated-fixture", observedAtMs: NOW },
  }));
  await page.route(`${API}/api/trading/portfolio/snapshot*`, async (route) => {
    if (portfolioFails) return route.fulfill({ status: 503,
      json: { error: { code: "TIMEOUT", message: "fixture portfolio unavailable" } } });
    const body = await (await route.fetch()).json();
    Object.assign(body.snapshot, { serverNowMs: NOW, degraded: false, problems: [],
      positions: [position("bitget", "SOLUSDT", 0.1, 150)] });
    body.snapshot.operationHealth = [{ venue: "bitget", operation: "positions", status: "ok",
      source: "isolated-fixture", message: "fixture", supported: true, configured: true, observedAtMs: NOW }];
    Object.assign(body.snapshot.accountState.positions, { status: "fresh", problems: [], rows: [],
      fieldQuality: [], rowHealth: [], observedAtMs: NOW });
    // Portfolio deliberately lags behind the newer trading-status receipt.
    body.snapshot.risk.hardLimits.killSwitchActive = false;
    body.snapshot.risk.hardLimits.openOrdersUsed = 99;
    return route.fulfill({ json: body });
  });
  const navigate = async (module: string) => {
    await page.getByRole("navigation", { name: "功能模块", exact: true })
      .locator(`button[data-module="${module}"]`).click();
    if (module === "positions") await page.getByRole("tab", { name: "控制", exact: true }).click();
  };
  const control = page.locator(".kill-switch-control");
  const toggle = control.locator(":scope > button");
  const recovery = page.getByRole("alert", { name: "设置操作待核对" });
  const save = page.getByRole("button", { name: "保存风控", exact: true });
  const settingsKill = page.getByRole("button", { name: /^(切换 Kill Switch|更新中)$/ });
  const check = () => recovery.getByRole("button", { name: "核对上次操作", exact: true }).click();
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/#positions");
  await page.getByRole("tab", { name: "控制", exact: true }).click();
  await expect(toggle).toHaveText("开启总闸");
  f.hold();
  await toggle.click();
  await expect.poll(() => f.calls.length).toBe(1);
  expect(f.calls[0].body).toMatchObject({ active: true, expectedActive: false,
    expectedOpenOrderCount: f.status.openOrderCount, reason: "positions.kill_switch.enable" });
  await navigate("settings");
  await expect(settingsKill).toBeDisabled();
  await expect(save).toBeDisabled();
  f.release();
  await expect(settingsKill).toBeEnabled();
  await navigate("positions");
  await expect(toggle).toHaveText("关闭总闸");
  await expect(toggle).toBeEnabled();
  await expect(page.locator(".compact-risk-row").filter({ hasText: "Kill switch" })).toContainText("已开启");
  await page.getByRole("tab", { name: "风险", exact: true }).click();
  await expect(page.locator(".risk-limits .kill-switch")).toHaveText("Kill Switch 开启");
  await page.getByRole("tab", { name: "控制", exact: true }).click();
  expect(f.calls).toHaveLength(1);

  f.fail(true);
  await toggle.click();
  await expect(recovery).toContainText("切换总闸结果待核对");
  await expect(toggle).toBeDisabled();
  expect(f.calls).toHaveLength(2);
  const lost = f.actions.data[0];
  await navigate("settings");
  await expect(save).toBeDisabled();
  await expect(settingsKill).toBeDisabled();
  portfolioFails = true;
  await page.reload();
  await navigate("positions");
  // A missing account snapshot must not hide the existing risk operation.
  await expect(recovery).toContainText("切换总闸结果待核对");
  await expect(toggle).toBeDisabled();
  lost.status = "accepted";
  await check();
  await expect(recovery).toContainText("后端已受理");
  await expect(toggle).toBeDisabled();
  await expect(control).toContainText("持仓待确认");
  await expect(page.locator(".close-all-control")).toHaveCount(0);
  await control.getByText("最近操作详情", { exact: true }).click();
  await expect(control.locator("details")).toContainText(f.calls[1].requestId!);
  await control.getByText("最近操作详情", { exact: true }).click();
  await expect(page.locator(".toast-item")).toHaveCount(0);
  expect(f.calls).toHaveLength(2);
  await recovery.scrollIntoViewIfNeeded();
  await page.screenshot({ path: test.info().outputPath("risk-recovery-desktop.png") });
  await page.setViewportSize({ width: 390, height: 844 });
  await recovery.scrollIntoViewIfNeeded();
  await expect(recovery.getByRole("button")).toBeInViewport({ ratio: 1 });
  expect(await recovery.getByRole("button").evaluate((el) => {
    const rect = el.getBoundingClientRect();
    return el.contains(document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2));
  })).toBe(true);
  expect(await recovery.evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
  await page.screenshot({ path: test.info().outputPath("risk-recovery-mobile.png") });
  await page.setViewportSize({ width: 1440, height: 900 });
  lost.status = "succeeded";
  f.status.risk.killSwitchActive = true;
  await check();
  await expect(recovery).toHaveCount(0);
  await expect(toggle).toHaveText("关闭总闸");
  await expect(toggle).toBeEnabled();
  expect(f.calls).toHaveLength(2);

  portfolioFails = false;
  f.fail(false);
  await page.getByRole("button", { name: "调整保护", exact: true }).click();
  await expect(page).toHaveURL(/#settings/);
  const profit = page.getByLabel(/最低净利润 USD/);
  await expect(profit).toHaveValue("0.5");
  await profit.fill("0.125");
  await page.getByRole("checkbox", { name: /^自动止盈并平双边/ }).check();
  f.hold();
  await save.click();
  await expect.poll(() => f.calls.length).toBe(3);
  await expect(profit).toBeDisabled();
  await navigate("positions");
  await expect(toggle).toBeDisabled();
  await expect(recovery).toContainText("保存风控处理中");
  f.release();
  await expect(recovery).toHaveCount(0);
  await expect(toggle).toBeEnabled();
  const protection = page.locator(".pair-protection-bar");
  await expect(protection).toContainText("$0.125");
  await expect(protection).toContainText("等待配对");
  await navigate("settings");
  await expect(profit).toHaveValue("0.125");
  await expect(save).toBeEnabled();
  await navigate("positions");
  await control.scrollIntoViewIfNeeded();
  await expect(toggle).toBeInViewport();
  await page.screenshot({ path: test.info().outputPath("risk-controls-desktop.png") });
  await page.setViewportSize({ width: 390, height: 844 });
  await toggle.scrollIntoViewIfNeeded();
  await expect(toggle).toBeInViewport({ ratio: 1 });
  await toggle.click({ trial: true });
  await expect(page.locator(".toast-item")).toHaveCount(0);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
  await page.screenshot({ path: test.info().outputPath("risk-controls-mobile.png") });
  await page.route(`${API}/api/trading/status`, (route) => route.fulfill({ status: 503,
    json: { error: { code: "TIMEOUT", message: "fixture risk state unavailable" } } }));
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.reload();
  await page.getByRole("tab", { name: "控制", exact: true }).click();
  await expect(toggle).toHaveText("等待快照");
  await expect(toggle).toBeDisabled();
  await expect(control.locator("header strong")).toHaveText("未知");
  await expect(page.locator(".compact-risk-row").filter({ hasText: "Kill switch" })).toContainText("待确认");
  await page.getByRole("tab", { name: "风险", exact: true }).click();
  await expect(page.locator(".risk-limits .kill-switch")).toHaveText("Kill Switch 待确认");
  expect(f.calls).toHaveLength(3);
  expect(new Set(f.calls.map((call) => call.key)).size).toBe(3);
  expect(f.calls[2].body.autoProfitClose).toMatchObject({ enabled: true, minNetProfitUsd: 0.125 });
  expect(f.calls[2].body.autoProfitClose.minRoiBps ?? null).toBeNull();
  expect(await page.evaluate(() => Object.keys(sessionStorage).filter((key) => key.startsWith("crossline.settings.pending.v1:")))).toEqual([]);
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("NAV history preserves samples through transport and storage failure then refreshes", async ({ page }) => {
  const f = await setup(page);
  let stage = "healthy";
  let reads = 0;
  await page.route(`${API}/api/trading/portfolio/nav-history?*`, async (route) => {
    reads++;
    if (stage === "timeout") return route.fulfill({ status: 504, json: { code: "TIMEOUT", message: "isolated NAV read timeout" } });
    const storage = stage === "storage";
    const problem = { code: "NAV_STORAGE_IO_FAILED", message: "disk write denied", source: "portfolio_nav_store",
      details: { path: "/Users/fixture/Library/Application Support/crossline-omni/portfolio_nav.sqlite", latestSampleSource: "account_equity_missing" } };
    return route.fulfill({ json: { count: 2, rows: [{ occurredAtMs: NOW - 300_000, navUsd: 100 }, { occurredAtMs: NOW, navUsd: stage === "recovered" ? 125 : 110 }],
      source: "portfolio_nav_history", observedAtMs: NOW, latestAtMs: NOW, freshnessMs: 0,
      backendStatus: { backend: "sqlite", enabled: true, durable: true, fallback: false, appendSuccessTotal: 2,
        appendErrorTotal: storage ? 1 : 0, querySuccessTotal: 1, queryErrorTotal: 0, observedAtMs: NOW },
      storageHealth: { venue: "system", operation: "storage:portfolio_nav", status: storage ? "blocked" : "ok", source: "portfolio_nav_store", message: storage ? "disk write denied" : "ok", observedAtMs: NOW, problem: storage ? problem : null },
      problem: storage ? problem : null } });
  });
  await page.goto("/#positions");
  await page.getByRole("tab", { name: "资产", exact: true }).click();
  const history = page.locator(".positions-nav-region");
  await expect(history.getByRole("img", { name: "账户净值历史趋势" })).toBeVisible();
  const refresh = history.getByRole("button", { name: "刷新净值历史", exact: true });
  await expect(refresh).toBeVisible();
  await page.screenshot({ path: test.info().outputPath("nav-before.png"), fullPage: true });
  stage = "timeout";
  await refresh.click();
  await expect(history).toContainText("历史暂时不可刷新");
  await expect(history).toContainText("$110");
  await history.getByText("技术诊断", { exact: true }).click();
  await expect(history).toContainText("isolated NAV read timeout");
  stage = "storage";
  await refresh.click();
  await expect(history).toContainText("历史存储不可用");
  await expect(history.locator(".nav-history-diagnostics")).toHaveAttribute("open", "");
  await expect(history).toContainText("disk write denied");
  await expect(history).not.toContainText("/Users/fixture/");
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    expect(await history.evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
    await page.screenshot({ path: test.info().outputPath(`nav-storage-${width}.png`), fullPage: true });
  }
  stage = "recovered";
  await refresh.click();
  await expect(history).toContainText("$125");
  await expect(history).not.toContainText("历史存储不可用");
  expect(reads).toBe(4);
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});
