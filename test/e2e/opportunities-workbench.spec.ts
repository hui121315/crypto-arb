import { expect, test } from "@playwright/test";
import { setup, API, NOW, strategies } from "./fixtures/opportunity-workbench";

async function scanner(page: Parameters<typeof setup>[0], paginated = false) {
  const f = await setup(page, paginated);
  let detailFailure = false;
  let wrongDetail = false;
  let heldDetail = false;
  let releaseDetail: (() => void) | undefined;
  let releaseTest: (() => void) | undefined;
  let failTest = false;
  let testCount = 0;
  let webhookFailure = false;
  let detailTransform: ((body: any) => void) | undefined;
  const requests: string[] = [];
  const webhook = { config: { enabled: true, provider: "generic", url: "[configured]", urlConfigured: true,
    secretConfigured: true, eventKinds: ["opportunity"], timeoutMs: 5000, maxAttempts: 3, baseBackoffMs: 500, queueCapacity: 128 },
    queueDepth: 0, deliveredTotal: 0, failedTotal: 0, droppedTotal: 0, recentDeliveries: [], updatedAtMs: NOW };
  await page.route(`${API}/api/v3/arbitrage/opportunities/*/detail?**`, async (route) => {
    requests.push(route.request().url());
    if (heldDetail) await new Promise<void>((resolve) => { releaseDetail = resolve; });
    if (detailFailure) return route.fulfill({ status: 503, json: { code: "DETAIL_UNAVAILABLE", message: "fixture: detail unavailable" } });
    const body = await (await route.fetch()).json();
    const id = new URL(route.request().url()).pathname.split("/").at(-2);
    const row = f.rows.find((row) => row.id === id) ?? f.rows[0];
    Object.assign(body.opportunity, { id, symbol: row.symbol, longExchange: row.longLeg.venue, shortExchange: row.shortLeg.venue });
    detailTransform?.(body);
    if (wrongDetail) body.opportunity.id = "fixture-another-opportunity";
    return route.fulfill({ json: body });
  });
  await page.route(`${API}/api/webhook/status`, (route) => webhookFailure
    ? route.fulfill({ status: 503, json: { code: "WEBHOOK_STATUS_UNAVAILABLE", message: "fixture: status unavailable" } })
    : route.fulfill({ json: webhook }));
  await page.route(`${API}/api/webhook/test`, async (route) => {
    testCount++;
    await new Promise<void>((resolve) => { releaseTest = resolve; });
    return failTest ? route.fulfill({ status: 400, json: { code: "TEST_REJECTED", message: "fixture: enqueue unavailable" } })
      : route.fulfill({ json: { eventId: "evt-webhook-test-fixture-scanner-test", queued: true,
          actionRunId: "fixture-scanner-test", requestId: route.request().headers()["x-request-id"],
          idempotencyKey: route.request().headers()["idempotency-key"] } });
  });
  return { ...f, requests, webhook,
    detailFailure: (value: boolean) => { detailFailure = value; },
    wrongDetail: (value: boolean) => { wrongDetail = value; },
    transformDetail: (transform?: (body: any) => void) => { detailTransform = transform; },
    holdDetail: () => { heldDetail = true; },
    releaseDetail: () => { heldDetail = false; releaseDetail?.(); },
    testCount: () => testCount,
    finishTest: (failed = false) => { failTest = failed; releaseTest?.(); },
    webhookFailure: (value: boolean) => { webhookFailure = value; },
  };
}

