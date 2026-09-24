import { test, expect } from "@playwright/test";
import { setup, snapshot, API, NOW, WEB } from "./fixtures/onchain-workbench";

test("onchain baseline and editable draft survive quote updates", async ({ page }) => {
  const fixture = await setup(page);
  await page.goto(`${WEB}/#onchain`);
  const rail = page.getByRole("complementary", { name: "链上套利监控配置" });
  await expect(rail.getByText("监控中", { exact: true })).toBeVisible();
  await expect.poll(() => fixture.sockets.size).toBeGreaterThan(0);
  const amount = page.getByLabel("统一对比资金 (USDC)");
  await amount.fill("123.45");
  await fixture.tick();
  await expect(amount).toHaveValue("123.45");
  await expect(amount).toBeFocused();
  await page.screenshot({ path: "output/playwright/onchain-baseline.png", fullPage: true });
  expect(fixture.errors).toEqual([]);
  expect(fixture.writes).toEqual([]);
});

test("failed seed is not reported as loading and offers retry", async ({ page }) => {
  const fixture = await setup(page, { failSeed: true });
  await page.goto(`${WEB}/#onchain`);
  await expect(page.locator(".onchain-rail-header-tools .read-only-flag")).toHaveText("读取失败");
  await expect(page.getByRole("button", { name: /重新读取|重试|刷新/ }).first()).toBeVisible();
  await page.getByRole("button", { name: "立即重试", exact: true }).click();
  await expect(page.locator(".onchain-rail-header-tools .read-only-flag")).toHaveText("监控中");
  expect(fixture.errors).toEqual([]);
});

test("late seed cannot undo a saved pause", async ({ page }) => {
  const fixture = await setup(page, { holdSeed: true });
  await page.goto(`${WEB}/#onchain`);
  await expect.poll(() => fixture.sockets.size).toBeGreaterThan(0);
  fixture.emit(snapshot(NOW + 1));
  await page.getByRole("button", { name: "暂停当前监控" }).click();
  await page.waitForTimeout(100);
  expect(fixture.errors).toEqual([]);
  await expect(page.locator(".onchain-rail-header-tools .read-only-flag")).toHaveText("已暂停");
  const response = page.waitForResponse(`${API}/api/onchain/comparison`);
  fixture.releaseSeed();
  await response;
  await page.waitForTimeout(100);
  await expect(page.locator(".onchain-rail-header-tools .read-only-flag")).toHaveText("已暂停");
  expect(fixture.errors).toEqual([]);
});

test("save locks inputs, preserves failed draft and does not accept an old stream", async ({ page }) => {
  const fixture = await setup(page);
  await page.goto(`${WEB}/#onchain`);
  const amount = page.getByLabel("统一对比资金 (USDC)");
  await expect(amount).toHaveValue("100");
  await amount.fill("123.45");
  fixture.holdSave(true);
  await page.getByRole("button", { name: "应用变更", exact: true }).click();
  await expect(amount).toBeDisabled();
  await expect(page.locator(".onchain-rail-header-tools .read-only-flag")).toHaveText("处理中");
  await expect.poll(() => fixture.requests.filter((request) => request.startsWith("PATCH")).length).toBe(1);
  fixture.tick();
  const response = page.waitForResponse(`${API}/api/onchain/comparison/config`);
  fixture.releaseSave();
  expect((await response).status()).toBe(503);
  await expect(page.getByRole("alert").filter({ hasText: "操作未完成" })).toBeVisible();
  await expect(page.getByRole("alert").filter({ hasText: "操作未完成" })).toContainText("fixture: configuration not saved");
  await expect(amount).toBeEnabled();
  await expect(amount).toHaveValue("123.45");
  expect(fixture.requests.filter((request) => request.startsWith("PATCH"))).toHaveLength(1);
  expect(fixture.errors).toEqual([]);
});

test("stream error keeps old values but blocks construction until a current frame", async ({ page }) => {
  const fixture = await setup(page);
  await page.goto(`${WEB}/#onchain`);
  await expect(page.getByRole("button", { name: "构建交易计划", exact: true })).toBeEnabled();
  await expect.poll(() => fixture.sockets.size).toBeGreaterThan(0);
  fixture.failStream();
  await expect(page.locator(".onchain-market-quality")).toHaveText("状态待确认");
  await expect(page.getByRole("button", { name: "等待最新快照", exact: true })).toBeDisabled();
  await expect(page.locator(".onchain-market-freshness")).toContainText("待确认");
  fixture.emit(snapshot(NOW - 1));
  await expect(page.locator(".onchain-market-quality")).toHaveText("状态待确认");
  fixture.tick();
  await expect(page.getByRole("button", { name: "构建交易计划", exact: true })).toBeEnabled();
  expect(fixture.requests.some((request) => request.includes("/execution/build"))).toBeFalsy();
  expect(fixture.errors).toEqual([]);
});

test("leaving the module safely discards a held save response", async ({ page }) => {
  const fixture = await setup(page);
  await page.goto(`${WEB}/#onchain`);
  fixture.holdSave();
  await page.getByRole("button", { name: "暂停当前监控" }).click();
  await expect.poll(() => fixture.requests.filter((request) => request.startsWith("PATCH")).length).toBe(1);
  await page.locator('.module-tabs button[data-module="futures"]').click();
  const response = page.waitForResponse(`${API}/api/onchain/comparison/config`);
  fixture.releaseSave();
  await response;
  await page.waitForTimeout(150);
  await expect(page.locator(".onchain-page")).toHaveCount(0);
  expect(fixture.errors).toEqual([]);
});

test("configuration controls fit the desktop and mobile workspaces", async ({ page }, info) => {
  const fixture = await setup(page);
  await page.goto(`${WEB}/#onchain`);
  await expect(page.locator(".onchain-rail-header-tools .read-only-flag")).toHaveText("监控中");
  for (const width of [1440, 1024, 390]) {
    await page.setViewportSize({ width, height: 900 });
    if (width < 721) await page.getByRole("navigation", { name: "链上套利工作区" }).getByRole("button", { name: "接入", exact: true }).click();
    await expect(page.getByLabel("统一对比资金 (USDC)")).toBeVisible();
    expect(await page.locator(".onchain-page").evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBeTruthy();
    expect(await page.locator(".onchain-dex-cross-control").evaluate((el) => el.getBoundingClientRect().height)).toBeLessThan(100);
    await page.screenshot({ path: info.outputPath(`onchain-${width}.png`), fullPage: true });
  }
  expect(fixture.errors).toEqual([]);
});
