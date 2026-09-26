import { expect, test } from "@playwright/test";
import { submissionFixture, reviewAndSubmit } from "./fixtures/execution-submission";
import { NOW } from "./fixtures/opportunity-workbench";

test("order WS updates preserve row, focus, details and run identity without snapshot requests", async ({ page }) => {
  const f = await submissionFixture(page);
  await reviewAndSubmit(page);
  await expect(page.locator(".execution-actionbar")).toContainText("等待交易所成交");
  const id = `${f.confirms[0].idempotencyKey}-long`;
  f.emitOrder(id, "accepted", NOW + 30);
  const row = page.locator(`[data-order-id="${id}"]`);
  await row.locator("summary").click();
  await page.locator(".queue-run-identity summary").click();
  await row.locator("summary").focus();
  await row.evaluate((node) => { node.setAttribute("data-fixture-stable", "true"); });
  const reads = f.orderReads.length;
  for (let i = 0; i < 30; i++) f.emitRecord({ ...f.makeOrder(id, "partially_filled", NOW + 40 + i), filledQuantity: 0.000000001 });
  f.emitRun(f.makeRun(undefined, "second_leg_submitted", NOW + 100));
  await expect(row.locator(".queue-order-status")).toHaveText("部分成交");
  await expect(row).toHaveAttribute("data-fixture-stable", "true");
  await expect(row.locator("summary")).toBeFocused();
  await expect(row.locator("details")).toHaveAttribute("open", "");
  await expect(page.locator(".queue-run-identity")).toHaveAttribute("open", "");
  await expect(row.locator("dl")).toContainText("0.000000001");
  expect(f.orderReads.length).toBe(reads);
  await page.screenshot({ path: test.info().outputPath("orders-desktop.png"), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await row.scrollIntoViewIfNeeded();
  expect(await row.evaluate((node) => node.scrollWidth <= node.clientWidth + 1)).toBe(true);
  await page.screenshot({ path: test.info().outputPath("orders-mobile.png") });
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("history can reveal all loaded rows and missing fills remain unknown", async ({ page }) => {
  const f = await submissionFixture(page);
  f.setOrders(Array.from({ length: 15 }, (_, i) => f.makeOrder(`history-${i}`, "accepted", NOW + i)));
  await page.goto("/#execution");
  await expect(page.locator(".queue-order-item")).toHaveCount(12);
  await page.getByRole("button", { name: "显示更多订单" }).click();
  await expect(page.locator(".queue-order-item")).toHaveCount(15);
  await page.locator(".queue-order-detail summary").first().click();
  await expect(page.locator(".queue-order-detail dl").first()).toContainText("已成交数量待确认");
  const before = await page.locator(".queue-order-item").evaluateAll((rows) => rows.map((row) => row.getAttribute("data-order-id")));
  f.emitOrder(before.at(-1)!, "filled", NOW + 100);
  await expect(page.locator(`[data-order-id="${before.at(-1)}"] .queue-order-status`)).toHaveText("已成交");
  expect(await page.locator(".queue-order-item").evaluateAll((rows) => rows.map((row) => row.getAttribute("data-order-id")))).toEqual(before);
  expect(f.errors).toEqual([]);
});

test("history handoffs follow only their run and never invent an exposure from unrelated orders", async ({ page }) => {
  const f = await submissionFixture(page);
  const request = { idempotencyKey: "history-original", ticketId: "history-ticket" };
  const run = f.makeRun(request);
  f.setRuns([run]);
  f.setOrders([
    f.makeOrder(`${request.idempotencyKey}-long`, "accepted", NOW + 10),
    f.makeOrder(`${request.idempotencyKey}-short`, "accepted", NOW + 10),
    f.makeOrder("another-run-long", "filled", NOW + 10),
  ]);
  await page.addInitScript((run) => {
    for (const key of ["opportunityId", "ticketId", "runId"] as const)
      localStorage.setItem(`crossline.execution.runContext.${key}`, JSON.stringify(run[key]));
  }, run);
  await page.goto("/#execution");
  const next = page.getByRole("navigation", { name: "历史执行后续操作", exact: true });
  const close = next.getByRole("link", { name: "去持仓平仓", exact: true });
  const review = next.getByRole("link", { name: "关联复盘", exact: true });
  await expect(review).toBeVisible();
  await expect(close).toHaveCount(0);
  await expect(page.locator(".queue-order-item")).toHaveCount(2);
  const params = new URLSearchParams((await review.getAttribute("href"))!.split("?")[1]);
  expect(params.get("run")).toBe(run.runId);
  expect(params.get("ticket")).toBe(run.ticketId);
  expect(params.get("opp")).toBe(run.opportunityId);
  // A matching partial fill is sufficient even before the run projection catches up.
  f.emitRecord({ ...f.makeOrder(`${request.idempotencyKey}-long`, "partially_filled", NOW + 20), filledQuantity: 0.1 });
  await expect(close).toBeVisible();
  await page.getByRole("tab", { name: "最近订单 3", exact: true }).click();
  await expect(page.locator(".queue-order-item")).toHaveCount(3);
  await expect(next).toHaveCount(0);
  await expect(page.getByRole("link", { name: "全部复盘", exact: true })).toHaveAttribute("href", "#review");
  await page.getByRole("tab", { name: "上一笔 2", exact: true }).click();
  await expect(close).toBeVisible();
  f.emitRun(f.makeRun(request, "closed", NOW + 30));
  await expect(close).toHaveCount(0);
  await expect(review).toBeVisible();
  await expect(page.locator(".confirm-action.primary")).toHaveCount(0);
  await page.goto("/#execution?run=missing-original&ticket=missing-ticket&opp=fixture-perp_cross-BTC");
  await expect(page.locator(".execution-idle-history")).toContainText("指定运行尚未读取成功");
  await expect(next).toHaveCount(0);
  await expect(page.getByRole("link", { name: "全部复盘", exact: true })).toHaveCount(0);
  expect(f.previews).toHaveLength(0); expect(f.confirms).toHaveLength(0); expect(f.cancels).toHaveLength(0);
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});

test("cancel ACK waits for WS terminal and reveals any fills before run catches up", async ({ page }) => {
  const f = await submissionFixture(page);
  f.setCancelState("cancel_requested");
  await reviewAndSubmit(page);
  await page.getByRole("button", { name: "撤单", exact: true }).click();
  await expect(page.getByRole("region", { name: "原撤单核对" })).toContainText("等待订单最终结果");
  await expect(page.getByRole("button", { name: "撤单待确认", exact: true })).toBeDisabled();
  const key = f.confirms[0].idempotencyKey;
  f.emitOrder(`${key}-long`, "cancelled", NOW + 40);
  f.emitOrder(`${key}-short`, "filled", NOW + 40);
  await expect(page.locator(".remedy-state")).toContainText("1 笔已有成交");
  await expect(page.getByRole("link", { name: "去持仓平仓" })).toBeVisible();
  await page.locator(".cancel-feedback summary").click();
  await expect(page.locator(".cancel-feedback")).toContainText("撤单不等于平仓");
  expect(f.cancels).toHaveLength(2);
  expect(f.errors).toEqual([]);
});

test("partial cancel failure is replaced by later exact terminal receipts", async ({ page }) => {
  const f = await submissionFixture(page);
  f.failCancel("short");
  await reviewAndSubmit(page);
  await page.getByRole("button", { name: "撤单", exact: true }).click();
  await expect(page.getByRole("region", { name: "原撤单核对" })).toContainText("1 笔反馈待核对");
  await page.locator(".cancel-feedback summary").click();
  await expect(page.getByRole("region", { name: "原撤单核对" })).toContainText("fixture cancel outcome unknown");
  f.emitOrder(`${f.confirms[0].idempotencyKey}-short`, "failed", NOW + 40);
  await expect(page.getByRole("region", { name: "原撤单核对" })).toContainText("1 笔反馈待核对");
  f.emitOrder(`${f.confirms[0].idempotencyKey}-short`, "cancelled", NOW + 50);
  await expect(page.locator(".remedy-state")).toContainText("2 笔撤销");
  await expect(page.locator(".cancel-feedback")).toHaveAttribute("open", "");
  await expect(page.locator(".cancel-feedback")).not.toContainText("TIMEOUT");
  await expect(page.getByRole("button", { name: "撤单", exact: true })).toBeDisabled();
  f.emitRun(f.makeRun(undefined, "closed", NOW + 100));
  await expect(page.locator(".queue-overview-copy > strong")).toHaveText("执行已收口");
  await page.getByRole("button", { name: "切换到期货套利", exact: true }).click();
  await page.getByRole("button", { name: "构建新双腿", exact: true }).click();
  await expect(page.locator(".remedy-state")).toContainText("上次撤单");
  await expect(page.locator(".cancel-feedback summary")).toHaveText("上次撤单处理结果");
  expect(f.errors).toEqual([]);
});

test("late cancel response cannot overwrite earlier WS fills", async ({ page }) => {
  const f = await submissionFixture(page);
  f.holdCancel();
  await reviewAndSubmit(page);
  await page.getByRole("button", { name: "撤单", exact: true }).click();
  await expect.poll(() => f.cancels.length).toBe(1);
  const id = `${f.confirms[0].idempotencyKey}-long`;
  f.emitOrder(id, "filled", NOW + 100);
  f.releaseCancel();
  await expect(page.locator(".remedy-state")).toContainText("1 笔已有成交");
  await expect(page.locator(`[data-order-id="${id}"] .queue-order-status`)).toHaveText("已成交");
  expect(f.errors).toEqual([]);
});

test("snapshot failure after leaving cannot poison the newly mounted order queue", async ({ page }) => {
  const f = await submissionFixture(page);
  await page.goto("/#futures");
  await expect(page.getByRole("button", { name: "构建新双腿", exact: true })).toBeVisible();
  f.setOrderError(true);
  f.holdOrders();
  const reads = f.orderReads.length;
  await page.getByRole("button", { name: /^切换到对冲执行/ }).click();
  await expect.poll(() => f.orderReads.length).toBe(reads + 1);
  await page.getByRole("button", { name: "切换到期货套利", exact: true }).click();
  f.setOrderError(false);
  f.setOrders([f.makeOrder("current-order", "filled", NOW + 10)]);
  await page.getByRole("button", { name: /^切换到对冲执行/ }).click();
  await expect(page.locator(".queue-order-item")).toHaveCount(1);
  const stale = page.waitForResponse((response) => response.url().includes("/api/trading/orders") && response.status() === 503);
  f.releaseOrders();
  await (await stale).finished();
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  await expect(page.locator(".queue-problem")).toHaveCount(0);
  expect(f.errors).toEqual([]);
});
