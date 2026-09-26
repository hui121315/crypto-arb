import { expect, test } from "@playwright/test";
import { reviewFixture } from "./fixtures/review-workbench";

test("review keeps simulated profits out of live totals and selects same-strategy environments independently", async ({ page }) => {
  const f = await reviewFixture(page);
  const snapshot = f.snapshot();
  const trade = (id: string, net: number, mode: string) => {
    const row = structuredClone(f.trade);
    Object.assign(row, { id, netPnlUsd: net, actualFields: ["net"], estimatedFields: [], missingFields: [] });
    for (const order of [...row.longOrders, ...row.shortOrders]) order.intent.mode = mode;
    return row;
  };
  const live = trade("live-loss", -2, "live"), paper = trade("paper-gain", 1000, "dry_run");
  expect(live.longOrders.length).toBeGreaterThan(0);
  expect(live.shortOrders.length).toBeGreaterThan(0);
  const unknown = trade("unknown", 600, "live");
  unknown.shortOrders = [];
  const mixed = trade("mixed", 700, "live");
  mixed.shortOrders[0].intent.mode = "dry_run";
  const estimated = trade("estimated", 4, "live");
  estimated.actualFields = []; estimated.estimatedFields = ["net"];
  const missing = trade("missing", 900, "live"); missing.missingFields = ["net"];
  const perf = (environment: string | null, net: number, count = 1) => ({ ...structuredClone(f.perf),
    executionEnvironment: environment, totalTrades30d: count, trades30d: count, actualTrades30d: count,
    estimatedTrades30d: 0, actualNetPnl30dUsd: net, netPnl30dUsd: net, estimatedNetPnl30dUsd: 0,
    avgPnlPerTradeUsd: net / count, profitableTrades30d: net > 0 ? count : 0,
    losingTrades30d: net < 0 ? count : 0, breakEvenTrades30d: net === 0 ? count : 0,
    maxDrawdownUsd: net < 0 ? -net : 0, independentPeriods30d: count,
    hitRatePct: net > 0 ? 100 : 0, sampleStatus: "complete" });
  snapshot.executed = f.envelope([live, paper, unknown, mixed, estimated, missing]);
  snapshot.executed.page.totalRows = 6;
  snapshot.strategyPerformance = f.envelope([
    { ...perf("live", -2), totalTrades30d: 3, trades30d: 2, estimatedTrades30d: 1,
      estimatedNetPnl30dUsd: 4, skippedTrades30d: 1, sampleStatus: "partial_evidence" },
    perf("paper", 1000), perf(null, 1300, 2),
  ]);
  snapshot.strategyPerformance.page.totalRows = 3;
  f.emit(snapshot);
  await page.goto("/#review");
  const summary = page.locator(".review-executed-summary");
  await expect(summary.locator("div").nth(1)).toContainText("实盘已确认净收益-$2.00");
  await expect(summary.locator("div").nth(1)).toContainText("估算 +$4.00");
  await expect(summary.locator("div").nth(2)).toContainText("模拟已确认净收益+$1000.00");
  await expect(summary.locator("div").nth(3)).toContainText("2 笔环境待核对");
  await expect(page.locator('[data-trade-id="missing"] td').nth(6)).toContainText("数据待确认");
  await expect(page.locator('[data-trade-id="unknown"]')).toContainText("环境未知");
  await expect(page.locator('[data-trade-id="mixed"]')).toContainText("环境混合");
  await page.getByRole("tab", { name: /策略绩效/ }).click();
  const table = page.locator(".review-strategy-table");
  await expect(table.locator("tbody tr")).toHaveCount(3);
  await expect(page.locator(".review-strategy-summary > div").nth(1)).toContainText("实盘已确认净收益-$2.00");
  const liveRow = table.locator('tr[data-environment="live"]');
  const paperRow = table.locator('tr[data-environment="paper"]');
  const detail = page.locator(".review-strategy-detail");
  const metric = (label: string) => detail.locator(".review-strategy-metrics > div")
    .filter({ has: page.locator("span", { hasText: new RegExp(`^${label}$`) }) });
  await paperRow.getByRole("button", { name: "查看", exact: true }).click();
  await expect(detail.locator("header")).toContainText("模拟");
  await expect(detail).toContainText("+$1000.00");
  await liveRow.getByRole("button", { name: "查看", exact: true }).click();
  await expect(detail.locator("header")).toContainText("实盘");
  await expect(detail).not.toContainText("+$1000.00");
  await expect(detail).not.toContainText("无亏损样本");
  await expect(metric("最差单笔")).toContainText("待确认");
  await expect(metric("已确认单笔")).toContainText("-$2.00");
  await expect(metric("已确认盈 / 亏 / 平")).toContainText("0 / 1 / 0");
  await expect(metric("最大回撤")).toContainText("$2.00");
  await expect(metric("Profit Factor")).toContainText("未知");
  const update = f.snapshot();
  Object.assign(update.strategyPerformance.rows[1], { actualNetPnl30dUsd: 2000, netPnl30dUsd: 2000, avgPnlPerTradeUsd: 2000 });
  update.generatedAtMs++; update.executed.generatedAtMs++; update.strategyPerformance.generatedAtMs++;
  f.emit(update);
  await expect(paperRow).toContainText("+$2000.00");
  await expect(detail.locator("header")).toContainText("实盘");
  await expect(detail).toContainText("-$2.00");
  await page.screenshot({ path: test.info().outputPath("review-environments-desktop.png"), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
  const net = paperRow.locator("td").nth(3);
  await expect(net).toBeVisible();
  expect(await net.evaluate((node) => {
    const box = node.getBoundingClientRect();
    return box.left >= 0 && box.right <= innerWidth && node.contains(document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2));
  })).toBe(true);
  await paperRow.getByRole("button", { name: "查看", exact: true }).click();
  await expect(detail.locator("header")).toContainText("模拟");
  await expect(metric("已确认单笔")).toContainText("+$2000.00");
  await expect(metric("已确认盈 / 亏 / 平")).toContainText("1 / 0 / 0");
  await expect(metric("最大回撤")).toContainText("$0.00");
  await page.screenshot({ path: test.info().outputPath("review-environments-mobile.png"), fullPage: true });
  // Old servers omit environment: retain the record without adopting the current mode.
  const legacy = f.snapshot();
  delete legacy.strategyPerformance.rows[0].executionEnvironment;
  legacy.strategyPerformance.rows = [legacy.strategyPerformance.rows[0]];
  legacy.generatedAtMs++; legacy.strategyPerformance.generatedAtMs++;
  f.emit(legacy);
  await expect(table.locator("tbody tr")).toHaveCount(1);
  await expect(table).toContainText("环境待核对");
  await expect(page.locator(".review-strategy-summary > div").nth(1)).toContainText("实盘已确认净收益—");
  await expect(detail).toHaveCount(0);
  const noActual = f.snapshot();
  Object.assign(noActual.strategyPerformance.rows[0], { actualTrades30d: 0, actualNetPnl30dUsd: 0,
    netPnl30dUsd: 0, avgPnlPerTradeUsd: 0, maxDrawdownUsd: 0, sampleStatus: "no_complete_sample" });
  noActual.generatedAtMs++; noActual.strategyPerformance.generatedAtMs++;
  f.emit(noActual);
  await table.getByRole("button", { name: "查看", exact: true }).click();
  await expect(metric("已确认单笔")).toContainText("—");
  await expect(metric("已确认盈 / 亏 / 平")).toContainText("待确认");
  await expect(metric("最大回撤")).toContainText("待确认");
  expect(f.reads.filter((path) => path === "/api/review/runtime")).toHaveLength(1);
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});
