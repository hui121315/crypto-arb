import { expect, test, type Page } from "@playwright/test";
import { settingsAccountFixture } from "./fixtures/settings-account";
import { evidenceRoutes } from "./fixtures/settings-evidence";
import { riskFixture } from "./fixtures/settings-risk";
import { API } from "./fixtures/opportunity-workbench";

const activeNav = (page: Page) => page.locator('nav button[data-module="settings"]');
const summary = (page: Page) => page.locator(".status-summary");
const tab = (page: Page, name: string) => page.getByRole("tablist", { name: "设置分类", exact: true }).getByRole("tab", { name, exact: true });

async function state(page: Page, expected: string) {
  await expect(activeNav(page)).toHaveAttribute("data-runtime-state", expected);
  if (expected === "ready") await expect(summary(page)).toHaveAttribute("data-state", "healthy");
  else await expect(summary(page)).not.toHaveAttribute("data-state", "healthy");
}

test("settings seven panes report their visible read state without hidden-pane failures leaking", async ({ page }, info) => {
  const f = await settingsAccountFixture(page, "execution");
  const e = await evidenceRoutes(page);
  const failures = new Set(["/api/trading/status"]);
  await page.route(API + "/api/**", route => {
    const path = new URL(route.request().url()).pathname;
    return route.request().method() === "GET" && failures.has(path)
      ? route.fulfill({ status: 503, json: { error: { code: "SETTINGS_READ_FAILED", message: `fixture read failed: ${path}` } } })
      : route.fallback();
  });
  f.holdAccount("GET /api/trading/adapters");
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/#settings");
  await state(page, "loading");
  await expect.poll(() => f.calls.some(row => row.key === "GET /api/trading/adapters")).toBe(true);
  f.releaseAccount("GET /api/trading/adapters");
  await state(page, "ready");
  failures.add("/api/trading/adapters");
  await page.getByRole("button", { name: "刷新执行环境", exact: true }).click();
  await state(page, "stale");
  failures.delete("/api/trading/adapters");
  await page.getByRole("button", { name: "刷新执行环境", exact: true }).click();
  await state(page, "ready");

  failures.add("/api/system/market-subscriptions");
  await tab(page, "行情").click();
  await state(page, "error");
  failures.delete("/api/system/market-subscriptions");
  await page.getByRole("button", { name: "刷新行情订阅", exact: true }).click();
  await state(page, "ready");

  failures.add("/api/exchanges/credentials");
  await tab(page, "凭证").click();
  await state(page, "error");
  const credentialTabs = page.getByRole("tablist", { name: "凭证类型", exact: true });
  await credentialTabs.getByRole("tab", { name: "链上凭证", exact: true }).click();
  await state(page, "ready");
  await credentialTabs.getByRole("tab", { name: "交易所账户", exact: true }).click();
  await state(page, "error");
  failures.delete("/api/exchanges/credentials");
  await page.getByRole("button", { name: "刷新当前数据依据", exact: true }).click();
  await state(page, "ready");
  await credentialTabs.getByRole("tab", { name: "链上凭证", exact: true }).click();
  failures.add("/api/onchain/credentials");
  await page.getByRole("button", { name: "刷新凭证状态", exact: true }).click();
  await state(page, "stale");
  await summary(page).click();
  const detail = page.getByRole("group", { name: "当前模块状态", exact: true });
  await expect(detail).toContainText("SETTINGS_READ_FAILED");
  await page.screenshot({ path: info.outputPath("settings-state-desktop.png") });
  await page.setViewportSize({ width: 390, height: 844 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth - innerWidth)).toBeLessThanOrEqual(1);
  await expect(detail).toBeVisible();
  await page.screenshot({ path: info.outputPath("settings-state-mobile.png") });
  await summary(page).click();
  await page.setViewportSize({ width: 1440, height: 900 });
  await credentialTabs.getByRole("tab", { name: "交易所账户", exact: true }).click();
  await state(page, "ready");
  await credentialTabs.getByRole("tab", { name: "链上凭证", exact: true }).click();
  await state(page, "stale");
  failures.delete("/api/onchain/credentials");
  const provider = e.providers.providers[0];
  provider.ready = false;
  provider.missingFields = ["api_key"] as never[];
  await page.getByRole("button", { name: "重新读取", exact: true }).click();
  await state(page, "setup-required");
  provider.ready = true; provider.missingFields = [];
  await page.getByRole("button", { name: "刷新凭证状态", exact: true }).click();
  await state(page, "ready");

  await tab(page, "风控").click();
  await state(page, "error");
  failures.delete("/api/trading/status");
  await page.getByRole("button", { name: "刷新风控状态", exact: true }).click();
  await state(page, "ready");

  failures.add("/api/trading/action-runs");
  await tab(page, "动作账本").click();
  await state(page, "error");
  failures.delete("/api/trading/action-runs");
  await page.getByRole("button", { name: "刷新", exact: true }).click();
  await state(page, "ready");
  failures.add("/api/trading/action-runs/fixture-action-0");
  await page.locator('[data-action-id="fixture-action-0"]').getByRole("button", { name: "详情", exact: true }).click();
  await state(page, "error");

  failures.add("/api/webhook/status");
  await tab(page, "Webhook").click();
  await state(page, "error");
  failures.delete("/api/webhook/status");
  await page.getByRole("button", { name: "刷新 Webhook 状态", exact: true }).click();
  await state(page, "ready");
  f.webhook.config.urlConfigured = false; f.emit();
  await state(page, "setup-required");
  f.webhook.config.urlConfigured = true; f.emit();
  await state(page, "ready");

  failures.add("/api/system/market-data-diagnostics");
  failures.add("/api/system/venue-runtime-health");
  await tab(page, "诊断").click();
  await state(page, "ready");
  const diagnostics = page.getByRole("tablist", { name: "诊断范围", exact: true });
  await diagnostics.getByRole("tab", { name: "行情", exact: true }).click();
  await state(page, "error");
  await diagnostics.getByRole("tab", { name: "连接", exact: true }).click();
  await state(page, "ready");
  await diagnostics.getByRole("tab", { name: "交易", exact: true }).click();
  await state(page, "error");
  failures.clear();
  await page.getByRole("button", { name: "刷新全部诊断", exact: true }).click();
  await state(page, "ready");
  await diagnostics.getByRole("tab", { name: "行情", exact: true }).click();
  await state(page, "ready");
  await diagnostics.getByRole("tab", { name: "运行数据依据", exact: true }).click();
  await state(page, "ready");
  await page.evaluate(() => { location.hash = "futures"; });
  f.tick();
  await expect(summary(page)).toHaveAttribute("data-state", "healthy");
  await page.evaluate(() => { location.hash = "settings"; });
  await state(page, "ready");
  expect(f.requests.filter(row => row.method !== "GET")).toEqual([]);
  expect(f.calls.filter(row => row.key.startsWith("POST"))).toEqual([]);
  expect(e.calls.filter(row => row.key.startsWith("POST"))).toEqual([]);
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});

