import { expect, test, type Page } from "@playwright/test";
import { settingsAccountFixture } from "./fixtures/settings-account";
import { API, NOW } from "./fixtures/opportunity-workbench";

const savePath = "POST /api/exchanges/credentials";
const adapterPath = "POST /api/trading/adapters/select";
const keyInput = (page: Page) => page.locator('.credential-fields input[name="api_key"]');

async function fixture(page: Page, tab = "credentials") {
  const f = await settingsAccountFixture(page, tab);
  const executed = await (await page.request.get(`${API}/api/review/executed`)).json();
  const strategyPerformance = await (await page.request.get(`${API}/api/review/strategy-performance`)).json();
  await page.route("**/api/review/runtime", (route) => route.fulfill({ json: {
    executed, strategyPerformance, generatedAtMs: NOW,
  } }));
  return f;
}

async function leave(page: Page) {
  await page.evaluate(() => { location.hash = "review"; });
  await expect(page.locator(".settings-workspace")).toHaveCount(0);
}

async function back(page: Page) {
  await page.evaluate(() => { location.hash = "settings"; });
  await expect(page.locator(".settings-workspace")).toBeVisible();
}

test("credential writes keep their venue and lock across navigation without retaining secret drafts", async ({ page }) => {
  const f = await fixture(page);
  await page.goto("/#settings");
  const venue = page.locator(".credential-venue select");
  const key = keyInput(page);
  const result = page.locator(".credential-action-result");
  await expect(key).toBeEnabled();
  await venue.selectOption("binance");
  await key.fill("fixture-secret-only");
  f.holdAccount(savePath); f.failAccount(savePath, true, 400);
  await page.getByRole("button", { name: "保存 1 项", exact: true }).click();
  await expect.poll(() => f.calls.filter((r) => r.key === savePath).length).toBe(1);
  await leave(page); await back(page);
  await expect(venue).toHaveValue("binance");
  await expect(venue).toBeDisabled();
  await expect(key).toHaveValue("");
  await expect(key).toBeDisabled();
  await expect(page.getByRole("button", { name: "保存中", exact: true })).toBeDisabled();
  await expect(result).toContainText("正在保存");
  await expect(result).not.toContainText("保存完成");
  f.releaseAccount(savePath);
  await expect(result).toContainText("FIXTURE_SAVE_UNCONFIRMED");
  await expect(result).toContainText("Binance");
  await expect(page.locator(".settings-content-panel")).toContainText("密钥输入已清空");
  await expect(key).toBeEnabled();
  await key.fill("fixture-secret-only");
  f.failAccount(savePath, false); f.holdAccount(savePath);
  await page.getByRole("button", { name: "保存 1 项", exact: true }).click();
  await page.getByRole("tab", { name: "Webhook", exact: true }).click();
  await page.getByRole("tab", { name: "凭证", exact: true }).click();
  await expect(page.getByRole("button", { name: "保存中", exact: true })).toBeDisabled();
  await leave(page);
  const saved = page.waitForResponse((r) => r.request().method() === "POST" && r.url().endsWith("/exchanges/credentials"));
  f.releaseAccount(savePath);
  await (await saved).finished();
  await back(page);
  await expect(result).toContainText("保存完成");
  await expect(key).toHaveValue("");
  // Historical success must not clear the next draft on a normal refresh.
  await key.fill("next-unsaved-secret");
  await page.getByRole("button", { name: "刷新当前数据依据" }).click();
  await expect(key).toHaveValue("next-unsaved-secret");
  await leave(page); await back(page);
  await expect(key).toHaveValue("");
  await expect(page.locator(".settings-content-panel")).toContainText("密钥输入已清空");

  for (const operation of ["migrate", "clear"]) {
    const path = `POST /api/exchanges/credentials/${operation}`;
    await page.locator(".credential-maintenance summary").click();
    f.holdAccount(path);
    if (operation === "clear") {
      await page.getByRole("textbox", { name: "清空确认" }).fill("CLEAR binance");
      await page.getByRole("button", { name: "清空已填字段" }).click();
    } else await page.getByRole("button", { name: "迁移到当前存储" }).click();
    await expect.poll(() => f.calls.filter((r) => r.key === path).length).toBe(1);
    await leave(page); await back(page);
    await expect(venue).toHaveValue("binance");
    await expect(venue).toBeDisabled();
    await expect(key).toBeDisabled();
    await page.locator(".credential-maintenance summary").click();
    await expect(page.getByRole("button", { name: "处理中", exact: true })).toBeDisabled();
    await expect(page.getByRole("textbox", { name: "清空确认" })).toHaveValue("");
    f.releaseAccount(path);
    await expect(key).toBeEnabled();
    await expect(page.locator(".credential-maintenance-actions em")).toContainText(operation === "clear" ? "清空" : "迁移");
    await page.locator(".credential-maintenance summary").click();
  }
  await venue.selectOption("okx");
  await expect(result).toContainText("等待保存");
  await expect(result).not.toContainText("保存完成");
  await expect(page.locator(".settings-content-panel")).not.toContainText("密钥输入已清空");
  expect(f.calls.filter((r) => r.key === savePath)).toHaveLength(2);
  expect(f.calls.filter((r) => r.key.startsWith("POST")).every((r) => r.body.venue === "binance")).toBe(true);
  expect(await page.evaluate(() => JSON.stringify({ ...localStorage, ...sessionStorage }))).not.toContain("secret");
  await page.screenshot({ path: test.info().outputPath("credentials-desktop.png") });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.getByRole("button", { name: "填写后保存" }).scrollIntoViewIfNeeded();
  await expect(page.getByRole("button", { name: "填写后保存" })).toBeInViewport();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
  await page.screenshot({ path: test.info().outputPath("credentials-mobile.png") });
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});

