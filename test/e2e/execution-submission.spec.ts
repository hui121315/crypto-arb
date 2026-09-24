import { expect, test } from "@playwright/test";
import { submissionFixture, reviewAndSubmit } from "./fixtures/execution-submission";
import { API, NOW } from "./fixtures/opportunity-workbench";

test("lost submit response survives reload and new selection until exact receipt arrives", async ({ page }) => {
  const f = await submissionFixture(page);
  f.setMode("timeout");
  await reviewAndSubmit(page);
  await expect(page.locator(".execution-actionbar")).toContainText("提交结果待核验");
  await expect(page.getByRole("button", { name: "重置状态", exact: true })).toHaveCount(0);
  await page.getByRole("button", { name: "查询提交结果", exact: true }).click();
  await expect(page.locator(".confirm-action.primary")).toBeDisabled();
  await page.reload();
  await expect(page.locator(".execution-actionbar")).toContainText("原提交结果待核验");
  await page.setViewportSize({ width: 390, height: 844 });
  const query = page.getByRole("button", { name: "查询提交结果", exact: true });
  await query.scrollIntoViewIfNeeded();
  await expect(query).toBeInViewport();
  expect(await query.evaluate((node) => {
    const r = node.getBoundingClientRect();
    return node.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2));
  })).toBe(true);
  await page.screenshot({ path: test.info().outputPath("execution-recovery-mobile.png"), fullPage: true });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.getByRole("button", { name: "切换到期货套利", exact: true }).click();
  await page.getByRole("button", { name: "构建新双腿", exact: true }).click();
  await expect(page.locator(".confirm-action.primary")).toBeDisabled();
  f.setRuns([{ ...f.makeRun(), ticketId: "another-ticket" }]);
  await page.getByRole("button", { name: "查询提交结果", exact: true }).click();
  await expect(page.locator(".execution-actionbar")).toContainText("原提交尚在核验");
  f.emitRun(f.makeRun(undefined, "hedged", NOW + 30));
  await expect(page.locator(".execution-actionbar")).toContainText("双腿成交已确认");
  await expect(page.getByRole("button", { name: "查询提交结果", exact: true })).toHaveCount(0);
  expect(await page.evaluate((key) => localStorage.getItem(key), `crossline.execution.pendingConfirm:${API}`)).toBeNull();
  expect(f.confirms).toHaveLength(1);
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("WS fill wins over same-millisecond late confirm ACK and later old HTTP", async ({ page }) => {
  const f = await submissionFixture(page);
  f.holdConfirm();
  await reviewAndSubmit(page);
  await expect.poll(() => f.confirms.length).toBe(1);
  f.emitRun(f.makeRun(undefined, "hedged"));
  await expect(page.locator(".execution-actionbar")).toContainText("双腿成交已确认");
  const response = page.waitForResponse("**/confirm");
  f.releaseConfirm();
  await (await response).finished();
  await expect(page.locator(".execution-actionbar")).toContainText("双腿成交已确认");
  f.setRuns([f.makeRun()]);
  f.holdRead();
  const reads = f.reads.length;
  await page.getByRole("button", { name: "切换到复盘", exact: true }).click();
  await page.getByRole("button", { name: /^切换到对冲执行/ }).click();
  await expect.poll(() => f.reads.length).toBeGreaterThan(reads);
  f.emitRun(f.makeRun(undefined, "hedged", NOW + 40));
  const oldRead = page.waitForResponse("**/execution-runs?**");
  f.releaseRead();
  await (await oldRead).finished();
  await expect(page.locator(".execution-actionbar")).toContainText("双腿成交已确认");
  expect(f.confirms).toHaveLength(1);
  expect(f.errors).toEqual([]);
});

