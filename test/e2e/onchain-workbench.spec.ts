import { test, expect } from "@playwright/test";
import { setup, snapshot, replenishmentRun, API, NOW, WEB } from "./fixtures/onchain-workbench";

test("replenishment confirmation survives quotes and expires with its plan", async ({ page }, info) => {
  const fixture = await setup(page, { scenario: "replenishment", authorizedRun: true });
  await page.goto(`${WEB}/#onchain`);
  await page.getByRole("button", { name: "生成补仓计划", exact: true }).click();
  const input = page.getByRole("textbox", { name: "实盘补仓授权口令" });
  await input.fill("AUTHORIZE LIVE");
  fixture.tick();
  await expect(input).toBeFocused();
  await expect(input).toHaveValue("AUTHORIZE LIVE");
  const progress = replenishmentRun();
  progress.status = "awaiting_source_finality";
  progress.updatedAtMs += 500;
  progress.nextAction = "fixture: 核对已发出的转账";
  fixture.setRestockRows([progress]);
  await expect(page.locator(".onchain-replenishment-run")).toContainText(progress.nextAction);
  await expect(input).toBeFocused();
  await expect(input).toHaveValue("AUTHORIZE LIVE");
  await expect(page.locator(".onchain-replenishment-plan")).toContainText("收益待核算");
  await page.clock.setFixedTime(NOW + 1_000);
  await expect(input).toBeFocused();
  await page.screenshot({ path: info.outputPath("replenishment-plan.png") });
  await page.setViewportSize({ width: 390, height: 900 });
  await page.getByRole("navigation", { name: "链上套利工作区" }).getByRole("button", { name: "套利", exact: true }).click();
  await input.scrollIntoViewIfNeeded();
  const bounds = await input.boundingBox();
  expect(bounds).not.toBeNull();
  expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(390);
  expect(await page.locator(".onchain-page").evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBeTruthy();
  await page.screenshot({ path: info.outputPath("replenishment-mobile.png") });
  await page.clock.setFixedTime(NOW + 61_000);
  await expect(input).toBeDisabled();
  await expect(page.getByRole("button", { name: "授权 60 秒" })).toBeDisabled();
  expect(fixture.requests.filter((request) => request.includes("/authorize") || request.includes("/submit"))).toEqual([]);
  expect(fixture.errors).toEqual([]);
});

test("cross-chain authorization retains its draft on quotes but clears it on configuration change", async ({ page }, info) => {
  const fixture = await setup(page, { scenario: "cross_chain" });
  await page.goto(`${WEB}/#onchain`);
  await page.getByRole("button", { name: "生成闭环预览", exact: true }).click();
  await expect(page.locator(".cross-chain-run-picker")).toBeHidden();
  const input = page.getByRole("textbox", { name: "本次授权短语" });
  await input.fill("AUTHORIZE LIVE");
  fixture.tick();
  await page.clock.setFixedTime(NOW + 1000);
  await expect(input).toBeFocused();
  await expect(input).toHaveValue("AUTHORIZE LIVE");
  await page.screenshot({ path: info.outputPath("cross-chain-preview.png") });
  await page.getByRole("button", { name: "暂停当前监控" }).click();
  await expect(input).toHaveCount(0);
  expect(fixture.requests.filter((r) => r.includes("/authorize") || r.includes("/submit"))).toEqual([]);
  expect(fixture.errors).toEqual([]);
});

for (const scenario of ["replenishment", "cross_chain"] as const) {
  const buildLabel = scenario === "replenishment" ? "生成补仓计划" : "生成闭环预览";
  const path = scenario === "replenishment" ? "/api/onchain/replenishment/build" : "/api/onchain/cross-chain/build";
  test(`${scenario} late preview cannot restore a plan after pausing`, async ({ page }) => {
    const fixture = await setup(page, { scenario, holdPlan: true });
    await page.goto(`${WEB}/#onchain`);
    await page.getByRole("button", { name: buildLabel, exact: true }).click();
    await expect.poll(() => fixture.requests.filter((request) => request.endsWith(path)).length).toBe(1);
    await page.getByRole("button", { name: "暂停当前监控" }).click();
    await expect(page.locator(".onchain-rail-header-tools .read-only-flag")).toHaveText("已暂停");
    const response = page.waitForResponse(`${API}${path}`);
    fixture.releasePlan();
    await response;
    await page.waitForTimeout(100);
    await expect(page.getByRole("textbox", { name: "实盘补仓授权口令" })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "确认本次授权" })).toHaveCount(0);
    expect(fixture.errors).toEqual([]);
  });

  test(`${scenario} leaving the module safely discards a held preview`, async ({ page }) => {
    const fixture = await setup(page, { scenario, holdPlan: true });
    await page.goto(`${WEB}/#onchain`);
    await page.getByRole("button", { name: buildLabel, exact: true }).click();
    await expect.poll(() => fixture.requests.filter((request) => request.endsWith(path)).length).toBe(1);
    await page.locator('.module-tabs button[data-module="futures"]').click();
    const response = page.waitForResponse(`${API}${path}`);
    fixture.releasePlan();
    await response;
    await page.waitForTimeout(100);
    expect(fixture.errors).toEqual([]);
  });
}

