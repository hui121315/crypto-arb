import { expect, test, type Page } from "@playwright/test";
import { reviewFixture } from "./fixtures/review-workbench";
import { API, NOW } from "./fixtures/opportunity-workbench";

const original = "isolated-fixture-token";
const other = "review-other-login";
const review = (page: Page) => page.getByRole("button", { name: /^切换到复盘(?:，|$)/ }).click();
async function login(page: Page, token: string) {
  await page.getByRole("button", { name: "切换到设置", exact: true }).click();
  await page.getByRole("tab", { name: "诊断", exact: true }).click();
  await page.getByRole("tab", { name: "连接", exact: true }).click();
  await page.locator(".settings-api-token-task input").fill(token);
  await page.getByRole("button", { name: "保存 Token", exact: true }).click();
}

function settlement(source: string, id: string, title: string) {
  return { rows: [{ source, id, title, executionState: "已成交，收支待核算",
    accountingState: "仅原币变化，非已实现利润", attention: true, updatedAtMs: NOW,
    amounts: [{ label: "USDC 变化", asset: "USDC", amount: "45.67" },
      { label: "净收益", asset: "USD", amount: null }],
    references: [["订单", id]], notes: ["fixture saved receipt"] }],
    observedAtMs: NOW, truncated: false, problems: [] };
}

test("review login switch isolates all five feeds and does not revive old history after A-B-A", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const f = await reviewFixture(page);
  const quality = await (await page.request.get(`${API}/api/trading/venues/quality`)).json();
  const missed = await (await page.request.get(`${API}/e2e-large-tables/api/review/missed?limit=1`)).json();
  const paths = ["/api/review/runtime", "/api/review/executed", "/api/review/missed",
    "/api/trading/venues/quality", "/api/review/settlements"];
  const requests: { path: string; auth: string }[] = [];
  const pending = new Map<string, () => void>();
  let hold = false;
  await page.route("**/api/**", async route => {
    const url = new URL(route.request().url());
    if (url.origin !== API || !paths.includes(url.pathname)) return route.fallback();
    const auth = route.request().headers().authorization ?? "";
    requests.push({ path: url.pathname, auth });
    const changed = auth === `Bearer ${other}`;
    const snapshot = f.snapshot();
    snapshot.executed.rows[0].symbol = changed ? "SOL" : "BTC";
    snapshot.executed.rows[0].id = changed ? "current-login-trade" : "original-login-trade";
    snapshot.strategyPerformance.rows[0].estimatedNetPnl30dUsd = changed ? 8 : 4;
    const missedRows = structuredClone(missed);
    missedRows.rows[0].symbol = changed ? "CURRENT-MISSED" : "ORIGINAL-MISSED";
    const q = structuredClone(quality);
    q.rows[0].operationHealth = [{ venue: "binance", operation: "positions_read", status: "ok",
      source: "fixture.review-login", message: changed ? "current login quality" : "original login quality", observedAtMs: NOW }];
    const body = url.pathname.endsWith("/runtime") ? snapshot
      : url.pathname.endsWith("/executed") ? f.envelope([{ ...f.trade, symbol: "OLD-LATE-PAGE" }], 1)
      : url.pathname.endsWith("/missed") ? missedRows
      : url.pathname.endsWith("/quality") ? q
      : settlement("onchain", changed ? "current-receipt" : "old-receipt", changed ? "Current receipt" : "Old receipt");
    if (hold && !pending.has(url.pathname)) await new Promise<void>(resolve => pending.set(url.pathname, resolve));
    await route.fulfill({ json: body });
  });
  await page.goto("/#review");
  await expect(page.locator(".review-executed-table tbody")).toContainText("BTC");
  await expect.poll(() => requests.some(r => r.path.endsWith("/missed"))).toBe(true);
  await expect.poll(() => requests.some(r => r.path.endsWith("/quality"))).toBe(true);
  await expect(page.getByRole("button", { name: "刷新复盘记录" })).toBeEnabled();
  hold = true;
  await page.getByRole("button", { name: "刷新复盘记录" }).click();
  await expect.poll(() => pending.size).toBe(3);
  await page.getByRole("button", { name: "下一页", exact: true }).click();
  await page.getByRole("tab", { name: /链上 \/ 股票/ }).click();
  await expect.poll(() => pending.size).toBe(5);
  await login(page, other);
  hold = false;
  const replies = paths.map(path => page.waitForResponse(r => new URL(r.url()).pathname === path));
  for (const release of pending.values()) release();
  await Promise.all((await Promise.all(replies)).map(response => response.finished()));
  await review(page);
  const notice = page.locator(".review-connection-notice");
  await expect(notice).toContainText("连接已改变");
  await expect(page.locator(".review-tab-panel")).toHaveCount(0);
  await expect(page.getByTestId("top-status-bar")).toContainText("连接已改变，待刷新");
  const readsBefore = requests.length;
  await login(page, original);
  await review(page);
  await expect(notice).toContainText("连接已改变");
  expect(requests.length).toBe(readsBefore);
  await page.screenshot({ path: test.info().outputPath("login-boundary-desktop.png"), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  const reload = page.getByRole("button", { name: "刷新当前连接", exact: true });
  await reload.scrollIntoViewIfNeeded();
  expect(await reload.evaluate(el => {
    const r = el.getBoundingClientRect();
    return el.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2));
  })).toBe(true);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
  await page.screenshot({ path: test.info().outputPath("login-boundary-mobile.png") });
  await page.setViewportSize({ width: 1440, height: 900 });
  await login(page, other);
  await review(page);
  await reload.click();
  await expect(page.locator(".review-executed-table tbody")).toContainText("SOL");
  await expect(page.locator(".review-page")).not.toContainText("OLD-LATE-PAGE");
  await page.getByRole("tab", { name: /错失机会/ }).click();
  await expect(page.locator(".review-missed-table")).toContainText("CURRENT-MISSED");
  await expect(page.locator(".review-missed-table")).not.toContainText("ORIGINAL-MISSED");
  await page.getByRole("tab", { name: /策略绩效/ }).click();
  await page.locator(".review-strategy-table").getByRole("button", { name: "查看", exact: true }).click();
  await expect(page.locator(".review-strategy-detail")).toContainText("+$8.00");
  await page.getByRole("tab", { name: /场所质量/ }).click();
  await page.locator(".venue-quality-table tbody tr").filter({ hasText: "binance" }).getByRole("button").click();
  await expect(page.locator(".review-page")).toContainText("current login quality");
  await page.getByRole("tab", { name: /链上 \/ 股票/ }).click();
  await expect(page.locator(".review-settlement-list")).toContainText("Current receipt");
  await expect(page.locator(".review-settlement-list")).not.toContainText("Old receipt");
  await page.getByRole("tab", { name: /执行记录/ }).click();
  const currentSnapshot = f.snapshot();
  currentSnapshot.generatedAtMs += 1;
  currentSnapshot.executed.generatedAtMs += 1;
  currentSnapshot.strategyPerformance.generatedAtMs += 1;
  currentSnapshot.executed.rows[0].symbol = "NEW-WS";
  currentSnapshot.executed.rows[0].id = "current-login-trade";
  await expect.poll(() => f.channelSockets.get("review")?.size ?? 0).toBeGreaterThan(0);
  f.emit(currentSnapshot);
  await expect(page.locator(".review-executed-table tbody")).toContainText("NEW-WS");
  await expect(page.locator(".review-page")).toContainText("后端保留历史 · 非当前持仓");
  expect(requests.slice(readsBefore).every(r => r.auth === `Bearer ${other}`)).toBe(true);
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});