test("ACK remains awaiting finality and only real fills hand off to positions", async ({ page }) => {
  const f = await submissionFixture(page);
  await reviewAndSubmit(page);
  await expect(page.locator(".execution-actionbar")).toContainText("等待交易所成交");
  await expect(page.getByRole("link", { name: "去持仓平仓", exact: true })).toHaveCount(0);
  f.emitRun(f.makeRun(undefined, "hedged", NOW + 30));
  await expect(page.getByRole("link", { name: "去持仓平仓", exact: true })).toBeVisible();
  await page.getByRole("link", { name: "去持仓平仓", exact: true }).click();
  await expect(page).toHaveURL(/#positions\?run=/);
  expect(new URLSearchParams(new URL(page.url()).hash.split("?")[1]).get("ticket")).toBe(f.confirms[0].ticketId);
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("pre-order rejection releases pending identity but does not retry on its own", async ({ page }) => {
  const f = await submissionFixture(page);
  f.setMode("reject");
  await reviewAndSubmit(page);
  await expect(page.locator(".execution-actionbar")).toContainText("fixture rejected before order");
  await expect(page.getByRole("button", { name: "刷新预览", exact: true })).toBeEnabled();
  expect(await page.evaluate((key) => localStorage.getItem(key), `crossline.execution.pendingConfirm:${API}`)).toBeNull();
  await page.getByRole("button", { name: "刷新预览", exact: true }).click();
  await expect.poll(() => f.builds.length).toBeGreaterThan(1);
  expect(f.confirms).toHaveLength(1);
  expect(f.errors).toEqual([]);
});

test("lost rejection is recovered only from the exact pre-order action journal entry", async ({ page }) => {
  const f = await submissionFixture(page);
  f.setMode("timeout");
  await reviewAndSubmit(page);
  await expect(page.locator(".execution-actionbar")).toContainText("提交结果待核验");
  const request = f.confirms[0];
  const rejection = { id: "fixture-action", kind: "hedge_confirm", status: "failed", actor: "fixture",
    target: "fixture-perp_cross-BTC", idempotencyKey: request.idempotencyKey,
    message: "fixture journal rejection",
    startedAtMs: NOW, updatedAtMs: NOW + 10,
    problem: { code: "HEDGE_PRE_TRADE_REJECTED", message: "fixture journal rejection" } };
  f.setActions([{ ...rejection, idempotencyKey: "another-request" }]);
  const read = page.waitForResponse("**/api/trading/action-runs");
  await page.getByRole("button", { name: "查询提交结果", exact: true }).click();
  await (await read).finished();
  await expect(page.locator(".confirm-action.primary")).toBeDisabled();
  f.setActions([rejection]);
  await page.getByRole("button", { name: "查询提交结果", exact: true }).click();
  await expect(page.locator(".execution-actionbar")).toContainText("已核实：下单前被拒绝");
  expect(await page.evaluate((key) => localStorage.getItem(key), `crossline.execution.pendingConfirm:${API}`)).toBeNull();
  expect(f.confirms).toHaveLength(1);
  expect(f.errors).toEqual([]);
});

test("browser storage failure blocks the write before any confirm request", async ({ page }) => {
  const f = await submissionFixture(page);
  await page.addInitScript(() => {
    const setItem = Storage.prototype.setItem;
    Storage.prototype.setItem = function(key, value) {
      if (key.startsWith("crossline.execution.pendingConfirm:")) throw new DOMException("fixture quota", "QuotaExceededError");
      return setItem.call(this, key, value);
    };
  });
  await reviewAndSubmit(page);
  await expect(page.locator(".execution-actionbar")).toContainText("无法保存原提交记录，未发送新请求");
  await expect(page.locator(".confirm-action.primary")).toBeDisabled();
  expect(f.confirms).toHaveLength(0);
  expect(f.errors).toEqual([]);
});

test("leaving execution during submission keeps recovery and accepts original response safely", async ({ page }) => {
  const f = await submissionFixture(page);
  f.holdConfirm();
  await reviewAndSubmit(page);
  await expect.poll(() => f.confirms.length).toBe(1);
  await page.getByRole("button", { name: "切换到复盘", exact: true }).click();
  const response = page.waitForResponse("**/confirm");
  f.releaseConfirm();
  await (await response).finished();
  await page.getByRole("button", { name: /^切换到对冲执行/ }).click();
  await expect(page.locator(".execution-actionbar")).toContainText("等待交易所成交");
  expect(f.errors).toEqual([]);
  expect(f.confirms).toHaveLength(1);
});

test("partial cancel retains successful receipt and late callback survives module switch", async ({ page }) => {
  const f = await submissionFixture(page);
  f.holdCancel();
  f.failCancel("short");
  await reviewAndSubmit(page);
  await page.getByRole("button", { name: "撤单", exact: true }).click();
  await expect.poll(() => f.cancels.length).toBe(1);
  await page.getByRole("button", { name: "切换到复盘", exact: true }).click();
  f.releaseCancel();
  await expect.poll(() => f.cancels.length).toBe(2);
  await page.getByRole("button", { name: /^切换到对冲执行/ }).click();
  await expect(page.locator(".remedy-state")).toContainText("1 笔收到回执，1 笔失败或待核验");
  await expect(page.locator(".execution-order-queue")).toContainText("fixture cancelled");
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
  await page.screenshot({ path: test.info().outputPath("execution-cancel-partial.png"), fullPage: true });
});