test("replenishment timeout retains the original run until a newer receipt arrives", async ({ page }) => {
  const fixture = await setup(page, { authorizedRun: true, failSubmit: true });
  await page.goto(`${WEB}/#onchain`);
  const submit = page.getByRole("button", { name: "提交真实链上充值", exact: true });
  await submit.click();
  await expect(page.getByRole("alert")).toContainText("提交反馈未确认");
  await expect(page.locator(".onchain-replenishment-run")).toBeVisible();
  await expect(submit).toBeDisabled();
  await expect.poll(() => fixture.requests.filter((r) => r.endsWith("/replenishment/runs")).length).toBeGreaterThan(1);
  await expect(submit).toBeDisabled();
  const completed = replenishmentRun();
  completed.status = "completed";
  completed.updatedAtMs += 500;
  completed.nextAction = "fixture: 已按原记录核对到账";
  fixture.setRestockRows([completed]);
  await expect(page.locator(".onchain-replenishment-run")).toContainText(completed.nextAction);
  await expect(submit).toHaveCount(0);
  expect(fixture.requests.filter((r) => r.endsWith("/replenishment/submit"))).toHaveLength(1);
  expect(fixture.errors).toEqual([]);
});

test("lost replenishment authorization is recovered by its original idempotency key", async ({ page }) => {
  const fixture = await setup(page, { scenario: "replenishment", lostAuthorization: true });
  await page.goto(`${WEB}/#onchain`);
  await page.getByRole("button", { name: "生成补仓计划", exact: true }).click();
  await page.getByRole("textbox", { name: "实盘补仓授权口令" }).fill("AUTHORIZE LIVE REPLENISHMENT");
  await page.getByRole("button", { name: "授权 60 秒" }).click();
  await expect(page.getByRole("alert")).toContainText("授权反馈未确认");
  await expect(page.getByRole("button", { name: "提交真实链上充值", exact: true })).toBeEnabled();
  await expect(page.locator(".onchain-replenishment-run")).toContainText("等待提交原资金动作");
  expect(fixture.requests.filter((r) => r.endsWith("/replenishment/authorize"))).toHaveLength(1);
  expect(fixture.requests.filter((r) => r.endsWith("/replenishment/submit"))).toHaveLength(0);
  expect(fixture.errors).toEqual([]);
});

test("quote frames preserve execution controls and use the latest build evidence", async ({ page }) => {
  const fixture = await setup(page);
  await page.goto(`${WEB}/#onchain`);
  const build = page.getByRole("button", { name: "构建交易计划", exact: true });
  await expect(build).toBeEnabled();
  await expect.poll(() => fixture.sockets.size).toBeGreaterThan(0);
  await build.focus();
  fixture.tick();
  await expect(build).toBeFocused();
  const costs = page.locator(".onchain-cost-selection").filter({ hasText: "补库费用" });
  await costs.locator("summary").click();
  fixture.tick();
  await expect(costs).toHaveAttribute("open", "");
  await expect(costs.locator("summary")).toBeFocused();
  const response = page.waitForResponse(`${API}/api/onchain/execution/build`);
  await build.click();
  const sent = (await response).request().postDataJSON();
  expect(sent.expectedQuoteObservedAtMs).toBe(NOW + 2);
  expect(sent.expectedCexObservedAtMs).toBe(NOW + 2);
  expect(fixture.errors).toEqual([]);
});

