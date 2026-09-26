import { expect, test } from "@playwright/test";
import { reviewFixture } from "./fixtures/review-workbench";
import { NOW, setup } from "./fixtures/opportunity-workbench";

test("shared error notices keep readable text, accessible details and time to read", async ({ page }, info) => {
  const f = await reviewFixture(page);
  await page.clock.install({ time: NOW });
  const original = f.snapshot();
  const source = "fixture.history." + "isolated_segment_".repeat(50);
  const message = "历史记录读取失败，无法确认完整盈亏；已经读到的记录可以保留查看，请稍后重试。";
  const failed = (envelope: any) => ({ ...envelope, rows: [], rowCount: 0, status: "degraded",
    missingFields: ["net"], generatedAtMs: NOW + 10,
    problems: [{ code: "REVIEW_HISTORY_READ_FAILED", message, source, status: 503,
      requestId: "isolated-review-request", retryAfterMs: 5000 }] });
  f.emit({ ...original, generatedAtMs: NOW + 10,
    executed: failed(original.executed), strategyPerformance: failed(original.strategyPerformance) });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/#review");
  const toast = page.locator(".toast-item.error");
  await expect(toast).toHaveCount(1);
  // The complete reason must be readable even before opening technical details.
  expect(await toast.locator(".toast-message").evaluate(el => {
    const text = el.getBoundingClientRect();
    const card = el.closest(".toast-item")!.getBoundingClientRect();
    return text.bottom <= card.bottom && text.right <= card.right;
  })).toBe(true);
  await expect(toast.locator(".toast-message")).toHaveText(`复盘：${message}`);
  const details = toast.locator(".toast-details");
  await expect(details).not.toHaveAttribute("open", "");
  await page.screenshot({ path: info.outputPath("notice-collapsed-mobile.png") });
  await details.locator("summary").focus();
  await page.clock.runFor(6_000);
  await expect(toast).toBeVisible();
  await page.keyboard.press("Enter");
  await expect(details).toHaveAttribute("open", "");
  await expect(details.locator("pre")).toContainText(source);
  await expect(details.locator("pre")).toContainText("isolated-review-request");
  await expect(details.locator("pre")).toContainText("5000ms");
  await page.keyboard.press("Tab");
  await expect(toast.getByRole("button", { name: "关闭通知" })).toBeFocused();
  await page.getByRole("button", { name: "刷新复盘记录", exact: true }).focus();
  await page.mouse.move(0, 0);
  await page.clock.runFor(6_000);
  await expect(toast).toBeVisible();
  for (const width of [1440, 390, 320]) {
    await page.setViewportSize({ width, height: width === 320 ? 568 : 844 });
    const inlineReason = page.locator(".review-executed-table td.empty-cell");
    await expect(inlineReason).toContainText(message);
    expect(await inlineReason.evaluate(el => getComputedStyle(el).whiteSpace === "normal"
      && el.scrollWidth <= el.clientWidth + 1)).toBe(true);
    const close = toast.getByRole("button", { name: "关闭通知" });
    expect(await close.evaluate(el => {
      const r = el.getBoundingClientRect();
      return r.top >= 0 && r.bottom <= innerHeight && r.right <= innerWidth
        && el.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2));
    })).toBe(true);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    await toast.locator(".toast-content").evaluate(el => { el.scrollTop = el.scrollHeight; });
    expect(await details.locator("pre").evaluate(el => {
      const r = el.getBoundingClientRect();
      const viewport = el.closest(".toast-content")!.getBoundingClientRect();
      return r.bottom <= viewport.bottom + 1 && el.scrollWidth <= el.clientWidth + 1;
    })).toBe(true);
    await page.screenshot({ path: info.outputPath(`notice-details-${width}.png`) });
  }
  await toast.getByRole("button", { name: "关闭通知" }).click();
  await expect(toast).toHaveCount(0);
  await page.screenshot({ path: info.outputPath("review-inline-reason-320.png") });
  // Closing a notice does not clear the underlying read failure.
  await page.getByRole("button", { name: "刷新复盘记录", exact: true }).click();
  await expect(page.locator(".review-state-disclosure")).toContainText("历史读取失败");
  expect(f.writes).toEqual([]);
  expect(f.errors).toEqual([]);
});

test("shared opportunity notices remain bounded, visible on mobile and resume idle dismissal", async ({ page }, info) => {
  const f = await setup(page);
  await page.clock.install({ time: NOW });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/#futures");
  await expect.poll(() => f.channelSockets.get("alerts")?.size ?? 0).toBeGreaterThan(0);
  const emit = (symbol: string) => f.channelSockets.get("alerts")!.forEach(socket => socket.send(JSON.stringify({
    type: "message", channel: "alerts", payload: { event: "alert_triggered", notification: {
      id: `isolated-${symbol}`, ruleId: 1, watchlistId: 1, opportunityId: `fixture-${symbol}`, symbol,
      strategy: "perp_cross", longExchange: "binance", shortExchange: "okx",
      oneCycleNetBps: 5, netSingleYield: 0.1, queuedAtMs: NOW,
    } },
  })));
  const notices = page.locator(".toast-item.info");
  emit("BTC"); emit("ETH"); emit("SOL"); emit("SOL");
  await expect(notices).toHaveCount(3);
  for (const notice of await notices.all()) await expect(notice).toBeVisible();
  await expect(notices.locator(".toast-details")).toHaveCount(0);
  emit("XRP");
  await expect(notices).toHaveCount(3);
  await expect(notices.first()).toContainText("ETH");
  const reading = notices.filter({ hasText: "XRP" });
  await reading.hover();
  await page.clock.runFor(6_000);
  await expect(notices).toHaveCount(1);
  await expect(reading).toBeVisible();
  await page.screenshot({ path: info.outputPath("notice-reading-mobile.png") });
  await page.mouse.move(0, 0);
  await page.clock.runFor(4_000);
  await expect(reading).toBeVisible();
  await page.clock.runFor(1_100);
  await expect(notices).toHaveCount(0);
  // A new notification is still usable after crossing modules and old timers finishing.
  await page.getByRole("button", { name: /^切换到设置(?:，|$)/ }).click();
  emit("NEW-ALERT");
  await expect(notices).toHaveCount(1);
  await notices.getByRole("button", { name: "关闭通知" }).click();
  await page.clock.runFor(6_000);
  await expect(notices).toHaveCount(0);
  expect(f.writes).toEqual([]);
  expect(f.errors).toEqual([]);
});
