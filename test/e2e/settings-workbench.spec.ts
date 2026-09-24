import { expect, test } from "@playwright/test";
import { settingsFixture } from "./fixtures/settings-workbench";
import { NOW } from "./fixtures/opportunity-workbench";

test("settings workspace renders its seven task areas", async ({ page }) => {
  const f = await settingsFixture(page);
  await page.goto("/#settings");
  for (const name of ["执行环境", "行情", "凭证", "风控", "动作账本", "诊断", "Webhook"]) {
    await page.getByRole("tab", { name, exact: true }).click();
    await expect(page.locator(".settings-content-panel")).toBeVisible();
    if (name === "行情") await expect(page.getByRole("checkbox", { name: "kraken 现货" })).toBeVisible();
    if (name === "Webhook") await expect(page.locator(".webhook-summary")).toContainText("Bark");
    await page.screenshot({ path: test.info().outputPath(`${name}-desktop.png`), fullPage: true });
  }
  await page.setViewportSize({ width: 390, height: 844 });
  const webhookTab = await page.getByRole("tab", { name: "Webhook", exact: true }).boundingBox();
  expect(webhookTab!.x + webhookTab!.width).toBeLessThanOrEqual(390);
  const refresh = await page.getByRole("button", { name: "刷新 Webhook 状态" }).boundingBox();
  const toggle = await page.getByRole("button", { name: "停用", exact: true }).boundingBox();
  expect(Math.abs((refresh!.y + refresh!.height / 2) - (toggle!.y + toggle!.height / 2))).toBeLessThan(2);
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(390);
  await page.screenshot({ path: test.info().outputPath("webhook-mobile.png"), fullPage: true });
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

const statusPath = "GET /api/webhook/status";
const savePath = "PATCH /api/webhook/config";
const marketRead = "GET /api/system/market-subscriptions";
const marketSave = "PATCH /api/system/market-subscriptions/config";
const saveButton = (page: import("@playwright/test").Page) => page.getByRole("button", { name: "保存配置", exact: true });

test("fresh webhook WS suppresses periodic reads and silent WS recovers a snapshot", async ({ page }) => {
  const f = await settingsFixture(page);
  await page.clock.install({ time: NOW });
  await page.goto("/#settings");
  await expect(saveButton(page)).toBeEnabled();
  await expect.poll(() => f.channelSockets.has("webhook")).toBe(true);
  f.emit();
  const reads = () => f.requests.filter((r) => r.method === "GET" && r.path === "/api/webhook/status").length;
  // Advance both the freshness clock and timers without a 30s wall-clock wait.
  await page.clock.runFor(5500);
  expect(reads()).toBe(1);
  f.fail(statusPath);
  const fallback = page.waitForResponse((r) => r.url().endsWith("/webhook/status") && r.status() === 503);
  await page.clock.runFor(31_000);
  await (await fallback).finished();
  await expect(page.getByRole("alert")).toContainText("SETTINGS_FIXTURE_UNAVAILABLE");
  await expect(saveButton(page)).toBeDisabled();
  expect(reads()).toBe(2);
  f.fail(statusPath, false);
  f.webhook.updatedAtMs = NOW + 40_001;
  f.emit();
  await expect(saveButton(page)).toBeEnabled();
  await expect(page.getByRole("alert")).toHaveCount(0);
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("webhook save failure retains input and in-flight save locks duplicate actions", async ({ page }) => {
  const f = await settingsFixture(page);
  await page.goto("/#settings");
  await expect(saveButton(page)).toBeEnabled();
  await page.getByLabel("投递提供方").selectOption("generic");
  const url = page.getByLabel("公网 HTTPS URL"), secret = page.locator('.webhook-core-grid input[type="password"]');
  await url.fill("https://example.com/hook");
  await secret.fill("fixture-secret");
  f.fail(savePath);
  await saveButton(page).click();
  await expect(page.getByRole("alert")).toContainText("SETTINGS_FIXTURE_UNAVAILABLE");
  await expect(url).toHaveValue("https://example.com/hook");
  await expect(secret).toHaveValue("fixture-secret");
  f.fail(savePath, false);
  f.hold(savePath);
  await saveButton(page).click();
  await expect(page.getByRole("button", { name: "处理中" })).toBeDisabled();
  await expect(page.getByRole("button", { name: "发送测试" })).toBeDisabled();
  await expect(url).toBeDisabled();
  f.release(savePath);
  await expect(page.locator(".webhook-settings").getByRole("status")).toContainText("配置已保存");
  await expect(url).toHaveValue("");
  await expect(secret).toHaveValue("");
  expect(f.requests.filter((r) => r.method === "PATCH")).toHaveLength(2);
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("invalid webhook numbers do not silently become unchanged settings", async ({ page }) => {
  const f = await settingsFixture(page);
  await page.goto("/#settings");
  await expect(saveButton(page)).toBeEnabled();
  await page.locator(".webhook-advanced-settings summary").click();
  await page.getByLabel("最大尝试", { exact: true }).fill("1.5");
  await saveButton(page).click();
  await expect(page.getByRole("alert")).toContainText("整数");
  expect(f.requests.filter((r) => r.method === "PATCH")).toHaveLength(0);
  expect(f.errors).toEqual([]);
});

test("webhook WS preserves drafts and history while old HTTP cannot undo a save", async ({ page }) => {
  const f = await settingsFixture(page);
  await page.goto("/#settings");
  await expect(saveButton(page)).toBeEnabled();
  await expect.poll(() => f.channelSockets.has("webhook")).toBe(true);
  await page.locator(".webhook-advanced-settings summary").click();
  await page.locator(".webhook-delivery-history summary").click();
  await page.getByLabel("超时 ms").fill("22000");
  await page.getByLabel("超时 ms").focus();
  f.webhook.deliveredTotal = 2;
  for (let i = 0; i < 20; i++) f.emit();
  await expect(page.locator(".webhook-summary")).toContainText("成功 2");
  await expect(page.locator(".webhook-delivery-history")).toHaveAttribute("open", "");
  await expect(page.getByLabel("超时 ms")).toHaveValue("22000");
  await expect(page.getByLabel("超时 ms")).toBeFocused();
  f.fail(statusPath);
  f.hold(statusPath);
  await page.getByRole("button", { name: "刷新 Webhook 状态" }).click();
  await expect.poll(() => f.requests.filter((r) => r.method === "GET" && r.path === "/api/webhook/status").length).toBe(2);
  await saveButton(page).click();
  await expect(page.locator(".webhook-settings").getByRole("status")).toContainText("配置已保存");
  const late = page.waitForResponse((r) => r.url().endsWith("/webhook/status") && r.status() === 503);
  f.release(statusPath);
  await (await late).finished();
  await expect(saveButton(page)).toBeEnabled();
  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect(page.getByLabel("超时 ms")).toHaveValue("22000");
  expect(f.errors).toEqual([]);
});

test("webhook read failure is recoverable and test acceptance is not delivery", async ({ page }) => {
  const f = await settingsFixture(page);
  f.fail(statusPath);
  await page.goto("/#settings");
  await expect(page.getByRole("alert")).toContainText("SETTINGS_FIXTURE_UNAVAILABLE");
  await expect(saveButton(page)).toBeDisabled();
  f.fail(statusPath, false);
  await page.getByRole("button", { name: "刷新 Webhook 状态" }).click();
  await expect(saveButton(page)).toBeEnabled();
  const testPath = "POST /api/webhook/test";
  f.hold(testPath);
  await page.getByRole("button", { name: "发送测试" }).click();
  await expect(page.getByRole("button", { name: "发送测试" })).toBeDisabled();
  f.release(testPath);
  await expect(page.locator(".webhook-settings").getByRole("status")).toContainText("实际送达以投递回执为准");
  await page.locator(".webhook-danger-zone summary").click();
  const clear = page.getByRole("button", { name: "停用并清除", exact: true });
  await expect(clear).toBeDisabled();
  await page.getByRole("checkbox", { name: "确认清除投递地址与密钥" }).check();
  await clear.click();
  await expect(page.locator(".webhook-summary")).toContainText("未配置投递地址");
  await expect(page.locator(".webhook-summary")).not.toContainText("已隐藏");
  expect(f.requests.findLast((r) => r.method === "PATCH")!.body).toMatchObject({ enabled: false, url: "", clearSecret: true });
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("market toggles retain saved state on failure and accept the mutation receipt", async ({ page }) => {
  const f = await settingsFixture(page, "market-data");
  await page.goto("/#settings");
  const spot = page.getByRole("checkbox", { name: "kraken 现货", exact: true });
  await expect(spot).toBeChecked();
  f.fail(marketSave);
  await spot.click();
  await expect(page.locator(".settings-market-subscriptions").getByRole("status")).toContainText("更新未确认");
  await expect(spot).toBeChecked();
  await expect(spot).toBeEnabled();
  f.fail(marketSave, false);
  f.fail(marketRead);
  f.hold(marketRead);
  const reads = f.requests.filter((r) => r.path.endsWith("/market-subscriptions")).length;
  await page.getByRole("button", { name: "刷新行情订阅" }).click();
  await expect.poll(() => f.requests.filter((r) => r.path.endsWith("/market-subscriptions")).length).toBe(reads + 1);
  f.hold(marketSave);
  await spot.click();
  await expect(spot).toBeDisabled();
  await expect(spot).toBeChecked();
  f.release(marketSave);
  await expect(spot).not.toBeChecked();
  const late = page.waitForResponse((r) => r.url().endsWith("/market-subscriptions") && r.status() === 503);
  f.release(marketRead);
  await (await late).finished();
  await expect(spot).not.toBeChecked();
  await expect(page.getByRole("alert")).toHaveCount(0);
  await page.setViewportSize({ width: 390, height: 844 });
  for (const label of ["kraken 现货", "kraken 永续", "kraken Funding"]) {
    const box = await page.getByRole("checkbox", { name: label, exact: true }).boundingBox();
    expect(box!.x + box!.width).toBeLessThanOrEqual(390);
  }
  await page.screenshot({ path: test.info().outputPath("market-mobile.png"), fullPage: true });
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

for (const tab of ["webhook", "market-data"]) test(`${tab} late mutation survives leaving without touching disposed UI`, async ({ page }) => {
  const f = await settingsFixture(page, tab);
  const path = tab === "webhook" ? savePath : marketSave;
  f.hold(path);
  await page.goto("/#settings");
  if (tab === "webhook") {
    await expect(saveButton(page)).toBeEnabled();
    await saveButton(page).click();
  } else await page.getByRole("checkbox", { name: "kraken 现货", exact: true }).click();
  await expect.poll(() => f.requests.filter((r) => r.method === "PATCH").length).toBe(1);
  await page.evaluate(() => { location.hash = "review"; });
  await expect(page.locator(".settings-workspace")).toHaveCount(0);
  const response = page.waitForResponse((r) => r.request().method() === "PATCH");
  f.release(path);
  await (await response).finished();
  await page.evaluate(() => { location.hash = "settings"; });
  if (tab === "webhook") await expect(saveButton(page)).toBeEnabled();
  else await expect(page.getByRole("checkbox", { name: "kraken 现货", exact: true })).not.toBeChecked();
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});
