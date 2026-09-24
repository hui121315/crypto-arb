import { expect, test } from "@playwright/test";
import { setup, API, NOW } from "./fixtures/opportunity-workbench";

async function scanner(page: Parameters<typeof setup>[0], paginated = false) {
  const f = await setup(page, paginated);
  let detailFailure = false;
  let heldDetail = false;
  let releaseDetail: (() => void) | undefined;
  let releaseTest: (() => void) | undefined;
  let failTest = false;
  let testCount = 0;
  let webhookFailure = false;
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
    return route.fulfill({ json: body });
  });
  await page.route(`${API}/api/webhook/status`, (route) => webhookFailure
    ? route.fulfill({ status: 503, json: { code: "WEBHOOK_STATUS_UNAVAILABLE", message: "fixture: status unavailable" } })
    : route.fulfill({ json: webhook }));
  await page.route(`${API}/api/webhook/test`, async (route) => {
    testCount++;
    await new Promise<void>((resolve) => { releaseTest = resolve; });
    return failTest ? route.fulfill({ status: 503, json: { code: "TEST_REJECTED", message: "fixture: enqueue unavailable" } })
      : route.fulfill({ json: { eventId: "fixture-test", queued: true } });
  });
  return { ...f, requests, webhook,
    detailFailure: (value: boolean) => { detailFailure = value; },
    holdDetail: () => { heldDetail = true; },
    releaseDetail: () => { heldDetail = false; releaseDetail?.(); },
    testCount: () => testCount,
    finishTest: (failed = false) => { failTest = failed; releaseTest?.(); },
    webhookFailure: (value: boolean) => { webhookFailure = value; },
  };
}

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
  const funding = detail.locator("details").filter({ has: page.locator("summary", { hasText: "Funding 周期" }) });
  await funding.locator("summary").click();
  const flow = page.locator(".opportunity-flow-details");
  await flow.locator("summary").click();
  const current = table.getByRole("button", { name: "当前证据", exact: true });
  await current.focus();
  for (const width of [1440, 1024, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await page.screenshot({ path: test.info().outputPath(`opportunities-${width}.png`), fullPage: true });
    const box = await table.locator("tbody tr[id]").first().locator("td").nth(4).boundingBox();
    const action = await table.locator("tbody tr[id]").first().locator("td").last().boundingBox();
    expect(box!.x + box!.width).toBeLessThanOrEqual(action!.x + 1);
    await expect(table.locator(".opportunity-mobile-route").first()).toBeVisible({ visible: width <= 640 });
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
  const evidence = detail.locator("details").filter({ has: page.locator("summary", { hasText: "数据证据" }) });
  await evidence.locator("summary").click();
  f.detailFailure(false);
  f.holdDetail();
  const n = f.requests.length;
  await detail.getByRole("button", { name: "刷新证据", exact: true }).click();
  await expect.poll(() => f.requests.length).toBe(n + 1);
  await expect(detail.getByRole("button", { name: "刷新证据", exact: true })).toBeDisabled();
  f.rows.forEach((row) => { row.cost.oneCycleNetBps = 25; row.metrics.oneCycleNetBps = 25; });
  f.tick();
  f.releaseDetail();
  await expect(detail.getByRole("button", { name: "刷新证据", exact: true })).toBeEnabled();
  await expect(detail).not.toContainText("DETAIL_UNAVAILABLE");
  await expect(evidence).toHaveAttribute("open", "");
  await expect(detail.locator(".detail-metrics")).toContainText("+0.250%");
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
  await expect(monitor).toContainText("测试请求已受理；实际送达以最近投递回执为准");
  await expect(monitor).not.toContainText("TEST_REJECTED");
  const history = monitor.locator(".webhook-monitor-history");
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