test("settings action status survives navigation and only confirmed results clear pending", async ({ page }) => {
  const f = await riskFixture(page);
  await page.goto("/#settings");
  const amount = page.getByLabel(/单笔名义上限 USD/);
  const save = page.getByRole("button", { name: "保存风控", exact: true });
  await state(page, "ready");
  await amount.fill("12.75");
  f.hold(); f.fail(true);
  await save.click();
  await state(page, "pending");
  await expect.poll(() => f.calls.length).toBe(1);
  await tab(page, "Webhook").click();
  await state(page, "ready");
  f.release();
  await expect.poll(() => f.status.risk.maxOrderNotional).toBe(12.75);
  await state(page, "ready");
  await tab(page, "风控").click();
  await state(page, "pending");
  await expect(summary(page)).toContainText("原操作结果待核对");
  await expect(amount).toBeDisabled();
  f.fail(false);
  await page.getByRole("button", { name: "核对上次操作", exact: true }).click();
  await state(page, "ready");
  await expect(amount).toHaveValue("12.75");
  expect(f.calls).toHaveLength(1);

  await amount.fill("20.5");
  f.reject(true);
  await save.click();
  await state(page, "pending");
  await page.getByRole("button", { name: "核对上次操作", exact: true }).click();
  await state(page, "error");
  await summary(page).click();
  await expect(page.getByRole("group", { name: "当前模块状态", exact: true })).toContainText("FIXTURE_REJECTED");
  await summary(page).click();
  await amount.fill("21.5");
  await state(page, "ready");
  f.reject(false);
  await save.click();
  await state(page, "ready");
  await expect.poll(() => f.status.risk.maxOrderNotional).toBe(21.5);
  expect(f.calls).toHaveLength(3);
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});
