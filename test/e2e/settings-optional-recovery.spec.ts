import { expect, test } from "@playwright/test";
import { settingsEvidenceFixture } from "./fixtures/settings-evidence";
import { API, NOW } from "./fixtures/opportunity-workbench";

test("optional fallback keeps each stream independent, rejects late missing routes and aborts on leave", async ({ page }, info) => {
  const f = await settingsEvidenceFixture(page, "diagnostics");
  await page.clock.install({ time: NOW });
  const watchlist = await (await page.request.get(API + "/e2e-watchlist-alerts-runtime/api/watchlist")).json();
  const alerts = await (await page.request.get(API + "/e2e-watchlist-alerts-runtime/api/alerts/rules")).json();
  watchlist.items[0].symbol = "OPTIONAL-CACHED-WATCH";
  let reads = 0, hold = false, missing = false, aborted = 0;
  const pending: (() => void)[] = [];
  page.on("requestfailed", request => { if (new URL(request.url()).pathname === "/api/watchlist") aborted++; });
  await page.route(API + "/api/watchlist", async route => {
    reads++;
    const response = missing ? { status: 404, json: { code: "NOT_ENABLED", message: "old missing route" } }
      : { json: structuredClone(watchlist) };
    if (hold) await new Promise<void>(resolve => pending.push(resolve));
    await route.fulfill(response);
  });
  await page.route(API + "/api/alerts/rules", route => route.fulfill({ json: alerts }));
  await page.goto("/#settings");
  await page.getByRole("tab", { name: "运行数据依据", exact: true }).click();
  const panel = page.getByRole("tabpanel", { name: "运行数据依据诊断", exact: true });
  await expect(panel).toContainText("OPTIONAL-CACHED-WATCH");
  await expect.poll(() => f.channelSockets.get("watchlist")?.size ?? 0).toBe(1);
  const emit = (channel: "watchlist" | "alerts") => {
    for (const socket of f.channelSockets.get(channel) ?? []) socket.send(JSON.stringify({
      type: "message", channel, payload: { event: channel === "watchlist" ? "watchlist_changed" : "alert_rules_changed",
        envelope: channel === "watchlist" ? watchlist : alerts, timestampMs: NOW + reads },
    }));
  };
  emit("watchlist"); emit("alerts");
  hold = true;
  await page.clock.runFor(15_100);
  await expect.poll(() => reads).toBe(2);
  await page.clock.runFor(15_100);
  await expect(panel).toContainText("SHARED_READ_TIMEOUT");
  await expect(panel).toContainText("OPTIONAL-CACHED-WATCH");
  await expect.poll(() => aborted).toBe(1);
  emit("alerts");
  await expect(panel).toContainText("SHARED_READ_TIMEOUT");
  pending.shift()!();
  missing = true;
  await page.clock.runFor(5000);
  await expect.poll(() => reads).toBe(3);
  watchlist.items[0].symbol = "OPTIONAL-NEW-WS";
  emit("watchlist");
  await expect(panel).toContainText("OPTIONAL-NEW-WS");
  await expect(panel).not.toContainText("SHARED_READ_TIMEOUT");
  const late = page.waitForResponse(r => r.url().endsWith("/api/watchlist") && r.status() === 404);
  pending.shift()!(); await (await late).finished();
  await expect(panel).toContainText("OPTIONAL-NEW-WS");
  await expect(panel).not.toContainText("当前未启用");
  missing = false;
  await page.clock.runFor(15_100);
  await expect.poll(() => reads).toBe(4);
  await page.locator('[data-module="review"]').click();
  await expect.poll(() => aborted).toBe(2);
  pending.shift()!();
  hold = false;
  await page.locator('[data-module="settings"]').click();
  await page.getByRole("tab", { name: "运行数据依据", exact: true }).click();
  await expect(panel).toContainText("OPTIONAL-NEW-WS");
  await page.setViewportSize({ width: 390, height: 844 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
  await page.screenshot({ path: info.outputPath("optional-fallback-mobile.png") });
  expect(f.calls.every(r => r.key.startsWith("GET "))).toBe(true);
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});

test("optional reminders stop waiting, explain failure and retry into live data without writes", async ({ page }, info) => {
  const f = await settingsEvidenceFixture(page, "diagnostics");
  await page.clock.install({ time: NOW });
  const watchlist = await (await page.request.get(API + "/e2e-watchlist-alerts-runtime/api/watchlist")).json();
  const alerts = await (await page.request.get(API + "/e2e-watchlist-alerts-runtime/api/alerts/rules")).json();
  watchlist.items[0].symbol = "RECOVERED-WATCH";
  let hold = true, reads = 0, aborted = 0, release: (() => void) | undefined;
  page.on("requestfailed", request => { if (new URL(request.url()).pathname === "/api/watchlist") aborted++; });
  await page.route("**/api/watchlist", async route => {
    reads++;
    if (hold) await new Promise<void>(resolve => { release = resolve; });
    await route.fulfill({ json: watchlist });
  });
  await page.route("**/api/alerts/rules", route => route.fulfill({ json: alerts }));
  await page.goto("/#settings");
  await page.getByRole("tab", { name: "运行数据依据", exact: true }).click();
  await expect.poll(() => reads).toBe(1);
  await expect(page.getByText("正在检测自选与提醒功能", { exact: true })).toBeVisible();
  await page.clock.runFor(15_100);
  const failure = page.getByRole("alert", { name: "自选与提醒读取失败", exact: true });
  await expect(failure).toContainText("15 秒");
  await expect(page.locator('[data-module="settings"]')).toHaveAttribute("data-runtime-state", "error");
  await expect.poll(() => aborted).toBe(1);
  expect(reads).toBe(1);
  const retry = page.getByRole("button", { name: "重新检测自选与提醒", exact: true });
  await retry.scrollIntoViewIfNeeded();
  await expect(retry).toBeInViewport();
  await page.screenshot({ path: info.outputPath("optional-read-timeout.png") });
  await page.setViewportSize({ width: 390, height: 844 });
  await retry.scrollIntoViewIfNeeded();
  await retry.click({ trial: true });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
  await page.screenshot({ path: info.outputPath("optional-read-timeout-mobile.png") });
  await page.setViewportSize({ width: 1280, height: 720 });
  hold = false; release!();
  await retry.click();
  await expect(failure).toHaveCount(0);
  const panel = page.getByRole("tabpanel", { name: "运行数据依据诊断", exact: true });
  await expect(panel).toContainText("RECOVERED-WATCH");
  await expect.poll(() => f.channelSockets.get("watchlist")?.size ?? 0).toBe(1);
  watchlist.items[0].symbol = "FRESH-WS-WATCH";
  for (const socket of f.channelSockets.get("watchlist")!) socket.send(JSON.stringify({
    type: "message", channel: "watchlist", payload: { event: "watchlist_changed", envelope: watchlist, timestampMs: NOW + 15101 },
  }));
  await expect(panel).toContainText("FRESH-WS-WATCH");
  expect(reads).toBe(2);
  await page.setViewportSize({ width: 390, height: 844 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
  await page.screenshot({ path: info.outputPath("optional-recovered-mobile.png") });
  expect(f.calls.every(r => r.key.startsWith("GET "))).toBe(true);
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});