test("built plan stays mounted across quotes and countdown, then expires without submitting", async ({ page }, info) => {
  const fixture = await setup(page);
  await page.goto(`${WEB}/#onchain`);
  await page.getByRole("button", { name: "构建交易计划", exact: true }).click();
  const plan = page.getByRole("status", { name: "已构建交易计划" });
  await expect(plan).toBeVisible();
  const submit = plan.getByRole("button", { name: "立即执行双腿", exact: true });
  await expect(submit).toBeEnabled();
  await submit.focus();
  fixture.tick();
  await page.clock.setFixedTime(NOW + 1_000);
  await expect(plan).toContainText("19.0s");
  await expect(submit).toBeFocused();
  await page.screenshot({ path: info.outputPath("plan-desktop.png"), fullPage: true });
  await page.setViewportSize({ width: 390, height: 900 });
  await page.getByRole("navigation", { name: "链上套利工作区" }).getByRole("button", { name: "套利", exact: true }).click();
  await submit.scrollIntoViewIfNeeded();
  await expect(submit).toBeVisible();
  expect(await page.locator(".onchain-page").evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBeTruthy();
  const bounds = await submit.boundingBox();
  expect(bounds).not.toBeNull();
  expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(390);
  const navigation = await page.getByRole("navigation", { name: "链上套利工作区" }).boundingBox();
  expect(bounds!.y).toBeGreaterThanOrEqual(navigation!.y + navigation!.height);
  expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(900);
  await page.screenshot({ path: info.outputPath("plan-mobile.png") });
  await page.clock.setFixedTime(NOW + 21_000);
  await expect(plan.getByRole("button", { name: "计划已过期", exact: true })).toBeDisabled();
  expect(fixture.requests.filter((request) => request.includes("/execution/submit"))).toEqual([]);
  expect(fixture.errors).toEqual([]);
});

test("a build response cannot restore a plan after changing configuration", async ({ page }) => {
  const fixture = await setup(page, { holdBuild: true });
  await page.goto(`${WEB}/#onchain`);
  await page.getByRole("button", { name: "构建交易计划", exact: true }).click();
  await expect.poll(() => fixture.requests.filter((request) => request.includes("/execution/build")).length).toBe(1);
  await expect(page.getByRole("button", { name: "构建中…", exact: true })).toBeDisabled();
  await page.getByRole("button", { name: "暂停当前监控" }).click();
  await expect(page.locator(".onchain-rail-header-tools .read-only-flag")).toHaveText("已暂停");
  const response = page.waitForResponse(`${API}/api/onchain/execution/build`);
  fixture.releaseBuild();
  await response;
  await page.waitForTimeout(100);
  await expect(page.getByRole("status", { name: "已构建交易计划" })).toHaveCount(0);
  expect(fixture.errors).toEqual([]);
});

test("a held build is safely discarded after leaving the module", async ({ page }) => {
  const fixture = await setup(page, { holdBuild: true });
  await page.goto(`${WEB}/#onchain`);
  await page.getByRole("button", { name: "构建交易计划", exact: true }).click();
  await expect.poll(() => fixture.requests.filter((request) => request.includes("/execution/build")).length).toBe(1);
  await page.locator('.module-tabs button[data-module="futures"]').click();
  const response = page.waitForResponse(`${API}/api/onchain/execution/build`);
  fixture.releaseBuild();
  await response;
  await page.waitForTimeout(100);
  expect(fixture.errors).toEqual([]);
});

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
  await expect(page.locator(".onchain-readiness-fact").filter({ hasText: "收益" })).not.toContainText("已通过");
  await expect(page.locator(".onchain-route-book-header")).toContainText("报价待确认");
  fixture.emit(snapshot(NOW - 1));
  await expect(page.locator(".onchain-market-quality")).toHaveText("状态待确认");
  fixture.tick();
  await expect(page.getByRole("button", { name: "构建交易计划", exact: true })).toBeEnabled();
  await expect(page.locator(".onchain-readiness-fact").filter({ hasText: "收益" })).toContainText("已通过");
  await expect(page.locator(".onchain-route-book-header")).toContainText("实时预览");
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
