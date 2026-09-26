import { expect, test, type Page } from "@playwright/test";
import { settingsAccountFixture } from "./fixtures/settings-account";

const save = "POST /api/exchanges/credentials";
const select = "POST /api/trading/adapters/select";
const records = (page: Page) => page.evaluate(() => Object.entries(sessionStorage).filter(([key]) => key.startsWith("crossline.settings.pending.v1:")));
const recovery = (page: Page) => page.getByRole("alert", { name: "设置操作待核对" });

async function screenshot(page: Page, name: string) {
  await page.screenshot({ path: test.info().outputPath(`${name}-desktop.png`) });
  await page.setViewportSize({ width: 390, height: 844 });
  await recovery(page).scrollIntoViewIfNeeded();
  await expect(recovery(page).getByRole("button")).toBeInViewport();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
  await page.screenshot({ path: test.info().outputPath(`${name}-mobile.png`) });
  await page.setViewportSize({ width: 1280, height: 720 });
}

test("credential reload recovers save migrate and clear without repeating writes or keeping secrets", async ({ page }) => {
  const f = await settingsAccountFixture(page);
  await page.goto("/#settings");
  const venue = page.locator(".credential-venue select");
  const input = page.locator('.credential-fields input[name="api_key"]');
  await venue.selectOption("binance");
  await input.fill("fixture-reload-sensitive-value");
  f.holdAccount(save);
  await page.getByRole("button", { name: "保存 1 项", exact: true }).click();
  await expect.poll(() => f.calls.filter((row) => row.key === save).length).toBe(1);
  const stored = await records(page);
  expect(stored).toHaveLength(1);
  expect(stored[0][0]).toMatch(/^crossline\.settings\.pending\.v1:credentials:[0-9a-f]{64}$/);
  expect(JSON.stringify(stored)).not.toContain("fixture-reload-sensitive-value");
  expect(JSON.stringify(stored)).not.toContain("isolated-fixture-token");
  expect(JSON.parse(stored[0][1]).context.request_id).toBe(f.calls.find((r) => r.key === save)!.requestId);
  await page.reload();
  await expect(recovery(page)).toContainText("binance · 保存凭证结果待核对");
  await expect(page.locator(".credential-action-result")).toContainText("保存结果待核对");
  await expect(page.locator(".credential-action-result")).not.toContainText("未提交");
  await expect(venue).toHaveValue("binance");
  await expect(venue).toBeDisabled();
  await expect(input).toHaveValue("");
  await expect(input).toBeDisabled();
  const run = f.actions.data.shift();
  await recovery(page).getByRole("button").click();
  await expect(recovery(page)).toContainText("SETTINGS_RECEIPT_NOT_FOUND");
  f.actions.data.unshift(run);
  await recovery(page).getByRole("button").click();
  await expect(recovery(page)).toContainText("后端已受理");
  expect(JSON.parse((await records(page))[0][1]).run_id).toBe(run.id);
  f.releaseAccount(save);
  await expect.poll(() => run.status).toBe("succeeded");
  run.result.venue = "okx";
  await recovery(page).getByRole("button").click();
  await expect(recovery(page)).toContainText("SETTINGS_RECEIPT_MISMATCH");
  await expect(input).toBeDisabled();
  await screenshot(page, "credential-reload");
  run.result.venue = "binance";
  await recovery(page).getByRole("button").click();
  await expect(recovery(page)).toHaveCount(0);
  await expect(input).toBeEnabled();
  expect(await records(page)).toEqual([]);

  // Denied persistence must fail before issuing a new write.
  await page.evaluate(() => {
    const original = Storage.prototype.setItem;
    Storage.prototype.setItem = function (key, value) {
      if (key.startsWith("crossline.settings.pending.v1:") && (window as any).denySettingsSave) throw new DOMException("fixture quota", "QuotaExceededError");
      return original.call(this, key, value);
    };
    (window as any).denySettingsSave = true;
  });
  await input.fill("fixture-quota-value");
  await page.getByRole("button", { name: "保存 1 项", exact: true }).click();
  await expect(recovery(page)).toContainText("无法更新恢复记录");
  expect(f.calls.filter((row) => row.key === save)).toHaveLength(1);
  await expect(input).toBeDisabled();
  await page.evaluate(() => { (window as any).denySettingsSave = false; });
  await recovery(page).getByRole("button").click();
  await expect(input).toBeEnabled();

  for (const operation of ["migrate", "clear"]) {
    const path = `POST /api/exchanges/credentials/${operation}`;
    await page.locator(".credential-maintenance summary").click();
    f.holdAccount(path);
    if (operation === "clear") {
      f.failAccount(path);
      await page.getByRole("textbox", { name: "清空确认" }).fill("CLEAR binance");
      await page.getByRole("button", { name: "清空已填字段" }).click();
    } else await page.getByRole("button", { name: "迁移到当前存储" }).click();
    await expect.poll(() => f.calls.filter((row) => row.key === path).length).toBe(1);
    await page.reload();
    await expect(recovery(page)).toContainText(operation === "clear" ? "清除凭证结果待核对" : "迁移凭证结果待核对");
    await expect(input).toBeDisabled();
    await expect(input).toHaveValue("");
    await page.locator(".credential-maintenance summary").click();
    await expect(page.getByRole("textbox", { name: "清空确认" })).toHaveValue("");
    await expect(page.getByRole("button", { name: "迁移到当前存储" })).toBeDisabled();
    f.releaseAccount(path);
    await expect.poll(() => f.actions.data[0].status).toBe(operation === "clear" ? "failed" : "succeeded");
    await recovery(page).getByRole("button").click();
    await expect(recovery(page)).toHaveCount(0);
    await expect(input).toBeEnabled();
    expect(await records(page)).toEqual([]);
    await page.locator(".credential-maintenance summary").click();
  }
  expect(f.calls.filter((row) => row.key.startsWith("POST"))).toHaveLength(3);
  expect(JSON.stringify(f.actions)).not.toContain("fixture-reload-sensitive-value");
  expect(await page.evaluate(() => JSON.stringify({ ...localStorage, ...sessionStorage }))).not.toContain("fixture-reload-sensitive-value");
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});

