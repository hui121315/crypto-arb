import { expect, test } from "@playwright/test";
import { submissionFixture } from "./fixtures/execution-submission";
import { API, NOW } from "./fixtures/opportunity-workbench";

test("review opens the exact historical run, showing closure without inventing a two-leg fill or a new ticket", async ({ page }) => {
  const f = await submissionFixture(page);
  const run = f.makeRun({ idempotencyKey: "history /1&", ticketId: "ticket /1&" }, "closed");
  run.shortLeg.state = "rejected";
  run.shortLeg.filledQuantity = 0;
  run.shortLeg.filledNotionalUsd = 0;
  run.statusReason = "补偿单成交，裸露已关闭";
  f.setRuns([run]);
  f.setOrders([f.makeOrder(run.longLeg.orderIds[0], "filled", NOW + 10),
    f.makeOrder(run.shortLeg.orderIds[0], "rejected", NOW + 10)]);
  const seed = await (await page.request.get(`${API}/e2e-large-tables/api/review/executed?limit=1`)).json();
  const trade = { ...seed.rows[0], id: "history-trade", evidence: { ...seed.rows[0].evidence,
    ledgerEvents: [], closeRunEvidence: [{ closeRunId: "closed-history", status: "succeeded",
      runId: run.runId, ticketId: run.ticketId, opportunityId: run.opportunityId,
      matchedNotionalUsd: 10, unwindStatus: null, compensationAttemptCount: 0, costReconciliation: null }] } };
  const executed = {
    ...seed, rows: [trade], days: 365, rowCount: 1, status: "fresh", problems: [],
    page: { ...seed.page, totalRows: 1, returnedCount: 1, hasMore: false, hasNextPage: false, nextCursor: null },
  };
  const strategyPerformance = await (await page.request.get(`${API}/api/review/strategy-performance`)).json();
  await page.route(`${API}/api/review/runtime`, (route) => route.fulfill({ json: {
    executed, strategyPerformance, generatedAtMs: NOW,
  } }));
  await page.route(`${API}/api/review/executed**`, (route) => route.fulfill({ json: executed }));
  const params = new URLSearchParams({ run: run.runId, ticket: run.ticketId, opp: run.opportunityId });
  let releaseHistory!: () => void;
  const readGate = new Promise<void>((resolve) => { releaseHistory = resolve; });
  let historyHeld = true;
  const requested: URL[] = [];
  await page.route(`${API}/api/trading/execution-runs?*`, async (route) => {
    requested.push(new URL(route.request().url()));
    if (historyHeld) await readGate;
    await route.fallback();
  });
  await page.goto(`/#review?${params}`);
  await page.getByRole("link", { name: "查看原执行", exact: true }).click();
  const flow = page.locator(".execution-flow-overview");
  await expect(flow).toContainText("读取原执行记录");
  await expect(page.getByRole("navigation", { name: "选择执行机会" })).toHaveCount(0);
  await expect(page.locator(".confirm-action.primary")).toHaveCount(0);
  await expect.poll(() => requested.length).toBeGreaterThan(0);
  const query = requested.at(-1)!.searchParams;
  expect(query.get("runId")).toBe(run.runId);
  expect(query.get("ticketId")).toBe(run.ticketId);
  expect(query.get("opportunityId")).toBe(run.opportunityId);
  historyHeld = false;
  releaseHistory();
  await expect(flow.locator(".execution-flow-current strong")).toHaveText("执行已收口");
  await flow.locator("summary").click();
  await expect(flow.locator("li")).toHaveCount(4);
  await expect(flow).toContainText("原订单与补偿最终结果待核对");
  await expect(flow).not.toContainText("双腿已平仓");
  await expect(flow).not.toContainText("双腿最终结果已确认");
  await expect(flow).not.toContainText("等待选择机会");
  await expect(page.locator(".queue-overview-copy > strong")).toHaveText("执行已收口");
  const actions = page.getByRole("navigation", { name: "历史执行后续操作" });
  await expect(actions.getByRole("link", { name: "去持仓平仓" })).toHaveCount(0);
  await expect(actions.getByRole("link", { name: "关联复盘" })).toHaveAttribute("href", `#review?${params}`);
  await page.screenshot({ path: test.info().outputPath("historical-closure-desktop.png"), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
  await actions.getByRole("link", { name: "关联复盘" }).click({ trial: true });
  await page.screenshot({ path: test.info().outputPath("historical-closure-mobile.png"), fullPage: true });
  await actions.getByRole("link", { name: "关联复盘" }).click();
  await expect(page.locator(".review-record-scope[role='status']")).toContainText(run.runId);
  await page.getByRole("link", { name: "查看原执行", exact: true }).click();
  await expect(flow.locator(".execution-flow-current strong")).toHaveText("执行已收口");
  expect(f.previews).toHaveLength(0); expect(f.builds).toHaveLength(0);
  expect(f.confirms).toHaveLength(0); expect(f.cancels).toHaveLength(0);
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});

test("missing history stays unconfirmed, ACKs stay pending and historical fills cannot validate a new ticket", async ({ page }) => {
  const f = await submissionFixture(page);
  const run = f.makeRun({ idempotencyKey: "original", ticketId: "original-ticket" });
  f.setRuns([{ ...run, runId: "another-run", ticketId: "another-ticket" }]);
  const params = new URLSearchParams({ run: run.runId, ticket: run.ticketId, opp: run.opportunityId });
  await page.goto(`/#execution?${params}`);
  const flow = page.locator(".execution-flow-overview");
  await expect(flow.locator(".execution-flow-current strong")).toHaveText("历史状态待确认");
  await expect(page.locator(".execution-status-bar .execution-section-head strong")).toHaveText("原执行记录待确认");
  await expect(page.getByRole("navigation", { name: "历史执行后续操作" })).toHaveCount(0);
  await expect(page.getByRole("navigation", { name: "选择执行机会" })).toHaveCount(0);
  f.setRuns([run]);
  f.setOrders([f.makeOrder("original-long", "accepted", NOW + 10), f.makeOrder("original-short", "accepted", NOW + 10)]);
  await page.reload();
  await expect(flow.locator(".execution-flow-current strong")).toHaveText("第二腿已提交，等待成交确认");
  await flow.locator("summary").click();
  await expect(flow).toContainText("等待成交最终结果");
  await expect(page.getByRole("link", { name: "去持仓平仓", exact: true })).toHaveCount(0);
  const incomplete = { ...run, state: "hedged", updatedAtMs: NOW + 20 };
  f.emitRun(incomplete);
  await expect(flow.locator(".execution-flow-current strong")).toHaveText("等待成交确认");
  await expect(flow).toContainText("双腿成交回报未齐");
  await expect(flow).not.toContainText("双腿最终结果已确认");
  f.emitRun(f.makeRun({ idempotencyKey: "original", ticketId: "original-ticket" }, "hedged", NOW + 30));
  await expect(flow.locator(".execution-flow-current strong")).toHaveText("双腿完成");
  await expect(flow).toContainText("当前持仓待核对");
  await expect(page.getByRole("link", { name: "去持仓平仓", exact: true })).toHaveAttribute("href", `#positions?${params}`);
  await page.getByRole("button", { name: "切换到期货套利", exact: true }).click();
  await page.getByRole("button", { name: "构建新双腿", exact: true }).click();
  await expect(page.locator(".execution-artifact-status")).toContainText("待校验");
  await expect(flow).not.toHaveClass(/historical/);
  await flow.locator("summary").click();
  await expect(flow.locator("li")).toHaveCount(7);
  await expect(flow).toContainText("尚未提交");
  await expect(flow).not.toContainText("双腿最终结果已确认");
  await expect(page.locator(".confirm-action.primary")).toBeDisabled();
  await expect(page.locator(".confirm-action.primary")).toHaveText("原执行未收口");
  await expect(page.locator(".execution-actionbar .run-state > span")).toHaveText("草案待提交 · 上一笔：双腿完成");
  await expect(page.locator(".action-feedback > summary")).toHaveText("上次提交处理结果");
  await expect(page.locator(".queue-overview-copy > strong")).toHaveText("双腿完成");
  await expect(page.locator(".execution-history-context")).toContainText("历史结果");
  await page.screenshot({ path: test.info().outputPath("new-ticket-keeps-history-separate.png"), fullPage: true });
  expect(f.previews).toHaveLength(1); expect(f.builds).toHaveLength(1);
  expect(f.confirms).toHaveLength(0); expect(f.cancels).toHaveLength(0);
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});