test("detail evidence ages independently and retains only the failed segment with its original source", async ({ page }) => {
  await page.clock.install({ time: NOW });
  const f = await scanner(page);
  let phase: "initial" | "partial" | "deferred" | "recovered" = "initial";
  const originalRequest = `original-snapshot-${"long-request-id-".repeat(12)}`;
  f.transformDetail((body) => {
    body.requestId = phase === "initial" ? originalRequest : `request-${phase}`;
    body.observedAtMs = NOW;
    body.error = null;
    body.partialFailures = [];
    for (const [index, book] of [body.longOrderbook, body.shortOrderbook].entries()) {
      book.data.exchange = "binance";
      book.data.symbol = "BTC";
      book.data.bids = [[60001 + index * 100, 1]];
      book.data.asks = [[60002 + index * 100, 1]];
      Object.assign(book.health, { quality: "fresh", source: "ws_push", freshnessMs: 20,
        observedAtMs: NOW, problem: null, lastError: null, retryAfterMs: null });
    }
    Object.assign(body.history, { source: "memory", observedAtMs: NOW, freshnessMs: 100,
      problem: null, problems: [], storageHealth: null, retryAfterMs: null });
    for (const record of body.history.rows) Object.assign(record, {
      symbol: body.opportunity.symbol, occurredAtMs: NOW,
      longExchange: "binance", shortExchange: "binance",
    });
    if (phase === "partial") {
      body.longOrderbook.data = null;
      Object.assign(body.longOrderbook.health, { quality: "rate_limited", source: "rest_fallback",
        freshnessMs: null, retryAfterMs: 2000,
        problem: { code: "BOOK_LIMITED", message: "fixture: long leg unavailable", requestId: "failed-book-request" } });
      body.shortOrderbook.data.bids[0][0] = 60201;
      body.shortOrderbook.data.asks[0][0] = 60202;
      body.history.rows = [];
      body.history.count = 0;
    } else if (phase === "deferred") {
      body.longOrderbook.data = null;
      Object.assign(body.longOrderbook.health, { quality: "unverified", source: "local_cache",
        freshnessMs: null, coverage: { requested: 0, received: 0, coveragePct: 0 } });
      body.shortOrderbook.data = null;
      Object.assign(body.shortOrderbook.health, { quality: "unsupported", freshnessMs: null });
      body.history.rows = [];
      body.history.count = 0;
    } else if (phase === "recovered") {
      body.longOrderbook.data.bids[0][0] = 60301;
      body.longOrderbook.data.asks[0][0] = 60302;
      body.shortOrderbook.health.freshnessMs = null;
    }
    if (!body.history.rows.length) {
      body.history.page.returnedCount = 0;
      Object.assign(body.history.rowCap, { returnedCount: 0, totalRows: 0, truncated: false, truncatedCount: 0 });
    }
  });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/#opportunities");
  await page.getByRole("tab", { name: "现货-永续", exact: true }).click();
  const detail = page.locator("#opportunity-detail-panel");
  await expect(detail.locator(".detail-head")).toContainText("BTC");
  const section = (label: string) => detail.locator(".opportunity-detail-section")
    .filter({ has: page.locator(":scope > summary > span", { hasText: label }) });
  const books = section("订单簿");
  const history = section("历史");
  const evidence = section("数据数据依据");
  for (const target of [books, history, evidence]) await target.locator(":scope > summary").click();
  const long = books.locator(".detail-book-row").nth(0);
  const short = books.locator(".detail-book-row").nth(1);
  await expect(long).toContainText("60001.0000");
  await expect(short).toContainText("60101.0000");
  await expect(long).toContainText("WS · 数据年龄");
  const initialHistory = await history.locator(".detail-row").count();
  expect(initialHistory).toBeGreaterThan(0);
  await expect(history.locator(".detail-row strong").first()).toHaveText(/2026-\d\d-\d\d \d\d:\d\d/);
  const readsBeforeTime = f.requests.length;
  await page.clock.fastForward(12_000);
  f.tick();
  await expect(long.locator(".detail-evidence-age")).toContainText(/1[2-4]\.\ds/);
  expect(f.requests).toHaveLength(readsBeforeTime);
  await expect(books).toHaveAttribute("open", "");

  f.detailFailure(true);
  await detail.getByRole("button", { name: "刷新数据依据", exact: true }).click();
  await expect(detail).toContainText("DETAIL_UNAVAILABLE");
  await expect(history.locator(".detail-row")).toHaveCount(initialHistory);
  await expect(history).toContainText("上次数据 · 历史 memory");
  f.detailFailure(false);

  phase = "partial";
  await detail.getByRole("button", { name: "刷新数据依据", exact: true }).click();
  await expect(long).toContainText("保留旧值");
  await expect(long).toContainText("60001.0000");
  await expect(long).toContainText("上次数据 · WS");
  await expect(long.locator(".detail-evidence-age")).toContainText(/1[2-4]\.\ds/);
  await expect(short).toContainText("60201.0000");
  await expect(short).not.toContainText("保留旧值");
  await expect(history.locator(".detail-row")).toHaveCount(0);
  await expect(history).toContainText("暂无历史记录");
  await long.locator(".detail-evidence-context summary").click();
  await expect(long).toContainText(originalRequest);
  await expect(long).toContainText("failed-book-request");
  await expect(long).toContainText("BOOK_LIMITED");
  await expect(long).toContainText("本次读取来源 REST 兜底");
  await expect(long).toContainText("2000ms");
  await long.locator(".detail-evidence-context summary").focus();
  f.rows.forEach((row) => { row.cost.oneCycleNetBps = 25; row.metrics.oneCycleNetBps = 25; });
  f.tick();
  await expect(detail.locator(".detail-metrics")).toContainText("+0.250%");
  await expect(long.locator(".detail-evidence-context")).toHaveAttribute("open", "");
  await expect(long.locator(".detail-evidence-context summary")).toBeFocused();
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await long.scrollIntoViewIfNeeded();
    expect(await detail.evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    await page.screenshot({ path: test.info().outputPath(`scanner-evidence-${width}.png`) });
  }
  f.detailFailure(true);
  await detail.getByRole("button", { name: "刷新数据依据", exact: true }).click();
  await expect(detail).toContainText("DETAIL_UNAVAILABLE");
  await expect(long).toContainText("60001.0000");
  await expect(short).toContainText("60201.0000");
  await page.clock.fastForward(5000);
  await expect(long.locator(".detail-evidence-age")).toContainText(/1[7-9]\.\ds/);
  f.detailFailure(false);
  phase = "deferred";
  await detail.getByRole("button", { name: "刷新数据依据", exact: true }).click();
  await expect(long).toContainText("构建时核对 · 未读取");
  await expect(long).not.toContainText("60001.0000");
  await expect(short).toContainText("读取时不支持");
  await expect(short).not.toContainText("60201.0000");
  await expect(books).not.toContainText("保留旧值");
  phase = "recovered";
  await detail.getByRole("button", { name: "刷新数据依据", exact: true }).click();
  await expect(long).toContainText("60301.0000");
  await expect(short).toContainText("数据年龄 未知");
  await expect(books).not.toContainText("保留旧值");
  await expect(history.locator(".detail-row")).toHaveCount(initialHistory);
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("scanner source and selection stay bound through stale quotes, late evidence and missing opportunities", async ({ page }) => {
  const f = await scanner(page);
  const live = f.rows[0];
  const outside = structuredClone(live);
  outside.id += "-kraken";
  outside.longLeg.venue = "kraken";
  outside.longLeg.action = "kraken 做多永续";
  outside.longLeg.price = 65000;
  Object.assign(outside.longLeg.marketEvidence, { venue: "kraken", price: 65000 });
  f.rows.push(outside);
  f.paginateList();
  f.transformSearch((response) => {
    response.cachedAt = new Date(NOW + 200).toISOString();
    response.observedAtMs = NOW + 200;
    response.rows[0].longLeg.price = 61000;
    response.rows[0].longLeg.marketEvidence.price = 61000;
  });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/#opportunities");
  await page.getByRole("tab", { name: "永续跨所", exact: true }).click();
  const search = page.getByPlaceholder("币种 / 交易所 / 路由", { exact: true });
  await search.fill("BTC");
  const rows = page.locator(".opportunity-table tbody tr[id]");
  const first = rows.filter({ hasText: "binance→bitget" });
  const other = rows.filter({ hasText: "kraken→bitget" });
  const build = (row: typeof first) => row.getByRole("button", { name: "构建对冲", exact: true });
  const detail = page.locator("#opportunity-detail-panel");
  await expect(rows).toHaveCount(2);
  await expect(first).toContainText("61000");
  await expect(build(first)).toBeEnabled();
  f.tick(1000);
  await expect(first).toContainText("60000.5");
  await first.click();
  await expect(detail.locator(".detail-head")).toContainText("BTC");
  f.holdDetail();
  const delayedDetail = page.waitForResponse("**/detail?**");
  const priorDetails = f.requests.length;
  await detail.getByRole("button", { name: "刷新数据依据", exact: true }).click();
  await expect.poll(() => f.requests.length).toBe(priorDetails + 1);
  live.cost.oneCycleNetBps = 12.5;
  live.metrics.oneCycleNetBps = 12.5;
  f.tick();
  await expect(first).toContainText("0.125%");
  f.stale(true);
  await expect(build(first)).toBeDisabled();
  await expect(first).toContainText("上次测算");
  await expect(build(other)).toBeEnabled();
  await expect(detail.locator(".opportunity-detail-snapshot-status")).toBeVisible();
  await expect(page.locator(".opportunity-readiness")).toContainText("等待新快照");
  f.releaseDetail();
  await (await delayedDetail).finished();
  await expect(detail.locator(".detail-metrics")).toContainText("0.125%");
  await expect(build(first)).toBeDisabled();
  const eligibility = page.getByRole("group", { name: "按可执行性筛选", exact: true });
  await expect(eligibility.getByRole("button", { name: /可预检/ })).toHaveText("可检查交易1");
  await expect(page.locator(".scan-kpi").nth(1).locator("strong")).toHaveText("1");
  await expect(page.locator(".scan-kpi").first().locator("strong")).toHaveText("2");
  await other.click();
  await expect(detail).toContainText(/kraken/i);
  await expect(detail.locator(".opportunity-detail-snapshot-status")).toHaveCount(0);
  await expect(page.locator(".opportunity-readiness")).toContainText("可进入交易检查");
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await build(other).scrollIntoViewIfNeeded();
    await build(other).click({ trial: true });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    expect(await page.locator(".scan-kpi > span").evaluateAll((labels) =>
      labels.every((label) => label.getBoundingClientRect().height < 24 && label.scrollWidth <= label.clientWidth + 1))).toBe(true);
    await page.screenshot({ path: test.info().outputPath(`scanner-mixed-source-${width}.png`), fullPage: true });
  }
  await page.setViewportSize({ width: 1440, height: 900 });
  f.stale(false);
  f.transformSearch((response) => {
    response.page = { ...response.page, totalRows: 3, returnedCount: 2,
      hasNextPage: true, nextCursor: "page-2", lastCursor: "page-2" };
  });
  await search.clear();
  await search.fill("BTC");
  await expect(page.getByRole("button", { name: "下一页", exact: true })).toBeEnabled();
  f.transformSearch((response) => {
    response.rows = [response.rows[0]];
    response.rows[0].id = "fixture-search-page-2";
    response.rows[0].longLeg.price = 62000;
    response.page = { ...response.page, startOffset: 2, totalRows: 3, returnedCount: 1,
      hasNextPage: false, nextCursor: null, previousCursor: null, lastCursor: null };
  });
  await page.getByRole("button", { name: "下一页", exact: true }).click();
  await expect(rows).toHaveCount(1);
  f.tick();
  await expect(rows).toContainText("62000");
  await expect(page.locator(".futures-search-status").last()).toContainText("WS 报价 0 条 · 搜索快照 1 条");

  f.paginateList(false);
  f.transformSearch();
  f.tick();
  await search.clear();
  await search.fill("BTC");
  await expect(rows).toHaveCount(2);
  await first.click();
  await expect(detail).toContainText(/binance/i);
  const context = page.locator(".opportunity-route-context");
  f.rows.splice(f.rows.indexOf(live), 1);
  f.tick();
  await expect(context).toContainText("当前列表未找到原机会");
  await expect(page.locator('.opportunity-table tr[aria-selected="true"]')).toHaveCount(0);
  await expect(detail.locator(".detail-head")).toHaveCount(0);
  f.rows.push(live);
  f.tick();
  await expect(first).toHaveAttribute("aria-selected", "true");
  await expect(detail).toContainText(/binance/i);
  f.holdDetail();
  const removedDetail = page.waitForResponse("**/detail?**");
  await detail.getByRole("button", { name: "刷新数据依据", exact: true }).click();
  await expect(detail.getByRole("button", { name: "刷新数据依据", exact: true })).toBeDisabled();
  f.holdSearch("BTCUSDT");
  const lateSearch = page.waitForResponse((response) => new URL(response.url()).searchParams.get("symbol") === "BTCUSDT");
  await page.goto(`/#opportunities?symbol=BTCUSDT&strategy=perp_cross&opp=${live.id}&page=0`);
  await expect.poll(() => f.searches.includes("BTCUSDT")).toBe(true);
  f.rows.splice(f.rows.indexOf(live), 1);
  f.tick();
  f.releaseSearch();
  await (await lateSearch).finished();
  f.releaseDetail();
  await (await removedDetail).finished();
  await expect(rows).toHaveCount(1);
  await expect(other).toBeVisible();
  await expect(page.locator('.opportunity-table tr[aria-selected="true"]')).toHaveCount(0);
  await expect(detail.locator(".detail-head")).toHaveCount(0);
  await expect(context).toContainText("当前列表未找到原机会");
  await expect(context).toContainText(live.id);
  await page.goto("/#opportunities?symbol=BTC&strategy=perp_cross&opp=fixture-missing-alert&page=0");
  await expect(context).toContainText("fixture-missing-alert");
  await expect(page.locator('.opportunity-table tr[aria-selected="true"]')).toHaveCount(0);
  await context.getByRole("button", { name: "取消定位", exact: true }).click();
  await expect(context).toHaveCount(0);
  await expect(other).toHaveAttribute("aria-selected", "true");
  await build(other).click();
  await expect(page.locator(".execution-ticket")).toContainText(/kraken/i);
  await expect(page.locator(".execution-ticket h3")).toHaveText("BTC · 永续跨所");
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("scanner detail timeout preserves evidence, retries independently and restores the same page", async ({ page }) => {
  await page.clock.install({ time: NOW });
  const f = await scanner(page, true);
  f.paginateList();
  await page.goto("/#opportunities");
  await page.getByRole("tab", { name: "永续跨所", exact: true }).click();
  await page.getByRole("button", { name: "下一页", exact: true }).click();
  const rows = page.locator(".opportunity-table tbody tr[id]");
  const build = rows.first().getByRole("button", { name: "构建对冲", exact: true });
  const detail = page.locator("#opportunity-detail-panel");
  await expect(rows).toContainText("62000");
  await expect(build).toBeEnabled();
  await expect(detail.locator(".detail-head")).toContainText("BTC");
  const evidence = detail.locator(".opportunity-detail-section").filter({ has: page.locator("summary", { hasText: "数据数据依据" }) });
  await evidence.locator(":scope > summary").click();
  const oldBody = await (await page.request.get(f.requests.at(-1)!)).json();
  let hold = true;
  let release!: () => void;
  let attempts = 0;
  let cancelled = 0;
  let lateReplyReleased = false;
  page.on("requestfailed", request => { if (request.url().includes("/detail?")) cancelled++; });
  await page.route(`${API}/api/v3/arbitrage/opportunities/*/detail?**`, async route => {
    attempts++;
    if (!hold) return route.fallback();
    await new Promise<void>(resolve => { release = resolve; });
    await route.fulfill({ json: oldBody });
    lateReplyReleased = true;
  });
  await detail.getByRole("button", { name: "刷新数据依据", exact: true }).click();
  await expect.poll(() => attempts).toBe(1);
  await expect(detail.getByRole("button", { name: "刷新数据依据", exact: true })).toBeDisabled();
  const beforeIdle = f.listRequests.length;
  await page.clock.fastForward(15_100);
  await expect(detail).toContainText("SHARED_READ_TIMEOUT");
  await expect.poll(() => cancelled).toBe(1);
  await expect(detail.getByRole("button", { name: "刷新数据依据", exact: true })).toBeEnabled();
  await expect(evidence).toHaveAttribute("open", "");
  expect(attempts).toBe(1);
  await page.clock.fastForward(15_900);
  f.tick();
  await expect(build).toBeDisabled();
  await expect(page.locator(".opportunity-readiness")).toContainText("等待新快照");
  await expect(detail.locator(".opportunity-detail-snapshot-status")).toContainText("候选报价待更新");
  await expect(detail.locator(".detail-metrics")).toContainText("上次测算边际");
  expect(f.listRequests).toHaveLength(beforeIdle);
  hold = false;
  await detail.getByRole("button", { name: "刷新数据依据", exact: true }).click();
  await expect.poll(() => attempts).toBe(2);
  await expect(detail).not.toContainText("SHARED_READ_TIMEOUT");
  await expect(evidence).toHaveAttribute("open", "");
  release();
  await expect.poll(() => lateReplyReleased).toBe(true);
  await expect(detail).not.toContainText("OPPORTUNITY_DETAIL_ID_MISMATCH");
  await expect(detail.getByRole("button", { name: "刷新数据依据", exact: true })).toBeEnabled();
  await expect(build).toBeDisabled();
  await expect(detail.locator(".opportunity-detail-snapshot-status")).toBeVisible();
  await page.setViewportSize({ width: 390, height: 844 });
  const kpis = page.locator(".scan-kpis");
  await expect(kpis.locator("strong")).toHaveText(["—", "—", "—"]);
  await expect(kpis.locator('[data-tone="ready"]')).toHaveCount(0);
  expect(await kpis.evaluate((el) => el.clientHeight)).toBeLessThan(100);
  await kpis.screenshot({ path: test.info().outputPath("scanner-expired-kpis-390.png") });
  await detail.scrollIntoViewIfNeeded();
  expect(await detail.evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
  await page.screenshot({ path: test.info().outputPath("scanner-expired-detail-390.png") });
  await page.getByRole("button", { name: "刷新当前页", exact: true }).click();
  await expect(build).toBeEnabled();
  await expect(detail.locator(".opportunity-detail-snapshot-status")).toHaveCount(0);
  expect(new URLSearchParams(f.listRequests.at(-1)).get("cursor")).toBe("page-2");
  await expect(rows).toContainText("62000");
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.getByPlaceholder("币种 / 交易所 / 路由", { exact: true }).fill("BTC");
  await expect(page.locator(".futures-search-status").last()).toContainText("BTC · 搜索快照");
  await page.getByRole("button", { name: "下一页", exact: true }).click();
  await expect(rows).toContainText("62000");
  await expect(build).toBeEnabled();
  const beforeSearchIdle = f.listRequests.length;
  await page.clock.fastForward(31_000);
  f.tick();
  await expect(build).toBeDisabled();
  await expect(detail.locator(".opportunity-detail-snapshot-status")).toBeVisible();
  expect(f.listRequests).toHaveLength(beforeSearchIdle);
  await page.getByRole("button", { name: "重新搜索", exact: true }).click();
  await expect(build).toBeEnabled();
  expect(new URLSearchParams(f.listRequests.at(-1)).get("cursor")).toBe("page-2");
  await expect(rows).toContainText("62000");
  await expect(detail.locator(".opportunity-detail-snapshot-status")).toHaveCount(0);
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("candidate evidence and keyboard focus survive a live quote update", async ({ page }) => {
  const f = await scanner(page);
  await page.goto("/#opportunities");
  const table = page.getByRole("table", { name: "机会扫描候选", exact: true });
  await expect(table.locator("tbody tr[id]")).toHaveCount(10);
  const all = page.getByRole("tab", { name: "全部", exact: true });
  await all.focus();
  await all.press("ArrowRight");
  await expect(page.getByRole("tab", { name: "永续跨所", exact: true })).toBeFocused();
  await page.getByRole("tab", { name: "永续跨所", exact: true }).press("Home");
  await expect(all).toBeFocused();
  await table.locator("tbody tr[id]").first().click();
  const detail = page.locator("#opportunity-detail-panel");
  await expect(detail.locator(".detail-head")).toBeVisible();
  const funding = detail.locator("details").filter({ has: page.locator("summary", { hasText: "资金费 周期" }) });
  await funding.locator("summary").click();
  const flow = page.locator(".opportunity-flow-details");
  await flow.locator("summary").click();
  const current = table.getByRole("button", { name: "当前数据依据", exact: true });
  await current.focus();
  for (const width of [1440, 1024, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await page.screenshot({ path: test.info().outputPath(`opportunities-${width}.png`), fullPage: true });
    const box = await table.locator("tbody tr[id]").first().locator("td").nth(4).boundingBox();
    const action = await table.locator("tbody tr[id]").first().locator("td").last().boundingBox();
    if (width > 800) expect(box!.x + box!.width).toBeLessThanOrEqual(action!.x + 1);
    else expect(box!.y + box!.height).toBeLessThanOrEqual(action!.y + 1);
    await expect(table.locator(".route-cell").first()).toBeVisible();
    if (width >= 1100) {
      const pager = await page.locator(".opportunity-layout .table-pager-bar").boundingBox();
      expect(pager!.y + pager!.height).toBeLessThanOrEqual(900);
      await expect(detail.locator(".funding-cycle-trend")).toBeVisible();
      expect(await detail.evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
    }
  }
  await page.setViewportSize({ width: 1440, height: 900 });
  await current.focus();
  f.rows[0].cost.oneCycleNetBps = 12.5;
  f.rows[0].metrics.oneCycleNetBps = 12.5;
  f.tick();
  await expect(table.locator("tbody tr[id]").first()).toContainText("60000.5");
  await expect(funding).toHaveAttribute("open", "");
  await expect(flow).toHaveAttribute("open", "");
  await expect(current).toBeFocused();
  const first = table.locator("tbody tr[id]").first();
  await first.focus();
  await first.press("ArrowDown");
  await expect(table.locator("tbody tr[id]").nth(1)).toBeFocused();
  await expect(detail.locator(".detail-head")).toContainText("ETH");
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("scanner mobile strategies retain both prices, costs and the exact execution handoff", async ({ page }) => {
  const f = await scanner(page);
  await page.goto("/#opportunities");
  const rows = page.locator(".opportunity-table tbody tr[id]");
  const first = rows.first();
  for (const width of [720, 390, 320]) {
    await page.setViewportSize({ width, height: 900 });
    for (const [kind, label] of strategies) {
      await page.getByRole("tab", { name: label, exact: true }).click();
      await expect(rows).toHaveCount(2);
      await expect(first.locator(".route-cell")).toContainText("60000.00");
      await expect(first.locator(".route-cell")).toContainText("60001.00");
      await expect(first.locator(".route-cell")).toContainText(kind.includes("spot") ? "买入现货" : "做多永续");
      await expect(first.locator(".route-cell")).toContainText(kind === "spot_cross" ? "卖出现货" : "做空永续");
      await expect(first.locator('td[data-label="完整成本"]')).toBeVisible();
      expect(await page.getByRole("tablist", { name: "策略范围" }).getByRole("tab").evaluateAll(tabs => tabs.every(el => {
        const r = el.getBoundingClientRect();
        return r.width > 0 && r.left >= 0 && r.right <= innerWidth && el.scrollWidth <= el.clientWidth + 1;
      }))).toBe(true);
      expect(await page.locator(".paged-table-wrap").evaluate(el => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
      expect(await first.locator("td").evaluateAll(cells => {
        const boxes = cells.map(el => el.getBoundingClientRect());
        return cells.every((el, i) => {
          const a = boxes[i];
          return a.width > 0 && a.left >= 0 && a.right <= innerWidth && el.scrollWidth <= el.clientWidth + 1
            && boxes.every((b, j) => i === j || a.right <= b.left + 1 || a.left >= b.right - 1 || a.bottom <= b.top + 1 || a.top >= b.bottom - 1);
        });
      })).toBe(true);
      const build = first.getByRole("button", { name: "构建对冲", exact: true });
      await build.click({ trial: true });
      if (kind === "perp_cross" || (kind === "spot_cross" && width === 320)) {
        await page.screenshot({ path: test.info().outputPath(`scanner-${kind}-${width}.png`), fullPage: true });
      }
    }
  }
  const evidence = first.getByRole("button", { name: "当前数据依据", exact: true });
  await evidence.focus();
  f.stale(true);
  const build = first.getByRole("button", { name: "构建对冲", exact: true });
  await expect(build).toBeDisabled();
  await expect(first.locator(".opportunity-net-decision")).toContainText("上次测算");
  await expect(evidence).toBeFocused();
  f.stale(false);
  f.tick();
  await expect(first.locator(".route-cell")).toContainText("60000.5");
  await expect(evidence).toBeFocused();
  await build.click();
  await expect(page.locator(".execution-ticket h3")).toHaveText("BTC · 现货跨所");
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("opportunity deep link retains exact identity through loading and a missing target", async ({ page }) => {
  const f = await scanner(page);
  const target = structuredClone(f.rows[0]);
  target.id = "fixture-perp_cross-BTC-gate";
  target.shortLeg.venue = "gate";
  target.shortLeg.action = "gate 做空永续";
  target.shortLeg.marketEvidence.venue = "gate";
  f.rows.push(target);
  await page.goto("/#opportunities");
  await page.getByLabel("最低净利百分比", { exact: true }).fill("99");
  f.holdSearch("BTC");
  await page.goto(`/#opportunities?symbol=BTC&strategy=perp_cross&opp=${target.id}&page=0`);
  await expect.poll(() => f.searches.includes("BTC")).toBe(true);
  f.releaseSearch();
  const table = page.getByRole("table", { name: "机会扫描候选", exact: true });
  const selected = table.locator('tbody tr[aria-selected="true"]');
  await expect(selected).toHaveCount(1);
  await expect(selected).toContainText("gate");
  await expect(page.getByLabel("最低净利百分比", { exact: true })).toHaveValue("0");
  await expect(page.locator("#opportunity-detail-panel")).toContainText("gate");
  await page.goto("/#opportunities?symbol=BTC&strategy=perp_cross&opp=fixture-no-longer-visible&page=0");
  const scope = page.locator(".opportunity-route-context");
  await expect(scope).toContainText("当前列表未找到原机会");
  await expect(selected).toHaveCount(0);
  await expect(page.locator("#opportunity-detail-panel .detail-head")).toHaveCount(0);
  const pager = await page.locator(".opportunity-layout .table-pager-bar").boundingBox();
  expect(pager!.y + pager!.height).toBeLessThanOrEqual(900);
  await page.screenshot({ path: test.info().outputPath("opportunity-link-desktop.png"), fullPage: true });
  const first = table.locator("tbody tr[id]").first();
  await expect(first).toHaveAttribute("tabindex", "0");
  await first.focus();
  await first.press("Enter");
  await expect(selected).toContainText("bitget");
  await expect(scope).toHaveCount(0);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/#opportunities?symbol=BTC&strategy=perp_cross&opp=fixture-no-longer-visible&page=0");
  await expect(scope).toContainText("当前列表未找到原机会");
  await scope.getByRole("button", { name: "取消定位", exact: true }).click({ trial: true });
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(390);
  await page.screenshot({ path: test.info().outputPath("opportunity-link-mobile.png"), fullPage: true });
  await scope.getByRole("button", { name: "取消定位", exact: true }).click();
  await expect(scope).toHaveCount(0);
  await expect(selected).toHaveCount(1);
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});

test("symbol changes, failures and later pages cannot reuse another search scope", async ({ page }) => {
  const f = await scanner(page, true);
  await page.goto("/#opportunities");
  const rows = page.locator(".opportunity-table tbody tr[id]");
  const search = page.getByPlaceholder("币种 / 交易所 / 路由", { exact: true });
  await expect(rows).toHaveCount(10);
  f.holdSearch("BTCUSDT");
  const old = page.waitForResponse((r) => new URL(r.url()).searchParams.get("symbol") === "BTCUSDT");
  await search.fill("BTCUSDT");
  await expect.poll(() => f.searches.includes("BTCUSDT")).toBe(true);
  await search.fill("ETH");
  await expect(rows).toHaveCount(5);
  f.releaseSearch();
  await (await old).finished();
  await expect(rows.first()).toContainText("ETH");
  f.failSearch("SOL");
  await search.fill("SOL");
  await expect(page.locator(".futures-search-status")).toContainText("SOL · 搜索失败");
  await expect(rows).toHaveCount(0);
  f.failSearch();
  await page.getByRole("button", { name: "重新搜索", exact: true }).click();
  await expect(page.locator(".futures-search-status")).toContainText("SOL · 搜索快照");
  await page.getByRole("tab", { name: "永续跨所", exact: true }).click();
  await search.fill("BTC");
  await expect(rows).toHaveCount(1);
  await page.getByRole("button", { name: "下一页", exact: true }).click();
  await expect(rows).toHaveCount(1);
  await expect(rows).toContainText("62000");
  f.tick();
  await expect(rows).toHaveCount(1);
  await expect(rows).toContainText("62000");
  await search.fill("kraken");
  await expect(rows).toHaveCount(0);
  expect(f.searches).not.toContain("KRAKEN");
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("stale snapshots block building and recovery uses the currently selected row", async ({ page }) => {
  const f = await scanner(page);
  await page.goto("/#opportunities");
  const rows = page.locator(".opportunity-table tbody tr[id]");
  await expect(rows).toHaveCount(10);
  await rows.first().click();
  const build = rows.first().getByRole("button", { name: "构建对冲", exact: true });
  await expect(build).toBeEnabled();
  f.stale(true);
  await expect(build).toBeDisabled();
  await expect(page.locator(".opportunity-readiness")).toContainText("等待新快照");
  f.stale(false);
  f.partial(true);
  await expect(build).toBeEnabled();
  await build.click();
  await expect(page.locator(".execution-ticket")).toContainText("BTC · 永续跨所");
  await expect(page.locator(".execution-ticket")).toContainText("机会扫描");
  await page.goto("/#opportunities");
  await page.getByPlaceholder("币种 / 交易所 / 路由", { exact: true }).fill("NORESULT");
  await expect(page.locator(".opportunity-table tbody tr[id]")).toHaveCount(0);
  await page.goto("/#execution");
  await expect(page.locator(".execution-ticket")).toContainText("BTC · 永续跨所");
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("detail retry preserves expanded evidence and cannot revert a live profitability update", async ({ page }) => {
  const f = await scanner(page);
  f.detailFailure(true);
  await page.goto("/#opportunities");
  const detail = page.locator("#opportunity-detail-panel");
  await expect(detail).toContainText("DETAIL_UNAVAILABLE");
  const evidence = detail.locator("details").filter({ has: page.locator("summary", { hasText: "数据数据依据" }) });
  await evidence.locator(":scope > summary").click();
  f.detailFailure(false);
  f.holdDetail();
  const n = f.requests.length;
  await detail.getByRole("button", { name: "刷新数据依据", exact: true }).click();
  await expect.poll(() => f.requests.length).toBe(n + 1);
  await expect(detail.getByRole("button", { name: "刷新数据依据", exact: true })).toBeDisabled();
  f.rows.forEach((row) => { row.cost.oneCycleNetBps = 25; row.metrics.oneCycleNetBps = 25; });
  f.tick();
  f.releaseDetail();
  await expect(detail.getByRole("button", { name: "刷新数据依据", exact: true })).toBeEnabled();
  await expect(detail).not.toContainText("DETAIL_UNAVAILABLE");
  await expect(evidence).toHaveAttribute("open", "");
  await expect(detail.locator(".detail-metrics")).toContainText("+0.250%");
  f.wrongDetail(true);
  await detail.getByRole("button", { name: "刷新数据依据", exact: true }).click();
  await expect(detail).toContainText("OPPORTUNITY_DETAIL_ID_MISMATCH");
  await expect(detail.locator(".detail-metrics")).toContainText("+0.250%");
  await expect(evidence).toHaveAttribute("open", "");
  f.wrongDetail(false);
  await detail.getByRole("button", { name: "刷新数据依据", exact: true }).click();
  await expect(detail).not.toContainText("OPPORTUNITY_DETAIL_ID_MISMATCH");
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("webhook test prevents duplicate requests and distinguishes enqueue from delivery", async ({ page }) => {
  const f = await scanner(page);
  await page.goto("/#opportunities");
  const monitor = page.locator(".webhook-monitor-disclosure");
  await expect(monitor).toContainText("投递通道已就绪");
  await monitor.locator(":scope > summary").click();
  const button = monitor.getByRole("button", { name: "测试投递", exact: true });
  await button.click();
  await expect(monitor.getByRole("button", { name: "提交中", exact: true })).toBeDisabled();
  await expect.poll(f.testCount).toBe(1);
  f.finishTest(true);
  await expect(monitor).toContainText("TEST_REJECTED");
  await button.click();
  await expect.poll(f.testCount).toBe(2);
  f.finishTest();
  await expect(monitor).toContainText("测试消息已排队");
  await expect(monitor).not.toContainText("TEST_REJECTED");
  const history = monitor.locator(".webhook-monitor-history").filter({ hasText: "最近投递" });
  await history.locator("summary").click();
  f.webhookFailure(true);
  await expect(monitor).toContainText("状态待确认", { timeout: 8000 });
  await expect(button).toBeDisabled();
  await expect(history).toHaveAttribute("open", "");
  f.webhookFailure(false);
  await page.goto("/#automation");
  await expect(page.locator(".webhook-monitor-disclosure")).toContainText("已启用，未订阅自动化决策事件");
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});