test("environment navigation retains pending and uncertain changes, then reconciles the original request", async ({ page }) => {
  const f = await fixture(page, "execution");
  await page.clock.install({ time: NOW });
  await page.goto("/#settings");
  const enable = page.getByRole("button", { name: "启用实盘", exact: true });
  const header = page.getByRole("group", { name: "执行环境", exact: true });
  await expect(enable).toBeEnabled();
  const readPath = "GET /api/trading/adapters";
  f.holdAccount(readPath);
  await page.getByRole("button", { name: "刷新执行环境" }).click();
  await expect.poll(() => f.calls.filter((r) => r.key === readPath).length).toBe(2);
  f.holdAccount(adapterPath);
  await enable.click();
  await page.getByRole("button", { name: "确认启用实盘" }).click();
  await expect.poll(() => f.calls.filter((r) => r.key === adapterPath).length).toBe(1);
  await leave(page); await back(page);
  await expect(enable).toBeDisabled();
  await expect(page.locator(".settings-content-panel").getByRole("status")).toContainText("正在切换");
  f.releaseAccount(adapterPath);
  await expect(header).toContainText("实盘");
  await expect(page.getByRole("button", { name: "切回模拟" })).toBeEnabled();
  const oldRead = page.waitForResponse((r) => r.url().endsWith("/trading/adapters"));
  f.releaseAccount(readPath); await (await oldRead).finished();
  await expect(page.locator(".settings-environment-state")).toContainText("实盘");

  // The backend applies the second request, but the response is lost.
  let loseResponse = true;
  await page.route("**/api/trading/adapters/select", async (route) => {
    if (!loseResponse) return route.fallback();
    loseResponse = false;
    const request = route.request();
    const body = request.postDataJSON();
    f.calls.push({ key: adapterPath, body, idempotency: request.headers()["idempotency-key"] });
    const option = f.adapters.options.find((row: any) => row.id === body.adapterId);
    f.adapters.current = option.id; f.adapters.currentEnvironment = option.environment;
    f.status.adapter = option.id; f.status.environment = option.environment;
    f.status.risk.liveTradingEnabled = false;
    const id = "fixture-lost-environment";
    const requestId = request.headers()["x-request-id"], idempotencyKey = request.headers()["idempotency-key"];
    f.actions.data.unshift({ id, kind: "trading_adapter_select", status: "succeeded", actor: "fixture",
      target: option.id, requestId, idempotencyKey, message: "fixture environment confirmed", startedAtMs: NOW, updatedAtMs: NOW,
      result: { ...structuredClone(f.status), requestId, actionRunId: id, idempotencyKey } });
    await route.fulfill({ status: 504, json: { error: { code: "TIMEOUT", message: "fixture receipt lost" } } });
  });
  await page.getByRole("button", { name: "切回模拟" }).click();
  const retry = page.getByRole("button", { name: "核对上次切换" });
  await expect(retry).toBeEnabled();
  await leave(page); await back(page);
  await expect(retry).toBeEnabled();
  await expect(enable).toBeDisabled();
  const receiptPath = "GET /api/trading/action-runs";
  const receiptReads = f.calls.filter((r) => r.key === receiptPath).length;
  f.holdAccount(receiptPath);
  await retry.click();
  await expect.poll(() => f.calls.filter((r) => r.key === receiptPath).length).toBe(receiptReads + 1);
  await leave(page); await back(page);
  await expect(enable).toBeDisabled();
  f.releaseAccount(receiptPath);
  await expect(header).toContainText("模拟");
  await expect(page.locator(".settings-environment-state")).toContainText("模拟");
  await expect(retry).toHaveCount(0);
  await expect(enable).toBeEnabled();
  const writes = f.calls.filter((r) => r.key === adapterPath);
  expect(writes[0].body.adapterId).toBe("live_router");
  expect(writes).toHaveLength(2);
  expect(writes[1].idempotency).toBeTruthy();
  expect(writes[0].idempotency).not.toBe(writes[1].idempotency);
  await leave(page);
  const reads = f.calls.filter((r) => r.key === readPath).length;
  await page.clock.runFor(25_000);
  expect(f.calls.filter((r) => r.key === readPath)).toHaveLength(reads);
  await back(page);
  await page.screenshot({ path: test.info().outputPath("environment-desktop.png") });
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(enable).toBeInViewport();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
  await page.screenshot({ path: test.info().outputPath("environment-mobile.png") });
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});
