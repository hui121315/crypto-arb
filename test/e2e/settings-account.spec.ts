import { expect, test } from "@playwright/test";
import { settingsAccountFixture } from "./fixtures/settings-account";
import { NOW } from "./fixtures/opportunity-workbench";

const savePath = "POST /api/exchanges/credentials";
const adapterSave = "POST /api/trading/adapters/select";
const credentialRead = "GET /api/exchanges/credentials";
const keyInput = (page: import("@playwright/test").Page) => page.locator('.credential-fields input[name="api_key"]');

test("credentials preserve focus and draft on refresh, failure and unrelated history", async ({ page }) => {
  const f = await settingsAccountFixture(page);
  await page.goto("/#settings");
  const key = keyInput(page);
  await expect(key).toBeVisible();
  await key.fill("fixture-key");
  await expect(page.locator(".credential-action-result")).toContainText("等待保存");
  f.holdAccount(credentialRead);
  await page.getByRole("button", { name: "刷新当前数据依据" }).click();
  await expect.poll(() => f.calls.filter((r) => r.key === credentialRead).length).toBe(2);
  await key.focus();
  f.releaseAccount(credentialRead);
  await expect(key).toBeFocused();
  await expect(key).toHaveValue("fixture-key");
  f.failAccount(savePath, true, 400);
  await page.getByRole("button", { name: "保存 1 项", exact: true }).click();
  await expect(page.locator(".credential-action-result")).toContainText("FIXTURE_SAVE_UNCONFIRMED");
  await expect.poll(() => f.calls.filter((r) => r.key === credentialRead).length).toBe(3);
  await expect(key).toHaveValue("fixture-key");
  await expect(page.locator(".credential-action-result")).not.toContainText("保存完成");
  f.failAccount(savePath, false);
  f.holdAccount(savePath);
  await page.getByRole("button", { name: "保存 1 项", exact: true }).click();
  await expect(key).toBeDisabled();
  await expect(page.locator(".credential-venue select")).toBeDisabled();
  await expect(page.getByRole("button", { name: "保存中", exact: true })).toBeDisabled();
  f.releaseAccount(savePath);
  await expect(page.locator(".credential-action-result")).toContainText("保存完成");
  await expect(key).toHaveValue("");
  await expect(key).toBeEnabled();
  await key.fill("next-draft");
  await page.getByRole("button", { name: "刷新当前数据依据" }).click();
  await expect.poll(() => f.calls.filter((r) => r.key === credentialRead).length).toBe(5);
  await expect(key).toHaveValue("next-draft");
  await page.screenshot({ path: test.info().outputPath("credentials-desktop.png"), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: test.info().outputPath("credentials-mobile.png"), fullPage: true });
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(390);
  expect(f.calls.filter((r) => r.key === savePath)).toHaveLength(2);
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("credential initial failure recovers without guessing fields", async ({ page }) => {
  const f = await settingsAccountFixture(page);
  f.failAccount(credentialRead);
  await page.goto("/#settings");
  await expect(page.locator('[data-credential-spec-state="error"]')).toBeVisible();
  await expect(page.getByRole("button", { name: "填写后保存" })).toBeDisabled();
  f.failAccount(credentialRead, false);
  await page.getByRole("button", { name: "刷新当前数据依据" }).click();
  await expect(keyInput(page)).toBeVisible();
  expect(f.calls.filter((r) => r.key.startsWith("POST"))).toHaveLength(0);
  expect(f.errors).toEqual([]);
});

for (const operation of ["save", "clear", "migrate"]) test(`credential ${operation} delayed receipt is safe after leaving`, async ({ page }) => {
  const f = await settingsAccountFixture(page);
  const path = operation === "save" ? savePath : `POST /api/exchanges/credentials/${operation}`;
  f.holdAccount(path);
  await page.goto("/#settings");
  await expect(keyInput(page)).toBeVisible();
  if (operation === "save") {
    await keyInput(page).fill("fixture-key");
    await page.getByRole("button", { name: "保存 1 项", exact: true }).click();
  } else {
    await page.locator(".credential-maintenance summary").click();
    if (operation === "clear") {
      await expect(page.getByRole("button", { name: "清空已填字段" })).toBeDisabled();
      await page.getByRole("textbox", { name: "清空确认" }).fill("CLEAR okx");
      await page.getByRole("button", { name: "清空已填字段" }).click();
    } else await page.getByRole("button", { name: "迁移到当前存储" }).click();
    await expect(keyInput(page)).toBeDisabled();
  }
  await expect.poll(() => f.calls.filter((r) => r.key === path).length).toBe(1);
  await page.evaluate(() => { location.hash = "review"; });
  await expect(page.locator(".settings-workspace")).toHaveCount(0);
  const response = page.waitForResponse((r) => r.request().method() === "POST" && r.url().includes("/exchanges/credentials"));
  f.releaseAccount(path);
  await (await response).finished();
  await page.evaluate(() => { location.hash = "settings"; });
  await expect(keyInput(page)).toBeEnabled();
  await expect(keyInput(page)).toHaveValue("");
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("execution environment uses returned adapter identity and syncs header without waiting for polling", async ({ page }) => {
  const f = await settingsAccountFixture(page, "execution");
  await page.clock.install({ time: NOW });
  await page.goto("/#settings");
  const enable = page.getByRole("button", { name: "启用实盘", exact: true });
  await expect(enable).toBeEnabled();
  const oldStatus = "GET /api/trading/status", oldAdapters = "GET /api/trading/adapters";
  f.holdAccount(oldStatus); f.holdAccount(oldAdapters);
  const reads = f.calls.filter((r) => r.key === oldStatus).length;
  await page.clock.runFor(5100);
  await expect.poll(() => f.calls.filter((r) => r.key === oldStatus).length).toBe(reads + 1);
  await page.getByRole("button", { name: "刷新执行环境" }).click();
  await expect.poll(() => f.calls.filter((r) => r.key === oldAdapters).length).toBe(2);
  await enable.click();
  f.holdAccount(adapterSave);
  await page.getByRole("button", { name: "确认启用实盘" }).click();
  await expect(enable).toBeDisabled();
  f.releaseAccount(adapterSave);
  await expect(page.getByRole("group", { name: "执行环境", exact: true })).toContainText("实盘");
  await expect(page.getByRole("button", { name: "切回模拟" })).toBeEnabled();
  expect(f.calls.find((r) => r.key === adapterSave)!.body.adapterId).toBe("live_router");
  const responses = ["/trading/status", "/trading/adapters"].map((path) => page.waitForResponse((r) => r.url().endsWith(path)));
  f.releaseAccount(oldStatus); f.releaseAccount(oldAdapters);
  for (const response of responses) await (await response).finished();
  await expect(page.getByRole("group", { name: "执行环境", exact: true })).toContainText("实盘");
  await expect(page.locator(".settings-environment-state")).toContainText("实盘");
  await page.screenshot({ path: test.info().outputPath("environment-desktop.png"), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: test.info().outputPath("environment-mobile.png"), fullPage: true });
  await page.getByRole("button", { name: "切回模拟" }).click();
  await expect(page.getByRole("group", { name: "执行环境", exact: true })).toContainText("模拟");
  expect(f.calls.filter((r) => r.key === adapterSave)).toHaveLength(2);
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("environment read and select failures recover and a late select still updates global state", async ({ page }) => {
  const f = await settingsAccountFixture(page, "execution");
  f.failAccount("GET /api/trading/adapters");
  await page.goto("/#settings");
  await expect(page.getByRole("alert")).toBeVisible();
  await expect(page.getByRole("button", { name: "启用实盘", exact: true })).toBeDisabled();
  f.failAccount("GET /api/trading/adapters", false);
  await page.getByRole("button", { name: "刷新执行环境" }).click();
  await page.getByRole("button", { name: "启用实盘", exact: true }).click();
  f.failAccount(adapterSave, true, 400);
  await page.getByRole("button", { name: "确认启用实盘" }).click();
  await expect(page.locator(".settings-content-panel").getByRole("status")).toContainText("切换失败");
  await expect(page.getByRole("group", { name: "执行环境", exact: true })).toContainText("模拟");
  f.failAccount(adapterSave, false); f.holdAccount(adapterSave);
  await page.getByRole("button", { name: "启用实盘", exact: true }).click();
  await page.getByRole("button", { name: "确认启用实盘" }).click();
  await expect.poll(() => f.calls.filter((r) => r.key === adapterSave).length).toBe(2);
  await page.getByRole("tab", { name: "Webhook", exact: true }).click();
  const response = page.waitForResponse((r) => r.url().endsWith("/adapters/select"));
  f.releaseAccount(adapterSave);
  await (await response).finished();
  await expect(page.getByRole("group", { name: "执行环境", exact: true })).toContainText("实盘");
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("connection probe locks its target and safely ignores late UI feedback", async ({ page }) => {
  const f = await settingsAccountFixture(page, "diagnostics");
  await page.goto("/#settings");
  const probe = page.getByRole("button", { name: "验证连通" });
  await expect(probe).toBeEnabled();
  const path = "GET /api/system/health";
  const reads = f.calls.filter((r) => r.key === path).length;
  f.holdAccount(path);
  await probe.click();
  await expect(page.getByRole("textbox", { name: "API Base", exact: true })).toBeDisabled();
  await expect.poll(() => f.calls.filter((r) => r.key === path).length).toBe(reads + 1);
  await page.getByRole("tab", { name: "Webhook", exact: true }).click();
  const response = page.waitForResponse((r) => r.url().endsWith("/system/health"));
  f.releaseAccount(path);
  await (await response).finished();
  await page.getByRole("tab", { name: "诊断", exact: true }).click();
  await expect(probe).toBeEnabled();
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});
