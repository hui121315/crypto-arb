import { expect, test } from "@playwright/test";
import { reviewFixture } from "./fixtures/review-workbench";

test("review rendered states", async ({ page }) => {
  const f = await reviewFixture(page);
  Object.assign(f.trade, { grossPnlUsd: -2, feeUsd: 0.41, fundingUsd: -0.1,
    slippageUsd: 1.75, netPnlUsd: -4.51, actualFields: ["gross", "fee", "funding", "slippage"],
    estimatedFields: ["net"], missingFields: [] });
  await page.goto("/#review");
  await expect(page.locator(".review-executed-table tbody")).toContainText("BTC");
  await expect(page.locator(".review-executed-table th").nth(4)).toHaveText("手续费");
  await expect(page.locator(".review-executed-table tbody td").nth(4)).toHaveText("$0.41已确认");
  await expect(page.locator(".review-executed-table tbody td").nth(6)).toContainText("-$4.51");
  await page.locator(".review-executed-table .review-evidence-action").click();
  await expect(page.locator(".review-selected-pnl > div").nth(3)).toContainText("滑点归因$1.75已确认已含成交价");
  await page.locator(".review-ledger-disclosure summary").click();
  await page.locator(".review-ledger-disclosure summary").focus();
  await page.locator('[data-trade-id="review-1"]').evaluate((node) => node.setAttribute("data-stable", "true"));
  const snapshot = f.snapshot();
  snapshot.executed.rows[0].slippageUsd = 50.75;
  snapshot.generatedAtMs += 1;
  snapshot.executed.generatedAtMs += 1;
  snapshot.strategyPerformance.generatedAtMs += 1;
  for (let i = 0; i < 20; i++) f.emit(snapshot);
  await expect(page.locator(".review-selected-pnl > div").nth(3)).toContainText("$50.75");
  await expect(page.locator(".review-selected-result")).toContainText("-$4.51");
  await expect(page.locator('[data-trade-id="review-1"]')).toHaveAttribute("data-stable", "true");
  await expect(page.locator(".review-ledger-disclosure")).toHaveAttribute("open", "");
  await expect(page.locator(".review-ledger-disclosure summary")).toBeFocused();
  await expect(page.locator(".review-mobile-context")).toBeHidden();
  await page.screenshot({ path: test.info().outputPath("executed-desktop.png"), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  const net = page.locator(".review-executed-table tbody td:nth-child(7)");
  await expect(net).toBeVisible();
  await expect(page.locator(".review-mobile-context")).toBeVisible();
  expect(await net.evaluate((node) => node.getBoundingClientRect().right <= window.innerWidth)).toBe(true);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
  await page.screenshot({ path: test.info().outputPath("executed-mobile.png"), fullPage: true });
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("review cash evidence recovery agrees with strategy totals without treating missing data as zero", async ({ page }) => {
  const f = await reviewFixture(page);
  Object.assign(f.trade, { grossPnlUsd: -2, feeUsd: 0.21, fundingUsd: 0, slippageUsd: 0,
    netPnlUsd: 999, actualFields: ["gross"], estimatedFields: [], missingFields: ["fee", "funding", "net", "slippage"] });
  Object.assign(f.perf, { totalTrades30d: 1, trades30d: 0, actualTrades30d: 0, estimatedTrades30d: 0,
    skippedTrades30d: 1, estimatedNetPnl30dUsd: 0, sampleStatus: "no_complete_sample" });
  await page.goto("/#review");
  const row = page.locator('[data-trade-id="review-1"]');
  await expect(row.locator("td").nth(6)).toContainText("数据待确认");
  await expect(row.locator("td").nth(4)).toContainText("数据待确认");
  await expect(page.locator(".review-page")).not.toContainText("$999");
  await row.getByRole("button", { name: "查看", exact: true }).click();
  await expect(page.locator(".review-selected-result")).toContainText("数据待确认");
  const snapshot = f.snapshot();
  Object.assign(snapshot.executed.rows[0], { feeUsd: 0.41, netPnlUsd: -2.41,
    actualFields: ["gross", "fee", "funding", "net"], missingFields: ["slippage"] });
  Object.assign(snapshot.strategyPerformance.rows[0], { trades30d: 1, actualTrades30d: 1, skippedTrades30d: 0,
    losingTrades30d: 1, actualNetPnl30dUsd: -2.41, netPnl30dUsd: -2.41, avgPnlPerTradeUsd: -2.41 });
  snapshot.generatedAtMs += 1;
  snapshot.executed.generatedAtMs += 1;
  snapshot.strategyPerformance.generatedAtMs += 1;
  await expect.poll(() => f.channelSockets.has("review")).toBe(true);
  f.emit(snapshot);
  await expect(page.locator(".review-selected-result")).toContainText("-$2.41已确认");
  await expect(page.locator(".review-selected-pnl > div").nth(3)).toContainText("数据待确认");
  await expect(page.locator(".review-executed-summary > div").nth(2)).toContainText("-$2.41");
  await page.getByRole("tab", { name: /策略绩效/ }).click();
  await expect(page.locator(".review-strategy-table tbody td").nth(3)).toContainText("-$2.41");
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("estimated-only strategy does not claim confirmed zero profit or no losses", async ({ page }) => {
  const f = await reviewFixture(page);
  await page.goto("/#review");
  await page.getByRole("tab", { name: /策略绩效/ }).click();
  const row = page.locator(".review-strategy-table tbody tr");
  await expect(row.locator("td").nth(3)).not.toContainText("$0.00");
  await row.getByRole("button", { name: "查看", exact: true }).click();
  await expect(page.locator(".review-strategy-detail")).not.toContainText("无亏损样本");
  await expect(page.locator(".review-strategy-detail")).toContainText("待确认");
  await row.getByRole("button", { name: "收起", exact: true }).focus();
  const snapshot = f.snapshot();
  snapshot.strategyPerformance.rows[0].estimatedNetPnl30dUsd = 7;
  f.emit(snapshot);
  await expect(page.locator(".review-strategy-detail")).toContainText("+$7.00");
  await expect(row.getByRole("button", { name: "收起", exact: true })).toBeFocused();
  expect(f.errors).toEqual([]);
});

test("runtime error remains visible when returning from a history page", async ({ page }) => {
  const f = await reviewFixture(page);
  await page.goto("/#review");
  await page.getByRole("button", { name: "下一页", exact: true }).click();
  await expect(page.locator(".review-executed-table tbody")).toContainText("ETH");
  f.failRuntime(true);
  await page.getByRole("button", { name: "刷新复盘记录" }).click();
  await expect(page.getByRole("button", { name: "刷新复盘记录" })).toBeEnabled();
  await page.getByRole("button", { name: "首页", exact: true }).click();
  await expect(page.locator(".review-executed-table tbody")).toContainText("BTC");
  await expect(page.locator(".review-state-disclosure")).toContainText("REVIEW_FIXTURE_UNAVAILABLE");
  f.failRuntime(false);
  await page.getByRole("button", { name: "刷新复盘记录" }).click();
  await expect(page.locator(".review-state-disclosure")).not.toContainText("REVIEW_FIXTURE_UNAVAILABLE");
  expect(f.errors).toEqual([]);
});

test("same-generation WS invalidates an in-flight HTTP error and leaves only one initial request", async ({ page }) => {
  const f = await reviewFixture(page);
  f.failRuntime(true);
  f.holdRuntime();
  await page.goto("/#review");
  await expect.poll(() => f.reads.filter((path) => path.endsWith("/runtime")).length).toBe(1);
  await expect.poll(() => f.channelSockets.has("review")).toBe(true);
  f.emit();
  await expect(page.locator(".review-executed-table tbody")).toContainText("BTC");
  const failed = page.waitForResponse((response) => response.url().includes("/review/runtime") && response.status() === 503);
  f.releaseRuntime();
  await (await failed).finished();
  await expect(page.getByRole("button", { name: "刷新复盘记录" })).toBeEnabled();
  await expect(page.locator(".review-state-disclosure")).not.toContainText("REVIEW_FIXTURE_UNAVAILABLE");
  expect(f.errors).toEqual([]);
});

test("failed page preserves prior data and refresh retries the requested page", async ({ page }) => {
  const f = await reviewFixture(page);
  await page.goto("/#review");
  f.failPage(true);
  await page.getByRole("button", { name: "下一页", exact: true }).click();
  await expect(page.locator(".review-state-disclosure")).toContainText("显示上次快照");
  await expect(page.locator(".review-executed-table tbody")).toContainText("BTC");
  f.failPage(false);
  await page.getByRole("button", { name: "刷新复盘记录" }).click();
  await expect(page.locator(".review-executed-table tbody")).toContainText("ETH");
  f.emit();
  await expect(page.locator(".review-executed-table tbody")).toContainText("ETH");
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("quality evidence stays expanded across refresh and missed time is local", async ({ page }) => {
  const f = await reviewFixture(page);
  await page.goto("/#review");
  await page.getByRole("tab", { name: /场所质量/ }).click();
  const row = page.locator(".venue-quality-table tbody tr").filter({ hasText: "binance" });
  await row.getByRole("button").click();
  const disclosure = page.locator(".review-quality-healthy-disclosure");
  await disclosure.locator("summary").click();
  f.qualityMessage("fixture version 2");
  await page.getByRole("button", { name: "刷新复盘记录" }).click();
  await expect(disclosure).toContainText("fixture version 2");
  await expect(disclosure).toHaveAttribute("open", "");
  await page.screenshot({ path: test.info().outputPath("quality-desktop.png"), fullPage: true });
  await page.getByRole("tab", { name: /错失机会/ }).click();
  await expect(page.locator(".review-attribution")).toContainText("当前页");
  await expect(page.locator(".review-missed-table tbody td").nth(1)).toContainText("09-24");
  const pager = page.locator("#review-panel-missed .table-pager-bar");
  await pager.evaluate((node) => node.setAttribute("data-stable", "true"));
  await page.getByRole("button", { name: "刷新复盘记录" }).click();
  await expect(pager).toHaveAttribute("data-stable", "true");
  await page.setViewportSize({ width: 390, height: 844 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
  await page.screenshot({ path: test.info().outputPath("missed-mobile.png"), fullPage: true });
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("a delayed history failure cannot overwrite data after leaving and returning", async ({ page }) => {
  const f = await reviewFixture(page);
  await page.goto("/#review");
  await expect(page.locator(".review-executed-table tbody")).toContainText("BTC");
  f.failPage(true);
  f.holdPage();
  await page.getByRole("button", { name: "下一页", exact: true }).click();
  await expect.poll(() => f.reads.filter((path) => path.includes("/executed?")).length).toBe(1);
  await page.evaluate(() => { location.hash = "futures"; });
  await expect(page.locator(".review-page")).toHaveCount(0);
  f.failPage(false);
  await page.evaluate(() => { location.hash = "review"; });
  await expect(page.locator(".review-executed-table tbody")).toContainText("ETH");
  const failed = page.waitForResponse((response) => response.url().includes("/review/executed?") && response.status() === 503);
  f.releasePage();
  await (await failed).finished();
  await expect(page.locator(".review-state-disclosure")).not.toContainText("REVIEW_FIXTURE_UNAVAILABLE");
  await expect(page.locator(".review-executed-table tbody")).toContainText("ETH");
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});