test("environment reload only reads original receipts and never restores a historical live mode", async ({ page }) => {
  const f = await settingsAccountFixture(page, "execution");
  await page.goto("/#settings");
  const enable = page.getByRole("button", { name: "启用实盘", exact: true });
  const confirm = page.getByRole("button", { name: "确认启用实盘", exact: true });
  const header = page.getByRole("group", { name: "执行环境", exact: true });
  await enable.click();
  await page.reload();
  await expect(enable).toBeEnabled();
  await expect(confirm).toHaveCount(0);
  expect(f.calls.filter((row) => row.key === select)).toHaveLength(0);
  f.holdAccount(select);
  await enable.click(); await confirm.click();
  await expect.poll(() => f.calls.filter((row) => row.key === select).length).toBe(1);
  await page.reload();
  await expect(recovery(page)).toContainText("live_router · 切换执行环境结果待核对");
  await expect(enable).toBeDisabled();
  const run = f.actions.data[0];
  await recovery(page).getByRole("button").click();
  await expect(recovery(page)).toContainText("后端已受理");
  f.releaseAccount(select);
  await expect.poll(() => run.status).toBe("succeeded");
  expect(run.result.environment).toBe("live");
  const paper = f.adapters.options.find((row: any) => row.environment === "paper");
  // Another operator has since selected Paper; the old Live receipt stays immutable.
  f.adapters.current = paper.id; f.adapters.currentEnvironment = paper.environment;
  f.status.adapter = paper.id; f.status.environment = paper.environment; f.status.risk.liveTradingEnabled = false;
  await page.reload();
  await expect(recovery(page)).toContainText("live_router");
  await expect(header).toContainText("模拟");
  const evidence = page.locator("details").filter({ has: page.locator("summary", { hasText: /^操作证据$/ }) });
  await expect(page.locator(".settings-content-panel").getByRole("status")).toContainText("上次切换结果待核对");
  await expect(evidence.locator("p")).not.toBeVisible();
  await evidence.locator("summary").click();
  await expect(evidence).toContainText(run.id);
  await evidence.locator("summary").click();
  await screenshot(page, "environment-reload");
  await recovery(page).getByRole("button").click();
  await expect(recovery(page)).toHaveCount(0);
  await expect(enable).toBeEnabled();
  await expect(header).toContainText("模拟");
  await expect(page.locator(".settings-environment-state")).toContainText("模拟");
  expect(f.calls.filter((row) => row.key === select)).toHaveLength(1);
  expect(await records(page)).toEqual([]);

  f.failAccount(select); f.holdAccount(select);
  await enable.click(); await confirm.click();
  await expect.poll(() => f.calls.filter((row) => row.key === select).length).toBe(2);
  const stored = (await records(page))[0];
  await page.getByRole("tab", { name: "诊断", exact: true }).click();
  await page.getByLabel("Token", { exact: true }).fill("fixture-other-auth");
  await page.getByRole("button", { name: "保存 Token", exact: true }).click();
  await page.getByRole("tab", { name: "执行环境", exact: true }).click();
  await expect(recovery(page)).toHaveCount(0);
  await expect(enable).toBeEnabled();
  expect(await records(page)).toEqual([stored]);
  // Init fixture restores the original auth; its pending action must reappear.
  await page.reload();
  await expect(recovery(page)).toContainText("结果待核对");
  await expect(enable).toBeDisabled();
  await page.evaluate(([key]) => sessionStorage.setItem(key, "{broken"), stored);
  await page.reload();
  await expect(recovery(page)).toContainText("恢复记录损坏");
  await expect(enable).toBeDisabled();
  await recovery(page).getByRole("button").click();
  await expect(enable).toBeDisabled();
  await page.evaluate(([key, value]) => sessionStorage.setItem(key, value), stored);
  await recovery(page).getByRole("button").click();
  f.releaseAccount(select);
  await expect.poll(() => f.actions.data[0].status).toBe("failed");
  await recovery(page).getByRole("button").click();
  await expect(recovery(page)).toHaveCount(0);
  await expect(enable).toBeEnabled();
  expect(f.calls.filter((row) => row.key === select)).toHaveLength(2);
  expect(await records(page)).toEqual([]);
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});