test("settlement source changes bypass slow reads and keep exact receipt and missing profit honest", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const f = await reviewFixture(page);
  const queries: URL[] = [];
  let release: (() => void) | undefined, failed = false, onchainReads = 0;
  await page.route("**/api/review/settlements?*", async route => {
    const url = new URL(route.request().url());
    queries.push(url);
    const source = url.searchParams.get("source") ?? "all", record = url.searchParams.get("record");
    let value = settlement(source === "all" ? "onchain" : source, record ?? `${source}-receipt`, `${source} current receipt`);
    if (record === "missing") value.rows = [];
    if (source === "onchain" && ++onchainReads === 1) {
      value.rows[0].title = "OBSOLETE onchain receipt";
      await new Promise<void>(resolve => { release = resolve; });
    }
    if (failed) return route.fulfill({ status: 503, json: { error: {
      code: "REVIEW_READ_FAILED", message: "fixture settlement read failed", source: "fixture.review" } } });
    return route.fulfill({ json: value });
  });
  await page.goto("/#review");
  await page.getByRole("tab", { name: /链上 \/ 股票/ }).click();
  const list = page.locator(".review-settlement-list"), source = page.getByLabel("收支记录来源");
  await expect(list).toContainText("all current receipt");
  await source.selectOption("onchain");
  await expect.poll(() => !!release).toBe(true);
  await source.selectOption("stocks");
  await expect(list).toContainText("stocks current receipt");
  await source.selectOption("onchain");
  await expect(list).toContainText("onchain current receipt");
  const late = page.waitForResponse(r => r.url().includes("/review/settlements?") && new URL(r.url()).searchParams.get("source") === "onchain");
  release!(); await (await late).finished();
  await expect(list).not.toContainText("OBSOLETE");
  await expect(page.getByRole("button", { name: "刷新复盘记录" })).toBeEnabled();
  failed = true;
  await page.getByRole("button", { name: "刷新复盘记录" }).click();
  await expect(page.locator(".review-state-disclosure")).toContainText("刷新失败 · 显示上次记录");
  await expect(list).toContainText("onchain current receipt");
  failed = false;
  await page.getByRole("button", { name: "刷新复盘记录" }).click();
  await expect(page.locator(".review-state-disclosure")).toContainText("已保存处理结果 · 只读复盘");
  await page.evaluate(() => { location.hash = "review?source=stocks&record=missing"; });
  await expect(page.getByText("未找到这条原记录；不会替换为其他交易，也不代表未成交或收益为零。")).toBeVisible();
  await expect(list.locator("article")).toHaveCount(0);
  await page.getByRole("button", { name: "查看该来源记录", exact: true }).click();
  await expect(list).toContainText("stocks current receipt");
  await expect(list).toContainText("45.67 USDC");
  await expect(list).toContainText("待核算");
  await list.locator("summary").click();
  await page.screenshot({ path: test.info().outputPath("settlement-desktop.png"), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.evaluate(() => window.scrollTo(0, 0));
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
  await page.screenshot({ path: test.info().outputPath("settlement-mobile.png"), fullPage: true });
  expect(queries.some(q => q.searchParams.get("record") === "missing")).toBe(true);
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});
