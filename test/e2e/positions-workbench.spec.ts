import { expect, test, type Page, type WebSocketRoute } from "@playwright/test";

const API = "http://127.0.0.1:18000";
const NOW = 1_790_210_400_000;

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
    message: status === "succeeded" ? "1 条订单已确认成交" : "已受理，等待交易所终态",
    legs: [{ venue: "bitget", symbol: "SOLUSDT", side: "long", status: legStatus,
      quantity: 0.1, markPrice: 150, notionalUsd: 15 }],
    startedAtMs: NOW - 60_000, updatedAtMs: NOW - (status === "succeeded" ? 30_000 : 1000) };
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
  await expect(facts.locator("strong").nth(0)).toHaveText("缺证据");
  await expect(facts.locator("strong").nth(1)).toHaveText("缺证据");
  await expect(facts.locator("strong").nth(2)).toHaveText("缺证据");
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
  const funding = page.locator(".compact-risk-row").filter({ hasText: "临近 Funding" });
  await expect(funding).toContainText("1 个仓位待补结算证据");
  await expect(funding).toContainText("待确认");
  await expect(funding).not.toContainText("无结算");
  await page.getByRole("tab", { name: "风险", exact: true }).click();
  await expect(page.locator("#positions-detail-risk")).toContainText("暂不汇总 Funding");
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("long positions remain usable while incidents have independent stable drafts", async ({ page }) => {
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

test("compensation HTTP receipt updates controls without waiting for a WS event", async ({ page }) => {
  const f = await setup(page, (snapshot) => { snapshot.recentCloseRuns = [incident("incident-http")]; });
  const requests: any[] = [];
  await page.route(`${API}/api/trading/portfolio/close-runs/incident-http/compensation-orders`, async (route) => {
    requests.push(route.request().postDataJSON());
    const receipt = incident("incident-http", "compensation_submitted");
    receipt.updatedAtMs = NOW + 200;
    receipt.unwindPlan.status = "compensation_submitted";
    receipt.unwindPlan.nextActions = [];
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

test("protection save keeps decimals, locks the draft and returns its receipt to positions", async ({ page }) => {
  const f = await setup(page);
  let saved: any;
  let release: (() => void) | undefined;
  const writes: any[] = [];
  await page.route(`${API}/api/trading/status`, async (route) => {
    const body = await (await route.fetch()).json();
    body.risk.maxOrderNotional = 12.75;
    body.risk.autoProfitClose = { enabled: false, minNetProfitUsd: 5, minRoiBps: 10,
      maxNetLossUsd: 25, maxLossRoiBps: 100, exitBufferBps: 5, confirmationSamples: 3, cooldownSecs: 60 };
    saved ??= body;
    return route.fulfill({ json: saved });
  });
  await page.route(`${API}/api/trading/risk-config`, async (route) => {
    const patch = route.request().postDataJSON();
    writes.push(patch);
    if (writes.length === 1) return route.fulfill({ status: 503, json: { code: "SAVE_UNAVAILABLE", message: "isolated save unavailable" } });
    await new Promise<void>((resolve) => { release = resolve; });
    const changes = Object.fromEntries(Object.entries(patch).filter(([, value]) => value != null));
    Object.assign(saved.risk, changes, { autoProfitClose: { ...saved.risk.autoProfitClose, ...patch.autoProfitClose } });
    return route.fulfill({ json: { ...saved, requestId: "risk-save-fixture" } });
  });
  await page.goto("/#positions");
  await page.getByRole("tab", { name: "控制", exact: true }).click();
  await page.getByRole("button", { name: "调整保护", exact: true }).click();
  await expect(page).toHaveURL(/#settings/);
  const profit = page.locator("label").filter({ hasText: "最低净利润 USD" }).locator("input");
  const orderCap = page.locator("label").filter({ hasText: "单笔名义上限 USD" }).locator("input");
  await expect(profit).toHaveValue("5");
  await page.screenshot({ path: test.info().outputPath("protection-settings-1440.png"), fullPage: true });
  await expect(orderCap).toHaveValue("12.75");
  await profit.fill("0.125");
  await page.locator("label").filter({ hasText: "自动止盈并平双边" }).locator("input").check();
  await page.getByRole("button", { name: "保存风控", exact: true }).click();
  await expect(page.locator(".settings-message").filter({ hasText: "isolated save unavailable" })).toBeVisible();
  await expect(profit).toBeEnabled();
  await expect(profit).toHaveValue("0.125");
  await expect.poll(() => writes.length).toBe(1);
  await page.getByRole("button", { name: "保存风控", exact: true }).click();
  await expect.poll(() => writes.length).toBe(2);
  await expect(profit).toBeDisabled();
  release!();
  await expect(page.getByRole("button", { name: "保存风控", exact: true })).toBeEnabled();
  await expect(profit).toHaveValue("0.125");
  await page.setViewportSize({ width: 390, height: 900 });
  expect(await page.locator(".settings-risk-editor").evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
  await page.screenshot({ path: test.info().outputPath("protection-settings-390.png"), fullPage: true });
  await page.setViewportSize({ width: 1440, height: 900 });
  expect(writes[0].autoProfitClose).toMatchObject({ enabled: true, minNetProfitUsd: 0.125, minRoiBps: 10 });
  const positionsNav = page.getByRole("button", { name: "切换到持仓/风控", exact: true });
  await expect(positionsNav).toBeVisible();
  await positionsNav.click();
  await page.getByRole("tab", { name: "控制", exact: true }).click();
  await expect(page.locator(".pair-protection-bar")).toContainText("$0.125");
  await expect(page.locator(".pair-protection-bar")).toContainText("等待配对");
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
