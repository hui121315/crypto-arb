import { expect, test, type Page } from "@playwright/test";
import { reviewFixture } from "./fixtures/review-workbench";
import { API, NOW } from "./fixtures/opportunity-workbench";

const refresh = (page: Page) => page.getByRole("button", { name: "刷新复盘记录", exact: true });
const tab = (page: Page, name: string) => page.getByRole("tab", { name: new RegExp(name) });
const paths = ["/api/review/runtime", "/api/review/executed", "/api/review/missed",
  "/api/trading/venues/quality", "/api/review/settlements"];

async function fixture(page: Page) {
  const f = await reviewFixture(page);
  await page.clock.install({ time: NOW });
  const quality = await (await page.request.get(API + "/api/trading/venues/quality")).json();
  const missed = await (await page.request.get(API + "/e2e-large-tables/api/review/missed?limit=1")).json();
  const waiting = new Map<string, () => void>();
  const requests: string[] = [], aborted: string[] = [];
  let hold = false;
  page.on("requestfailed", request => {
    const path = new URL(request.url()).pathname;
    if (paths.includes(path)) aborted.push(path);
  });
  await page.route("**/api/**", async route => {
    const path = new URL(route.request().url()).pathname;
    if (!paths.includes(path)) return route.fallback();
    requests.push(path);
    const body = path.endsWith("/runtime") ? f.snapshot()
      : path.endsWith("/quality") ? quality
      : path.endsWith("/missed") ? missed
      : path.endsWith("/executed") ? f.envelope([{ ...f.trade, id: "page-two", symbol: "ETH" }], 1)
      : { rows: [{ source: "stocks", id: "stock-receipt", title: "Saved stock receipt",
          executionState: "已成交", accountingState: "收支待核算", attention: true, updatedAtMs: NOW,
          amounts: [{ label: "实际变化", asset: "USDC", amount: "45.67" },
            { label: "净收益", asset: "USD", amount: null }], references: [], notes: [] }],
        observedAtMs: NOW, truncated: false, problems: [] };
    if (hold) await new Promise<void>(resolve => waiting.set(path, resolve));
    await route.fulfill({ json: body });
  });
  return { ...f, requests, aborted, waiting, hold: (value: boolean) => { hold = value; },
    release: () => { for (const release of waiting.values()) release(); waiting.clear(); } };
}

test("all review reads stop waiting, preserve records and retry without financial writes", async ({ page }, info) => {
  const f = await fixture(page);
  await page.goto("/#review");
  await expect(page.locator(".review-executed-table tbody")).toContainText("BTC");
  await tab(page, "链上 / 股票").click();
  await expect(page.locator(".review-settlement-list")).toContainText("45.67 USDC");
  await tab(page, "执行记录").click();
  await expect(refresh(page)).toBeEnabled();
  f.hold(true);
  const start = f.requests.length;
  await refresh(page).click();
  await expect.poll(() => f.waiting.size).toBe(3);
  await page.getByRole("button", { name: "下一页", exact: true }).click();
  await tab(page, "链上 / 股票").click();
  await expect.poll(() => f.waiting.size).toBe(5);
  await page.clock.runFor(15_100);
  await expect(refresh(page)).toBeEnabled();
  await expect(page.locator(".review-state-disclosure")).toContainText("刷新失败");
  await expect(page.locator(".review-state-disclosure p")).toContainText("15 秒");
  await expect(page.locator(".review-settlement-list")).toContainText("45.67 USDC");
  await expect(page.locator(".review-settlement-list")).toContainText("待核算");
  for (const path of paths) {
    expect(f.requests.slice(start).filter(p => p === path)).toHaveLength(1);
    await expect.poll(() => f.aborted.includes(path)).toBe(true);
  }
  for (const name of ["执行记录", "未执行机会", "交易所表现", "策略表现"]) {
    await tab(page, name).click();
    await expect(refresh(page)).toBeEnabled();
    await expect(page.locator(".review-state-disclosure p")).toContainText("15 秒");
  }
  f.hold(false); f.release();
  await refresh(page).click();
  await tab(page, "执行记录").click();
  await expect(page.locator(".review-executed-table tbody")).toContainText("ETH");
  await expect(page.locator(".review-state-disclosure p")).not.toContainText("15 秒");
  await page.screenshot({ path: info.outputPath("review-recovered-desktop.png") });
  await tab(page, "链上 / 股票").click();
  await expect(page.locator(".review-state-disclosure")).toContainText("已保存处理结果");
  await page.setViewportSize({ width: 390, height: 844 });
  await refresh(page).scrollIntoViewIfNeeded();
  expect(await refresh(page).evaluate(el => {
    const r = el.getBoundingClientRect();
    return el.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2));
  })).toBe(true);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
  await page.screenshot({ path: info.outputPath("review-recovered-mobile.png") });
  expect(f.writes).toEqual([]); expect(f.errors).toEqual([]);
});

test("fresh review pushes cancel obsolete reads and leaving cancels remaining requests", async ({ page }) => {
  const f = await fixture(page);
  await page.goto("/#review");
  await expect(page.locator(".review-executed-table tbody")).toContainText("BTC");
  await expect.poll(() => f.channelSockets.get("review")?.size ?? 0).toBeGreaterThan(0);
  f.hold(true);
  await refresh(page).click();
  await expect.poll(() => f.waiting.size).toBe(3);
  const newer = f.snapshot();
  newer.generatedAtMs++; newer.executed.generatedAtMs++; newer.strategyPerformance.generatedAtMs++;
  newer.executed.rows[0].symbol = "FRESH-WS";
  f.emit(newer);
  await expect(page.locator(".review-executed-table tbody")).toContainText("FRESH-WS");
  await expect(refresh(page)).toBeEnabled();
  await expect.poll(() => f.aborted.includes("/api/review/runtime")).toBe(true);
  await tab(page, "链上 / 股票").click();
  await expect.poll(() => f.waiting.size).toBe(4);
  await page.getByRole("button", { name: /^切换到期货套利(?:，|$)/ }).click();
  for (const path of paths.filter(path => !path.endsWith("/executed"))) {
    await expect.poll(() => f.aborted.includes(path)).toBe(true);
  }
  f.hold(false); f.release();
  await page.getByRole("button", { name: /^切换到复盘(?:，|$)/ }).click();
  await expect(page.locator(".review-settlement-list")).toContainText("Saved stock receipt");
  await tab(page, "执行记录").click();
  await expect(page.locator(".review-executed-table tbody")).toContainText("FRESH-WS");
  expect(f.writes).toEqual([]); expect(f.errors).toEqual([]);
});
