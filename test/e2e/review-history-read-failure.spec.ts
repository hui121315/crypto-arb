import { expect, test } from "@playwright/test";
import { reviewFixture } from "./fixtures/review-workbench";
import { NOW } from "./fixtures/opportunity-workbench";

test("history failures preserve last results across WS and pages, reject old success and recover", async ({ page }, info) => {
  const f = await reviewFixture(page);
  const original = f.snapshot();
  const problem = { code: "REVIEW_HISTORY_READ_FAILED", message: "历史记录读取失败，无法确认完整盈亏；请稍后重试", source: "trading_sql_ledger" };
  const failed = (envelope: any) => ({ ...envelope, rows: [], rowCount: 0, status: "degraded",
    ledgerStatus: "partial_evidence", missingFields: ["net"], problems: [problem], generatedAtMs: NOW + 10,
    page: { ...envelope.page, returnedCount: 0, totalRows: 0, hasMore: false, nextCursor: null } });
  await page.goto("/#review");
  await expect(page.locator(".review-executed-table tbody")).toContainText("BTC");
  await expect.poll(() => f.channelSockets.get("review")?.size ?? 0).toBeGreaterThan(0);
  f.emit({ ...original, generatedAtMs: NOW + 10, executed: failed(original.executed),
    strategyPerformance: failed(original.strategyPerformance) });
  await expect(page.locator(".review-state-disclosure summary")).toContainText("历史读取失败 · 上次数据");
  await expect(page.locator(".review-executed-table tbody")).toContainText("BTC");
  await page.getByRole("tab", { name: /策略表现/ }).click();
  await expect(page.locator(".review-strategy-table")).toBeVisible();
  await expect(page.locator(".review-state-disclosure summary")).toContainText("上次数据");
  f.emit(original);
  await page.getByRole("button", { name: "刷新复盘记录", exact: true }).click();
  await expect(page.locator(".review-state-disclosure summary")).toContainText("历史读取失败 · 上次数据");
  const recovered = structuredClone(original);
  recovered.generatedAtMs = NOW + 20;
  recovered.executed.generatedAtMs = NOW + 20;
  recovered.strategyPerformance.generatedAtMs = NOW + 20;
  f.emit(recovered);
  await expect(page.locator(".review-state-disclosure summary")).not.toContainText("历史读取失败");
  await page.getByRole("tab", { name: /执行记录/ }).click();
  let pageFails = true;
  await page.route("**/api/review/executed?**", route => pageFails
    ? route.fulfill({ json: failed(original.executed) }) : route.fallback());
  await page.getByRole("button", { name: "下一页", exact: true }).click();
  await expect(page.locator(".review-state-disclosure summary")).toContainText("历史读取失败 · 上次数据");
  await expect(page.locator(".review-executed-table tbody")).toContainText("BTC");
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    await page.screenshot({ path: info.outputPath(`review-last-result-${width}.png`) });
  }
  pageFails = false;
  await page.getByRole("button", { name: "刷新复盘记录", exact: true }).click();
  await expect(page.locator(".review-executed-table tbody")).toContainText("ETH");
  await expect(page.locator(".review-state-disclosure summary")).not.toContainText("历史读取失败");
  expect(f.writes).toEqual([]); expect(f.errors).toEqual([]);
});
