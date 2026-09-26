import { expect, test, type Page } from "@playwright/test";
import { submissionFixture, reviewAndSubmit } from "./fixtures/execution-submission";
import { API, NOW } from "./fixtures/opportunity-workbench";

const pending = (page: Page) => page.evaluate(() => Object.entries(sessionStorage)
  .filter(([key]) => key.startsWith("crossline.execution.pendingCancel.v1:")));

test("account-rejected cancellation stays unsubmitted and can be retried only by the user", async ({ page }) => {
  const f = await submissionFixture(page);
  let wrongAccount: "all" | "short" | "none" = "all";
  const requests: string[] = [];
  await page.route(`${API}/api/trading/orders/*/cancel`, route => {
    requests.push(route.request().url());
    if (wrongAccount === "none" || (wrongAccount === "short" && route.request().url().includes("-long/cancel"))) return route.fallback();
    return route.fulfill({ status: 409, json: { error: {
      code: "ORDER_ACCOUNT_MISMATCH",
      message: "订单账户与当前连接不匹配；未向当前账户发送操作",
    } } });
  });
  await reviewAndSubmit(page);
  await page.getByRole("button", { name: "撤单", exact: true }).click();
  await expect(page.locator(".execution-page")).toContainText("撤单未发送：账户不匹配");
  await expect(page.locator(".execution-page")).toContainText("恢复原账户后核对挂单");
  expect(requests).toHaveLength(1); expect(f.cancels).toHaveLength(0);
  expect(await pending(page)).toHaveLength(0);
  await expect(page.getByRole("button", { name: "核对原撤单", exact: true })).toHaveCount(0);
  await page.screenshot({ path: test.info().outputPath("account-cancel-desktop.png"), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  const cancel = page.getByRole("button", { name: "撤单", exact: true });
  await cancel.scrollIntoViewIfNeeded();
  await expect(cancel).toBeEnabled();
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(390);
  await page.screenshot({ path: test.info().outputPath("account-cancel-mobile.png") });
  // The next attempt reaches the first venue but the second rejects its account binding.
  wrongAccount = "short";
  f.setCancelState("cancel_requested");
  expect(requests).toHaveLength(1);
  await cancel.click();
  await expect.poll(() => requests.length).toBe(3);
  await expect.poll(() => pending(page)).toHaveLength(1);
  const saved = JSON.parse((await pending(page))[0][1]);
  expect(saved.requests.map((request: any) => request.account_rejected)).toEqual([false, true]);
  // ACK cannot clear the recovery lock. A terminal first leg with a fill must retain its warning.
  const long = `${f.confirms[0].idempotencyKey}-long`;
  f.emitRecord({ ...f.makeOrder(long, "cancelled", NOW + 50), filledQuantity: 0.4 });
  await expect(page.locator(".execution-page")).toContainText("撤单部分完成：账户不匹配");
  await expect(page.locator(".execution-page")).toContainText("1 笔已有成交");
  await expect(page.locator(".execution-page")).toContainText("撤单不等于平仓");
  const instruction = page.getByRole("status", { name: "撤单处理提示" });
  await expect(instruction).toBeVisible();
  await expect(instruction).toContainText("已有成交；撤单不等于平仓");
  expect(await instruction.evaluate(el => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
  await expect.poll(() => pending(page)).toHaveLength(0);
  expect(f.cancels).toHaveLength(1);
  await page.screenshot({ path: test.info().outputPath("account-cancel-partial-mobile.png") });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.screenshot({ path: test.info().outputPath("account-cancel-partial-desktop.png"), fullPage: true });
  // Restoring the account requires another explicit click and only retries the unresolved leg.
  wrongAccount = "none";
  f.setCancelState("cancelled");
  expect(requests).toHaveLength(3);
  await cancel.click();
  await expect.poll(() => f.cancels.length).toBe(2);
  expect(f.cancels).toEqual([long, `${f.confirms[0].idempotencyKey}-short`]);
  await expect.poll(() => pending(page)).toHaveLength(0);
  expect(requests).toHaveLength(4); expect(f.confirms).toHaveLength(1);
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});

test("cancel batch survives lost feedback and reload, resolves only exact terminal quantities", async ({ page }) => {
  const f = await submissionFixture(page);
  f.failCancel("short");
  let response: any;
  const queries: string[] = [];
  await page.route(`${API}/api/trading/orders/*`, route => {
    if (route.request().method() !== "GET") return route.fallback();
    const id = decodeURIComponent(new URL(route.request().url()).pathname.split("/").at(-1)!);
    queries.push(id);
    return response
      ? route.fulfill({ json: id.endsWith("long") ? f.makeOrder(id, "cancelled", NOW + 30) : response })
      : route.fulfill({ status: 503, json: { error: { code: "ORDER_READ_FAILED", message: "fixture original order unavailable" } } });
  });
  await reviewAndSubmit(page);
  await page.getByRole("button", { name: "撤单", exact: true }).click();
  const panel = page.getByRole("region", { name: "原撤单核对" });
  await expect(panel).toContainText("fixture cancel outcome unknown");
  expect(f.cancels).toHaveLength(2);
  const saved = await pending(page);
  expect(saved).toHaveLength(1);
  const batch = JSON.parse(saved[0][1]);
  expect(batch.requests.map((r: any) => r.sent)).toEqual([true, true]);
  expect(JSON.stringify(saved)).not.toContain("isolated-fixture-token");
  await expect(page.getByRole("button", { name: "撤单待确认", exact: true })).toBeDisabled();
  await expect(page.getByRole("button", { name: "重置状态", exact: true })).toHaveCount(0);
  await page.getByRole("button", { name: "切换到期货套利", exact: true }).click();
  await page.getByRole("button", { name: "构建新双腿", exact: true }).click();
  await expect(page.locator(".execution-history-context").first()).toContainText("原提交或撤单尚在核对");
  expect(f.previews).toHaveLength(1);

  f.setOrders([]);
  await page.reload();
  await expect(panel).toContainText("原撤单结果待核对");
  const query = panel.getByRole("button", { name: "核对原撤单", exact: true });
  await query.click();
  await expect(panel).toContainText("fixture original order unavailable");
  expect(f.cancels).toHaveLength(2);
  const short = `${f.confirms[0].idempotencyKey}-short`;
  response = f.makeOrder(short, "cancelled", NOW + 40);
  response.intent.exchange = "wrong-venue";
  await query.click();
  await expect(panel).toContainText("订单身份不匹配");
  expect(await pending(page)).toHaveLength(1);
  response = f.makeOrder(short, "cancel_requested", NOW + 40);
  await query.click();
  await expect(panel).toContainText("尚未确认最终结果或成交量");
  response = { ...f.makeOrder(short, "cancelled", NOW + 50), filledQuantity: null };
  await query.click();
  await expect(panel).toContainText("尚未确认最终结果或成交量");
  expect(await pending(page)).toHaveLength(1);
  await page.screenshot({ path: test.info().outputPath("cancel-recovery-desktop.png"), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await query.scrollIntoViewIfNeeded();
  await expect(query).toBeInViewport();
  expect(await query.evaluate(el => {
    const r = el.getBoundingClientRect();
    return el.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2));
  })).toBe(true);
  expect(await panel.evaluate(el => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
  await page.screenshot({ path: test.info().outputPath("cancel-recovery-mobile.png") });

  response = { ...f.makeOrder(short, "cancelled", NOW + 60), filledQuantity: 0.4 };
  await query.click();
  await expect(panel).toContainText("1 笔已有成交");
  await expect(panel).toContainText("撤单不等于平仓");
  await expect(query).toHaveCount(0);
  expect(await pending(page)).toHaveLength(0);
  await expect(page.getByRole("link", { name: "去持仓平仓", exact: true })).toBeVisible();
  expect(queries.every(id => batch.requests.some((r: any) => r.order_id === id))).toBe(true);
  expect(f.cancels).toHaveLength(2); expect(f.confirms).toHaveLength(1);
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});

test("cancel storage denial sends nothing and interrupted queued leg is never auto-resumed", async ({ page }) => {
  const f = await submissionFixture(page);
  f.holdCancel();
  await page.addInitScript(() => {
    const original = Storage.prototype.setItem;
    Storage.prototype.setItem = function(key: string, value: string) {
      if (key.startsWith("crossline.execution.pendingCancel.v1:") && !(window as any).allowCancelStorage)
        throw new DOMException("fixture no storage");
      return original.call(this, key, value);
    };
  });
  await reviewAndSubmit(page);
  await page.getByRole("button", { name: "撤单", exact: true }).click();
  const panel = page.getByRole("region", { name: "原撤单核对" });
  await expect(panel).toContainText("无法保存撤单恢复记录");
  expect(f.cancels).toHaveLength(0);
  await page.evaluate(() => { (window as any).allowCancelStorage = true; });
  await panel.getByRole("button", { name: "核对原撤单", exact: true }).click();
  await expect(panel).toHaveCount(0);
  await page.getByRole("button", { name: "撤单", exact: true }).click();
  await expect.poll(() => f.cancels.length).toBe(1);
  const saved = await pending(page);
  const batch = JSON.parse(saved[0][1]);
  expect(batch.requests.map((r: any) => r.sent)).toEqual([true, false]);
  // The browser disappears while leg one is in flight. Backend finishes it only once.
  const long = batch.requests[0].order_id;
  f.setOrders([f.makeOrder(long, "cancelled", NOW + 50)]);
  await page.addInitScript(() => { (window as any).allowCancelStorage = true; });
  await page.reload();
  f.releaseCancel();
  await expect(panel).toContainText("1 笔未发送");
  expect(await pending(page)).toHaveLength(0);
  expect(f.cancels).toEqual([long]);
  expect(f.confirms).toHaveLength(1);
  // A different login must not claim the saved original batch even on the same backend.
  await page.evaluate(([key, value]) => sessionStorage.setItem(key, value), saved[0]);
  await page.addInitScript(() => localStorage.setItem("api_auth_token", JSON.stringify("different-fixture-login")));
  await page.reload();
  await expect(page.locator(".execution-idle")).toBeVisible();
  await expect(panel).toHaveCount(0);
  expect(await pending(page)).toEqual(saved);
  await page.addInitScript(() => localStorage.setItem("api_auth_token", JSON.stringify("isolated-fixture-token")));
  await page.reload();
  await expect(panel).toContainText("1 笔未发送");
  expect(await pending(page)).toHaveLength(0);
  expect(f.cancels).toEqual([long]);
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});
