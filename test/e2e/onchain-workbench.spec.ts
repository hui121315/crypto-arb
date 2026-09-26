import { test, expect } from "@playwright/test";
import { setup, snapshot, batchItem, executionPlan, executionRun, replenishmentRun, crossChainRun, recoveryPlan, API, NOW, WEB } from "./fixtures/onchain-workbench";

test("four-step cross-chain cycle waits for destination receipts and preserves expanded details", async ({ page }, info) => {
  const fixture = await setup(page, { scenario: "cross_chain", crossCycle: true, crossAuthorizeLost: true });
  await page.goto(`${WEB}/#onchain`);
  await page.getByRole("button", { name: "生成完整流程预览", exact: true }).click();
  await page.getByRole("textbox", { name: "本次授权短语" }).fill("AUTHORIZE LIVE CROSS CHAIN");
  await page.getByRole("button", { name: "确认本次授权", exact: true }).click();
  const region = page.getByRole("region", { name: "跨链执行与到账记录" });
  const submit = region.locator(".cross-chain-submit-step");
  await expect(submit).toHaveText("重报价并提交第 1 步");
  await expect(region.getByRole("alert")).toHaveCount(0);
  await expect(region.locator(".cross-chain-progress-leg")).toHaveCount(4);
  for (let position = 1; position <= 4; position++) {
    await expect(submit).toHaveText(`重报价并提交第 ${position} 步`);
    await submit.click();
    await expect(submit).toBeDisabled();
    const leg = region.locator(`.cross-chain-progress-leg[data-position="${position}"]`);
    await leg.locator("summary").click();
    const details = leg.locator("details");
    await expect(details).toHaveAttribute("open", "");
    if (position === 2 || position === 4) {
      fixture.progressCycle(position, "source_confirmed");
      await region.getByRole("button", { name: "刷新记录", exact: true }).click();
      await expect(region.locator(".cross-chain-run-heading")).toContainText("等待目标链到账");
      await expect(submit).toBeDisabled();
      await expect(details).toHaveAttribute("open", "");
      await expect(leg.locator(".cross-chain-leg-output")).toContainText("待确认");
    }
    fixture.progressCycle(position, "completed");
    await region.getByRole("button", { name: "刷新记录", exact: true }).click();
    await expect(region.locator(".cross-chain-step-summary")).toContainText(`已完成 ${position} / 4 步`);
    await expect(details).toHaveAttribute("open", "");
    await details.locator("summary").click();
  }
  await expect(submit).toBeHidden();
  await expect(region).toContainText("资产路径已完成；费用与净收益仍待核对。");
  await expect(region.locator('[data-position="2"] .cross-chain-leg-output')).toContainText("0.995 WSOL");
  await expect(region.locator('[data-position="4"] .cross-chain-leg-output')).toContainText("103 USDC");
  await page.reload();
  await expect(region.locator(".cross-chain-step-summary")).toContainText("已完成 4 / 4 步");
  await region.scrollIntoViewIfNeeded();
  await page.screenshot({ path: info.outputPath("cross-chain-cycle-desktop.png") });
  await page.setViewportSize({ width: 390, height: 900 });
  await page.getByRole("navigation", { name: "链上套利工作区" }).getByRole("button", { name: "套利", exact: true }).click();
  await region.scrollIntoViewIfNeeded();
  expect(await page.locator(".onchain-page").evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBeTruthy();
  await region.screenshot({ path: info.outputPath("cross-chain-cycle-mobile.png") });
  expect(fixture.requests.filter((r) => r === "POST /api/onchain/cross-chain/authorize")).toHaveLength(1);
  expect(fixture.requests.filter((r) => r === "POST /api/onchain/cross-chain/submit")).toHaveLength(4);
  expect(fixture.errors).toEqual([]);
});

test("unknown cross-chain step stays locked through late earlier receipts and reload", async ({ page }) => {
  const fixture = await setup(page, { scenario: "cross_chain", crossCycle: true });
  await page.goto(`${WEB}/#onchain`);
  await page.getByRole("button", { name: "生成完整流程预览", exact: true }).click();
  await page.getByRole("textbox", { name: "本次授权短语" }).fill("AUTHORIZE LIVE CROSS CHAIN");
  await page.getByRole("button", { name: "确认本次授权", exact: true }).click();
  const region = page.getByRole("region", { name: "跨链执行与到账记录" });
  const submit = region.locator(".cross-chain-submit-step");
  await submit.click();
  await expect(submit).toBeDisabled();
  fixture.progressCycle(1, "completed");
  await region.getByRole("button", { name: "刷新记录", exact: true }).click();
  await expect(submit).toHaveText("重报价并提交第 2 步");
  fixture.crossSubmitMode("unknown");
  await submit.click();
  await expect(region).toContainText("正在核对请求结果");
  fixture.reviseEarlierCycleReceipt();
  await region.getByRole("button", { name: "刷新记录", exact: true }).click();
  await expect(submit).toBeDisabled();
  await page.reload();
  await expect(region).toContainText("正在核对请求结果");
  await expect(submit).toBeDisabled();
  fixture.progressCycle(2, "paused");
  await region.getByRole("button", { name: "刷新记录", exact: true }).click();
  await expect(region.getByRole("alert")).toHaveCount(0);
  await expect(region).not.toContainText("正在核对请求结果");
  await region.getByRole("button", { name: "重新核对到账", exact: true }).click();
  await expect(region.locator(".cross-chain-run-heading")).toContainText("等待目标链到账");
  await expect(submit).toBeDisabled();
  expect(fixture.requests.filter((r) => r === "POST /api/onchain/cross-chain/submit")).toHaveLength(2);
  expect(fixture.requests.filter((r) => r === "POST /api/onchain/cross-chain/recheck")).toHaveLength(1);
  expect(fixture.errors).toEqual([]);
});

test("cross-chain pre-write rejection permits retry only after refreshing the original step", async ({ page }) => {
  const fixture = await setup(page, { scenario: "cross_chain", crossCycle: true, crossSubmitMode: "reject" });
  await page.goto(`${WEB}/#onchain`);
  await page.getByRole("button", { name: "生成完整流程预览", exact: true }).click();
  await page.getByRole("textbox", { name: "本次授权短语" }).fill("AUTHORIZE LIVE CROSS CHAIN");
  await page.getByRole("button", { name: "确认本次授权", exact: true }).click();
  const region = page.getByRole("region", { name: "跨链执行与到账记录" });
  const submit = region.locator(".cross-chain-submit-step");
  await submit.click();
  await expect(region.getByRole("alert")).toContainText("step not acknowledged");
  await expect(submit).toBeEnabled();
  fixture.crossSubmitMode();
  await submit.click();
  await expect(region.locator(".cross-chain-run-heading")).toContainText("等待源链确认");
  await expect(region.getByRole("alert")).toHaveCount(0);
  await expect(submit).toBeDisabled();
  expect(fixture.requests.filter((r) => r === "POST /api/onchain/cross-chain/submit")).toHaveLength(2);
  expect(fixture.errors).toEqual([]);
});

test("execution receipt survives batch add switch remove rebuild and refresh without resubmitting", async ({ page }, info) => {
  const fixture = await setup(page, { execution: "ack" });
  fixture.setBatchItems([batchItem("fixture-remove")]);
  await page.goto(`${WEB}/#onchain`);
  await page.locator(".onchain-batch-add").click();
  await page.getByRole("tab", { name: "监控", exact: true }).click();
  const added = page.locator('tr[data-item-id="fixture-added-1"]');
  await expect(added).toContainText("SOL");
  await added.getByRole("button", { name: "载入", exact: true }).click();
  const remove = page.locator('tr[data-item-id="fixture-remove"]');
  await remove.getByRole("button", { name: "移除", exact: true }).click();
  await expect(remove).toHaveCount(0);
  await page.getByRole("button", { name: "构建交易计划", exact: true }).click();
  await page.locator(".onchain-submit-action").click();
  const receipts = page.getByRole("region", { name: "执行结果", exact: true });
  await expect(receipts).toContainText("按顺序执行中");
  await expect(page.locator(".onchain-submit-action")).toBeDisabled();
  await expect(page.locator(".onchain-build-boundary")).toContainText("此计划已提交");
  fixture.setExecutionRows([executionRun("completed", "fixture-build", NOW + 100)]);
  await receipts.getByRole("button", { name: "刷新执行结果" }).click();
  await expect(receipts).toContainText("全部腿已完成");
  await page.getByRole("button", { name: "构建交易计划", exact: true }).click();
  await expect(page.locator(".onchain-submit-action")).toBeEnabled();
  await expect(receipts).toContainText("全部腿已完成");
  await page.reload();
  await expect(receipts.locator(".onchain-submit-result")).toContainText("全部腿已完成");
  await expect(receipts.locator(".onchain-submit-exposure")).toHaveClass(/is-pending/);
  const selector = receipts.getByRole("combobox", { name: "选择执行记录" });
  expect(await selector.evaluate((el) => getComputedStyle(el).backgroundColor)).not.toBe("rgb(255, 255, 255)");
  await receipts.scrollIntoViewIfNeeded();
  await page.screenshot({ path: info.outputPath("execution-receipts-desktop.png") });
  await page.setViewportSize({ width: 390, height: 900 });
  await page.getByRole("navigation", { name: "链上套利工作区" }).getByRole("button", { name: "套利", exact: true }).click();
  await receipts.scrollIntoViewIfNeeded();
  expect(await page.locator(".onchain-page").evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBeTruthy();
  await page.screenshot({ path: info.outputPath("execution-receipts-mobile.png") });
  expect(fixture.requests.filter((r) => r === "POST /api/onchain/execution/submit")).toHaveLength(1);
  expect(fixture.errors).toEqual([]);
});

test("unknown execution remains locked across empty history and reload until its own receipt arrives", async ({ page }) => {
  const fixture = await setup(page, { execution: "unknown" });
  await page.goto(`${WEB}/#onchain`);
  await page.getByRole("button", { name: "构建交易计划", exact: true }).click();
  await page.locator(".onchain-submit-action").click();
  const receipts = page.getByRole("region", { name: "执行结果", exact: true });
  await expect(receipts).toContainText("尚未找到构建 fixture-build");
  await expect(page.locator(".onchain-submit-action")).toBeDisabled();
  await page.reload();
  await expect(receipts).toContainText("尚未找到构建 fixture-build");
  fixture.setExecutionRows([executionRun("completed", "unrelated")]);
  await receipts.getByRole("button", { name: "刷新执行结果" }).click();
  await expect(receipts).toContainText("尚未找到构建 fixture-build");
  await page.getByRole("button", { name: "构建交易计划", exact: true }).click();
  await expect(page.locator(".onchain-submit-action")).toBeDisabled();
  fixture.setExecutionRows([executionRun("completed", "fixture-build", NOW + 100), executionRun("completed", "unrelated")]);
  await receipts.getByRole("button", { name: "刷新执行结果" }).click();
  await expect(receipts.locator(".onchain-execution-recovery")).toHaveCount(0);
  await expect(receipts).toContainText("全部腿已完成");
  await expect(page.locator(".onchain-submit-action")).toBeEnabled();
  const selector = receipts.getByRole("combobox", { name: "选择执行记录" });
  await selector.selectOption("run-unrelated");
  await selector.focus();
  fixture.tick();
  await expect(selector).toBeFocused();
  await expect(selector).toHaveValue("run-unrelated");
  expect(fixture.requests.filter((r) => r === "POST /api/onchain/execution/submit")).toHaveLength(1);
  expect(fixture.errors).toEqual([]);
});

test("lost execution reply recovers by build while an older in-flight read cannot erase the run", async ({ page }) => {
  const fixture = await setup(page, { execution: "lost_reply" });
  await page.goto(`${WEB}/#onchain`);
  const receipts = page.getByRole("region", { name: "执行结果", exact: true });
  await expect(receipts.locator(".onchain-execution-recovery")).toHaveCount(0);
  fixture.holdExecutionRead();
  await receipts.getByRole("button", { name: "刷新执行结果" }).click();
  await page.getByRole("button", { name: "构建交易计划", exact: true }).click();
  await page.locator(".onchain-submit-action").click();
  await expect(receipts).toContainText("执行反馈未确认");
  fixture.releaseExecutionRead();
  await expect(receipts.getByRole("button", { name: "刷新执行结果" })).toBeEnabled();
  await expect(receipts).toContainText("执行反馈未确认");
  await receipts.getByRole("button", { name: "刷新执行结果" }).click();
  await expect(receipts).toContainText("按顺序执行中");
  await expect(page.locator(".onchain-submit-action")).toBeDisabled();
  fixture.failExecutionRead();
  await receipts.getByRole("button", { name: "刷新执行结果" }).click();
  await expect(receipts).toContainText("执行记录刷新失败");
  await expect(receipts).toContainText("按顺序执行中");
  expect(fixture.requests.filter((r) => r === "POST /api/onchain/execution/submit")).toHaveLength(1);
  expect(fixture.errors).toEqual([]);
});

test("unread execution history blocks submission and explicit pre-write rejection permits rebuilding", async ({ page }) => {
  const fixture = await setup(page, { execution: "reject", failExecutionRead: true });
  await page.goto(`${WEB}/#onchain`);
  await page.getByRole("button", { name: "构建交易计划", exact: true }).click();
  await expect(page.locator(".onchain-submit-action")).toBeDisabled();
  const receipts = page.getByRole("region", { name: "执行结果", exact: true });
  await expect(receipts).toContainText("execution records unavailable");
  fixture.failExecutionRead(false);
  await receipts.getByRole("button", { name: "刷新执行结果" }).click();
  await page.locator(".onchain-submit-action").click();
  await expect(receipts).toContainText("plan expired");
  await expect(receipts.locator(".onchain-execution-recovery")).toHaveCount(0);
  await expect(page.locator(".onchain-submit-action")).toHaveCount(0);
  await page.getByRole("button", { name: "构建交易计划", exact: true }).click();
  await expect(page.locator(".onchain-submit-action")).toBeEnabled();
  expect(fixture.errors).toEqual([]);
});

test("batch markets preserve controls and details across quotes and mark a disconnected queue stale", async ({ page }, info) => {
  const fixture = await setup(page);
  const first = batchItem();
  const second = batchItem("fixture-other", "BTC");
  fixture.setBatchItems([first, second]);
  await page.goto(`${WEB}/#onchain`);
  await page.getByRole("tab", { name: "监控", exact: true }).click();
  const row = page.locator('tr[data-item-id="fixture-peer"]');
  const details = row.locator("details");
  await details.locator("summary").click();
  const load = row.getByRole("button", { name: "载入", exact: true });
  await load.focus();
  first.bestNetSpreadBps = 121;
  second.bestNetSpreadBps = 150;
  fixture.setBatchItems([first, second]);
  await expect(row.locator('[data-label="价差 / 提醒"]')).toContainText("+1.210%");
  await expect(load).toBeFocused();
  await expect(details).toHaveAttribute("open", "");
  const sidebar = page.getByRole("complementary", { name: "链上套利市场列表" });
  const market = sidebar.getByRole("button", { name: /ETH\/USDC/ });
  await market.focus();
  fixture.tick();
  await expect(market).toBeFocused();
  expect(await sidebar.locator('.onchain-market-row:not(.is-active) strong').allTextContents()).toEqual([
    "ETH/USDC", "BINANCE", "+1.210%", "BTC/USDC", "BINANCE", "+1.500%",
  ]);
  await sidebar.getByRole("searchbox").fill("ETH");
  await expect(sidebar.locator(".onchain-market-row.is-active")).toBeHidden();
  await expect(sidebar.locator(".onchain-market-row:not(.is-active)")).toHaveCount(1);
  await sidebar.getByRole("searchbox").fill("");
  fixture.failStream();
  await expect(row.locator(".onchain-state-badge")).toHaveText("报价已过期");
  await expect(page.locator(".onchain-batch-summary")).toContainText("费后机会待确认");
  await expect(sidebar.locator("footer")).toContainText("状态待确认");
  await expect(details).toHaveAttribute("open", "");
  fixture.tick();
  await expect(row.locator(".onchain-state-badge")).toHaveText("报价新鲜");
  await page.screenshot({ path: info.outputPath("batch-desktop.png") });
  await page.setViewportSize({ width: 390, height: 900 });
  await page.getByRole("navigation", { name: "链上套利工作区" }).getByRole("button", { name: "套利", exact: true }).click();
  await row.scrollIntoViewIfNeeded();
  expect(await page.locator(".onchain-page").evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBeTruthy();
  await page.screenshot({ path: info.outputPath("batch-mobile.png") });
  expect(fixture.requests.filter((r) => r.startsWith("PATCH"))).toEqual([]);
  expect(fixture.errors).toEqual([]);
});

test("batch market switches serialize saves and failed switches keep the applied market", async ({ page }) => {
  const fixture = await setup(page);
  fixture.setBatchItems([batchItem(), batchItem("fixture-other", "BTC")]);
  await page.goto(`${WEB}/#onchain`);
  await page.getByRole("tab", { name: "监控", exact: true }).click();
  const first = page.locator('tr[data-item-id="fixture-peer"]');
  const second = page.locator('tr[data-item-id="fixture-other"]');
  fixture.holdSave(true);
  await first.getByRole("button", { name: "载入", exact: true }).click();
  await expect(second.getByRole("button", { name: "载入", exact: true })).toBeDisabled();
  await expect(second.getByRole("button", { name: "移除", exact: true })).toBeDisabled();
  await expect(page.locator(".onchain-market-row:not(.is-active)").first()).toBeDisabled();
  fixture.releaseSave();
  await expect(page.locator(".onchain-config-problem")).toContainText("configuration not saved");
  await expect(page.locator(".onchain-market-row.is-active")).toContainText("SOL/USDC");
  fixture.holdSave();
  const latest = batchItem("fixture-other", "BTC");
  latest.config.slippageBps = 33;
  fixture.setBatchItems([batchItem(), latest]);
  const request = page.waitForRequest(`${API}/api/onchain/comparison/config`);
  await second.getByRole("button", { name: "载入", exact: true }).click();
  expect((await request).postDataJSON()).toMatchObject({ baseToken: "BTC", slippageBps: 33 });
  fixture.releaseSave();
  await expect(page.locator(".onchain-market-row.is-active")).toContainText("BTC/USDC");
  await expect(page.locator(".onchain-config-problem")).toHaveCount(0);
  expect(fixture.requests.filter((r) => r === "PATCH /api/onchain/comparison/config")).toHaveLength(2);
  expect(fixture.errors).toEqual([]);
});

test("missing cross-chain target stays explicit and can be disabled without silently replacing it", async ({ page }) => {
  const fixture = await setup(page, { scenario: "cross_chain" });
  await page.goto(`${WEB}/#onchain`);
  const select = page.getByRole("combobox", { name: "跨链目标市场" });
  const toggle = page.getByRole("checkbox", { name: "启用跨链完整流程监控" });
  await expect(select).toHaveValue("fixture-peer");
  await expect(select.locator("option:checked")).toHaveText("原目标已移除或与当前链相同");
  await expect(toggle).toBeChecked();
  await expect(toggle).toBeEnabled();
  await page.locator(".onchain-cross-chain-toggle").click();
  await expect(toggle).not.toBeChecked();
  await expect(toggle).toBeDisabled();
  const apply = page.getByRole("button", { name: "应用变更", exact: true });
  await expect(apply).toBeEnabled();
  const request = page.waitForRequest(`${API}/api/onchain/comparison/config`);
  await apply.click();
  expect((await request).postDataJSON().crossChain).toMatchObject({ enabled: false, peerItemId: "fixture-peer" });
  await expect(select).toBeEnabled();
  fixture.setBatchItems([batchItem("fixture-other", "BTC")]);
  await expect(select).toHaveValue("fixture-peer");
  await select.selectOption("fixture-other");
  await page.locator(".onchain-cross-chain-toggle").click();
  await expect(toggle).toBeChecked();
  await select.focus();
  fixture.tick();
  await expect(select).toBeFocused();
  await expect(select).toHaveValue("fixture-other");
  fixture.setBatchItems([]);
  await expect(select.locator("option:checked")).toHaveText("原目标已移除或与当前链相同");
  await expect(apply).toBeDisabled();
  await expect(toggle).toBeEnabled();
  await toggle.focus();
  await toggle.press("Space");
  await expect(toggle).not.toBeChecked();
  await select.selectOption("");
  await expect(select).toHaveValue("");
  expect(fixture.errors).toEqual([]);
});

test("cross-chain preview preserves controls and uses the current quote after stream recovery", async ({ page }) => {
  const fixture = await setup(page, { scenario: "cross_chain" });
  await page.goto(`${WEB}/#onchain`);
  const panel = page.getByRole("region", { name: "跨链完整流程监控" });
  const costs = panel.locator("details").filter({ hasText: "补库费用" });
  await costs.locator("summary").click();
  const button = panel.locator(".onchain-cross-chain-preview-action");
  await expect(button).toBeEnabled();
  await button.focus();
  fixture.tick();
  await expect(button).toBeFocused();
  await expect(costs).toHaveAttribute("open", "");
  fixture.failStream();
  await expect(button).toBeDisabled();
  await expect(panel.locator("header em")).toHaveText("已过期");
  fixture.tick();
  await expect(button).toBeEnabled();
  const request = page.waitForRequest(`${API}/api/onchain/cross-chain/build`);
  await button.click();
  expect((await request).postDataJSON().expectedQuoteObservedAtMs).toBe(NOW + 2);
  expect(fixture.errors).toEqual([]);
});

test("recovery amount survives receipt refresh and previews the latest run revision", async ({ page }, info) => {
  const fixture = await setup(page, { scenario: "cross_chain", crossRecovery: true });
  await page.goto(`${WEB}/#onchain`);
  const input = page.getByRole("textbox", { name: "本次处置数量" });
  await input.fill("3.25");
  const run = crossChainRun();
  run.updatedAtMs += 1_000;
  fixture.setCrossRows([run]);
  const response = page.waitForResponse((response) => new URL(response.url()).pathname === "/api/onchain/cross-chain/runs");
  await page.getByRole("button", { name: "刷新记录", exact: true }).evaluate((button: HTMLButtonElement) => button.click());
  await response;
  await expect(input).toHaveValue("3.25");
  await expect(input).toBeFocused();
  const request = page.waitForRequest(`${API}/api/onchain/cross-chain/recovery/preview`);
  await page.getByRole("button", { name: "核对余额与新报价" }).click();
  expect((await request).postDataJSON()).toMatchObject({ expectedRunUpdatedAtMs: NOW + 1_000, amountExact: "3.25" });
  await expect(page.locator(".cross-chain-recovery-quote-result")).toContainText("报价已核对 · 未锁定资金");
  await page.screenshot({ path: info.outputPath("recovery-preview-desktop.png") });
  await page.setViewportSize({ width: 390, height: 900 });
  await page.getByRole("navigation", { name: "链上套利工作区" }).getByRole("button", { name: "套利", exact: true }).click();
  await input.scrollIntoViewIfNeeded();
  expect(await page.locator(".cross-chain-recovery-quote-result dd").evaluateAll((values) => values.every((value) =>
    value.getBoundingClientRect().height <= Number.parseFloat(getComputedStyle(value).lineHeight) + 1))).toBeTruthy();
  expect(await page.locator(".onchain-page").evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBeTruthy();
  await page.screenshot({ path: info.outputPath("recovery-preview-mobile.png") });
  expect(fixture.requests.filter((r) => /\/(submit|reserve|authorize|cancel)$/.test(r))).toEqual([]);
  expect(fixture.errors).toEqual([]);
});

test("saved recovery plan expires without an active execution timer and blocks unread state", async ({ page }) => {
  const fixture = await setup(page, { scenario: "cross_chain", crossRecovery: true, savedRecovery: true });
  await page.goto(`${WEB}/#onchain`);
  const reserve = page.getByRole("button", { name: "确认计划并预留" });
  await expect(reserve).toBeEnabled();
  fixture.failCrossRead();
  await page.getByRole("button", { name: "刷新记录", exact: true }).click();
  await expect(page.getByText("运行记录读取失败，保留上次结果：", { exact: false })).toBeVisible();
  await expect(reserve).toBeDisabled();
  await expect(page.getByRole("button", { name: "取消计划 / 释放预留" })).toBeDisabled();
  fixture.failCrossRead(false);
  await page.getByRole("button", { name: "刷新记录", exact: true }).click();
  await expect(reserve).toBeEnabled();
  await page.clock.setFixedTime(NOW + 20_001);
  await expect(reserve).toBeDisabled();
  await expect(page.locator(".cross-chain-accounting-detail > summary")).toContainText(["已过期，未提交"]);
  expect(fixture.requests.filter((r) => /\/(submit|reserve|authorize|cancel)$/.test(r))).toEqual([]);
  expect(fixture.errors).toEqual([]);
});

test("configuration change discards a held recovery preview without forgetting the original run", async ({ page }) => {
  const fixture = await setup(page, { scenario: "cross_chain", crossRecovery: true, holdRecovery: true });
  await page.goto(`${WEB}/#onchain`);
  await page.getByRole("button", { name: "核对余额与新报价" }).click();
  await expect.poll(() => fixture.requests.filter((r) => r.endsWith("/recovery/preview")).length).toBe(1);
  await page.getByRole("button", { name: "暂停当前监控" }).click();
  await expect(page.locator(".onchain-rail-header-tools .read-only-flag")).toHaveText("已暂停");
  const response = page.waitForResponse(`${API}/api/onchain/cross-chain/recovery/preview`);
  fixture.releaseRecovery();
  await response;
  await expect(page.locator(".cross-chain-recovery-quote-result")).toHaveCount(0);
  await expect(page.locator(".cross-chain-run-heading")).toContainText("fixture-cross-run");
  expect(fixture.errors).toEqual([]);
});

test("recovery reservation and cancellation follow receipts without rolling back to an older plan", async ({ page }) => {
  const fixture = await setup(page, { scenario: "cross_chain", crossRecovery: true, savedRecovery: true, simulateRecoveryMutation: true });
  await page.goto(`${WEB}/#onchain`);
  const panel = page.getByRole("region", { name: "已保存处置计划" });
  const reserve = panel.getByRole("button", { name: "确认计划并预留" });
  await reserve.click();
  await expect(panel.locator("summary")).toContainText("已预留，未提交");
  await expect(reserve).toBeDisabled();
  fixture.setRecoveryPlans([recoveryPlan()]);
  await page.getByRole("button", { name: "刷新记录", exact: true }).click();
  await expect(panel.locator("summary")).toContainText("已预留，未提交");
  const current = recoveryPlan();
  current.status = "reserved";
  current.updatedAtMs += 1;
  fixture.setRecoveryPlans([current]);
  await panel.getByRole("button", { name: "取消计划 / 释放预留" }).click();
  await expect(panel.locator("summary")).toContainText("已取消");
  await panel.locator("summary").click();
  await expect(reserve).toBeDisabled();
  await expect(panel.getByRole("button", { name: "取消计划 / 释放预留" })).toBeDisabled();
  expect(fixture.requests.filter((r) => r.endsWith("/recovery/reserve"))).toHaveLength(1);
  expect(fixture.requests.filter((r) => r.endsWith("/recovery/cancel"))).toHaveLength(1);
  expect(fixture.requests.filter((r) => r.endsWith("/submit"))).toEqual([]);
  expect(fixture.errors).toEqual([]);
});

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
  await page.getByRole("button", { name: "生成完整流程预览", exact: true }).click();
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
  const buildLabel = scenario === "replenishment" ? "生成补仓计划" : "生成完整流程预览";
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

test("frozen onchain quotes age across duplicate frames and navigation", async ({ page }, info) => {
  await page.clock.install({ time: NOW });
  const fixture = await setup(page);
  await page.goto(`${WEB}/#onchain`);
  const quality = page.locator(".onchain-market-quality");
  const action = page.locator(".onchain-build-action");
  await expect(action).toBeEnabled();
  await page.getByLabel("统一对比资金 (USDC)").fill("123.45");
  await page.clock.fastForward(11_000);
  await expect(quality).toHaveText("报价过期");
  await expect(action).toBeDisabled();
  await expect(page.locator(".onchain-ticket-summary dd")).toHaveText(["--", "--"]);
  await expect(page.locator(".onchain-route-book-edge")).toContainText("上次测算");
  fixture.emit(snapshot());
  await expect(quality).toHaveText("报价过期");
  await page.locator('.module-tabs button[data-module="futures"]').click();
  await page.clock.fastForward(1_000);
  await page.locator('.module-tabs button[data-module="onchain"]').click();
  await expect(quality).toHaveText("报价过期");
  await expect(page.getByLabel("统一对比资金 (USDC)")).toHaveValue("123.45");
  fixture.emit(snapshot(NOW + 12_000));
  await expect(action).toBeEnabled();
  await expect(quality).toHaveText("可构建");
  await page.clock.setFixedTime(NOW - 60_000);
  await page.clock.fastForward(11_000);
  await expect(quality).toHaveText("报价过期");
  await expect(action).toBeDisabled();
  await page.screenshot({ path: info.outputPath("stale-onchain-desktop.png") });
  await page.setViewportSize({ width: 390, height: 900 });
  await page.getByRole("navigation", { name: "链上套利工作区" }).getByRole("button", { name: "套利", exact: true }).click();
  await action.scrollIntoViewIfNeeded();
  expect(await page.locator(".onchain-page").evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBeTruthy();
  await page.screenshot({ path: info.outputPath("stale-onchain-mobile.png") });
  const unknown = snapshot(NOW + 30_000);
  Object.assign(unknown, { cexFreshnessMs: null, cexObservedAtMs: null });
  fixture.emit(unknown);
  await expect(action).toBeDisabled();
  await expect(quality).toHaveText("时效待确认");
  await expect(page.locator(".onchain-market-freshness")).toContainText("未知");
  fixture.emit(snapshot(NOW + 31_000));
  await expect(action).toBeEnabled();
  const paused = snapshot(NOW + 32_000);
  paused.config.enabled = false;
  paused.quality = "disabled";
  fixture.emit(paused);
  await page.clock.fastForward(20_000);
  await expect(quality).toHaveText("已暂停");
  expect(fixture.requests.filter((r) => /\/(build|submit|authorize)$/.test(r))).toHaveLength(0);
  expect(fixture.errors).toEqual([]);
  expect(fixture.writes).toEqual([]);
});

test("onchain freshness keeps independent routes separate and rejects delayed baselines", async ({ page }) => {
  await page.clock.install({ time: NOW });
  const fixture = await setup(page, { holdSeed: true });
  await page.goto(`${WEB}/#onchain`);
  await expect.poll(() => fixture.requests.filter((r) => r === "GET /api/onchain/comparison").length).toBeGreaterThan(0);
  await page.clock.fastForward(11_000);
  const response = page.waitForResponse(`${API}/api/onchain/comparison`);
  fixture.releaseSeed();
  await response;
  await expect(page.locator(".onchain-build-action")).toBeDisabled();
  await expect(page.locator(".onchain-market-quality")).toHaveText("报价过期");
  const current = snapshot(NOW + 20_000);
  const cross = { provider: "lifi", peerItemId: "fixture-peer", peerChain: "base", quality: "evidence_pending", legs: [],
    atomic: false, previewReady: true, submitReady: true, quoteObservedAtMs: NOW + 20_000, observedAtMs: NOW + 20_000 };
  const dex = { primaryProvider: "jupiter_swap_v2", peerProvider: "fixture-peer", quality: "evidence_pending", routes: [],
    quoteObservedAtMs: NOW + 20_000, observedAtMs: NOW + 20_000 };
  Object.assign(current.config, { crossChain: { enabled: true, peerItemId: "fixture-peer", provider: "lifi", stablecoinRiskBps: 50 },
    dexComparison: { enabled: true, peerProvider: "fixture-peer" } });
  Object.assign(current, { cexObservedAtMs: NOW, cexFreshnessMs: 20_000, crossChain: cross, dexComparison: dex });
  const peer = batchItem("fixture-peer", "SOL");
  peer.config.maxAgeMs = 5_000;
  current.batch.items = [peer];
  fixture.emit(current);
  const crossBuild = page.locator(".onchain-cross-chain-preview-action");
  await expect(page.locator(".onchain-build-action")).toBeDisabled();
  await expect(crossBuild).toBeEnabled();
  const sources = page.locator(".onchain-market-source-age");
  await expect(sources.filter({ hasText: "链上 时效" })).toHaveClass(/is-positive/);
  await expect(sources.filter({ hasText: "交易所 时效" })).toHaveClass(/is-warning/);
  await page.clock.fastForward(6_000);
  await expect(crossBuild).toBeDisabled();
  current.observedAtMs = NOW + 26_000;
  Object.assign(current, { quoteObservedAtMs: NOW + 26_000, onchainFreshnessMs: 10,
    cexObservedAtMs: NOW + 26_000, cexFreshnessMs: 10 });
  Object.assign(dex, { quoteObservedAtMs: NOW, observedAtMs: NOW });
  Object.assign(cross, { quoteObservedAtMs: NOW + 26_000, observedAtMs: NOW + 26_000 });
  const conversion = { venue: "binance", symbol: "USDC/USD", source: "ws_push", cexQuote: "USD", onchainQuote: "USDC",
    sourceBid: 1, sourceAsk: 1, cexToOnchainBid: 1, cexToOnchainAsk: 1, cexToOnchainCapacity: 100, onchainToCexCapacity: 100,
    freshnessMs: 26_000, observedAtMs: NOW };
  Object.assign(current, { quoteConversion: conversion });
  fixture.emit(current);
  await expect(page.locator(".onchain-build-action")).toBeDisabled();
  await expect(page.locator(".onchain-execution-blocker")).toContainText("换汇报价已过期");
  await expect(crossBuild).toBeEnabled();
  await expect(page.locator(".onchain-dex-cross")).toContainText("报价已过期");
  Object.assign(conversion, { freshnessMs: 10, observedAtMs: NOW + 26_000 });
  Object.assign(peer, { observedAtMs: NOW + 26_000, quoteObservedAtMs: NOW + 26_000,
    cexObservedAtMs: NOW + 26_000, quoteConversion: { ...conversion, freshnessMs: 26_000 } });
  fixture.emit(current);
  await expect(page.locator(".onchain-build-action")).toBeEnabled();
  await page.getByRole("button", { name: "打开套利市场列表", exact: true }).click();
  await expect(page.locator(".onchain-market-row:not(.is-active)")).toContainText(/报价.*过期/);
  expect(fixture.requests.filter((r) => /\/(build|submit|authorize)$/.test(r))).toHaveLength(0);
  expect(fixture.errors).toEqual([]);
  expect(fixture.writes).toEqual([]);
});

test("direction changes discard stale execution previews and preserve receipts", async ({ page }, info) => {
  const fixture = await setup(page, { holdBuild: true });
  fixture.setExecutionRows([executionRun("completed", "previous-trade")]);
  await page.goto(`${WEB}/#onchain`);
  const directions = page.getByRole("group", { name: "价差与执行方向" });
  const buyChain = directions.getByRole("button", { name: /链买 → CEX 卖/ });
  const buyCex = directions.getByRole("button", { name: /CEX 买 → 链卖/ });
  const build = page.getByRole("button", { name: "构建交易计划", exact: true });
  const plan = page.getByRole("status", { name: "已构建交易计划" });
  const history = page.getByRole("region", { name: "执行结果" }).getByRole("combobox", { name: "选择执行记录" });
  await expect(history).toHaveValue("run-previous-trade");
  await expect(buyCex).toHaveAttribute("aria-pressed", "true");
  await build.click();
  await expect.poll(() => fixture.requests.filter((r) => r.endsWith("/execution/build")).length).toBe(1);
  await buyChain.click();
  await buyCex.click();
  const late = page.waitForResponse(`${API}/api/onchain/execution/build`);
  fixture.releaseBuild();
  await late;
  await expect(build).toBeEnabled();
  await expect(plan).toHaveCount(0);
  await build.click();
  await expect(plan).toBeVisible();
  await buyCex.click();
  await expect(plan).toBeVisible();
  await buyChain.click();
  await expect(plan).toHaveCount(0);
  await page.route(`${API}/api/onchain/execution/build`, (route) => route.fulfill({ json: executionPlan() }), { times: 1 });
  await build.click();
  await expect(page.getByRole("alert").filter({ hasText: "返回计划与所选方向不一致" })).toBeVisible();
  await expect(plan).toHaveCount(0);
  const rebuilt = page.waitForResponse(`${API}/api/onchain/execution/build`);
  await build.click();
  expect((await rebuilt).request().postDataJSON().direction).toBe("buy_onchain_sell_cex");
  await expect(plan).toBeVisible();
  await expect(plan.getByRole("list", { name: "交易计划执行顺序" })).toContainText("USDC → SOL");
  const submit = plan.getByRole("button", { name: "立即执行双腿", exact: true });
  await submit.focus();
  fixture.tick();
  await page.clock.setFixedTime(NOW + 1_000);
  await expect(plan).toContainText("19.0s");
  await expect(submit).toBeFocused();
  await expect(history).toHaveValue("run-previous-trade");
  await page.screenshot({ path: info.outputPath("direction-plan-desktop.png"), fullPage: true });
  await page.setViewportSize({ width: 390, height: 900 });
  await page.getByRole("navigation", { name: "链上套利工作区" }).getByRole("button", { name: "套利", exact: true }).click();
  await submit.scrollIntoViewIfNeeded();
  expect(await page.locator(".onchain-page").evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBeTruthy();
  const bounds = await submit.boundingBox();
  const tabs = await page.getByRole("navigation", { name: "链上套利工作区" }).boundingBox();
  expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(390);
  expect(bounds!.y).toBeGreaterThanOrEqual(tabs!.y + tabs!.height);
  expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(900);
  await page.screenshot({ path: info.outputPath("direction-plan-mobile.png") });
  await page.clock.setFixedTime(NOW + 21_000);
  await expect(plan.getByRole("button", { name: "计划已过期", exact: true })).toBeDisabled();
  expect(fixture.requests.filter((r) => r.endsWith("/execution/submit"))).toHaveLength(0);
  expect(fixture.errors).toEqual([]);
  expect(fixture.writes).toEqual([]);
});

test("direction changes invalidate approval and restock previews without losing transfers", async ({ page }) => {
  const fixture = await setup(page, { authorizedRun: true });
  let holdApproval = true;
  let wrongApproval = false;
  let releaseApproval: (() => void) | undefined;
  let approvalReads = 0;
  await page.route(`${API}/api/onchain/execution/build`, (route) => route.fulfill({ status: 409,
    json: { error: { code: "ONCHAIN_TOKEN_APPROVAL_REQUIRED", message: "fixture: approval required" } } }));
  await page.route(`${API}/api/onchain/token-approval/build`, async (route) => {
    approvalReads += 1;
    const direction = wrongApproval ? "buy_cex_sell_onchain" : route.request().postDataJSON().direction;
    const buyChain = direction === "buy_onchain_sell_cex";
    const result = { approvalId: "fixture-approval", direction, provider: "okx_dex", chain: "ethereum",
      walletAddress: "0x0000000000000000000000000000000000000001",
      tokenAddress: buyChain ? "0x0000000000000000000000000000000000000002" : "0x0000000000000000000000000000000000000003",
      tokenSymbol: buyChain ? "USDC" : "WETH", tokenDecimals: buyChain ? 6 : 18,
      spender: "0x0000000000000000000000000000000000000004",
      requiredAmountRaw: buyChain ? "100000000" : "1000000000000000000", currentAllowanceRaw: "0",
      transactions: [], builtAtMs: NOW, validUntilMs: NOW + 20_000, officialDocsUrl: "https://example.test/approval",
      approvalRequired: true, submitReady: true, blockers: [] };
    if (holdApproval) await new Promise<void>((resolve) => { releaseApproval = resolve; });
    return route.fulfill({ json: result });
  });
  await page.goto(`${WEB}/#onchain`);
  await expect.poll(() => fixture.sockets.size).toBeGreaterThan(0);
  const current = snapshot(NOW + 1);
  Object.assign(current.config, { chain: "ethereum", provider: "okx_dex", baseToken: "WETH", baseDecimals: 18,
    baseMint: "0x0000000000000000000000000000000000000003", quoteMint: "0x0000000000000000000000000000000000000002",
    cexSymbol: "ETH/USDC", baseAmountRaw: "1000000000000000000" });
  for (const row of current.executionReadiness.directions) Object.assign(row.cexInstrument,
    { requestedSymbol: "ETH/USDC", nativeSymbol: "ETHUSDC" });
  fixture.emit(current);
  const directions = page.getByRole("group", { name: "价差与执行方向" });
  const buyChain = directions.getByRole("button", { name: /链买 → CEX 卖/ });
  const buyCex = directions.getByRole("button", { name: /CEX 买 → 链卖/ });
  const build = page.getByRole("button", { name: "构建交易计划", exact: true });
  await expect(buyCex).toHaveAttribute("aria-pressed", "true");
  await build.click();
  await expect.poll(() => approvalReads).toBe(1);
  await expect(page.getByRole("button", { name: "核对授权中…", exact: true })).toBeDisabled();
  await buyChain.click();
  await buyCex.click();
  const late = page.waitForResponse(`${API}/api/onchain/token-approval/build`);
  holdApproval = false;
  releaseApproval?.();
  await late;
  await expect(build).toBeEnabled();
  await expect(page.getByRole("button", { name: "独立授权", exact: true })).toHaveCount(0);
  await build.click();
  await expect(page.getByRole("button", { name: "独立授权", exact: true })).toBeEnabled();
  await buyChain.click();
  await expect(page.getByRole("button", { name: "独立授权", exact: true })).toHaveCount(0);
  wrongApproval = true;
  await build.click();
  await expect(page.locator(".onchain-approval-result[role=alert]")).toContainText("返回计划与所选方向不一致");
  wrongApproval = false;
  await build.click();
  await expect(page.getByRole("button", { name: "独立授权", exact: true })).toBeEnabled();
  await expect(page.locator(".onchain-approval-amount")).toContainText("USDC");
  current.observedAtMs += 1;
  for (const row of current.executionReadiness.directions) Object.assign(row, { buildReady: false,
    path: { kind: "direct_two_leg", availability: "replenishable", summary: "fixture: 可补仓", legs: [], replenishment: [] } });
  fixture.emit(current);
  await buyCex.click();
  const restock = page.getByRole("button", { name: "生成补仓计划", exact: true });
  fixture.holdPlan();
  await restock.click();
  await expect.poll(() => fixture.requests.filter((r) => r.endsWith("/replenishment/build")).length).toBe(1);
  await buyChain.click();
  await buyCex.click();
  const latePlan = page.waitForResponse(`${API}/api/onchain/replenishment/build`);
  fixture.releasePlan();
  await latePlan;
  await expect(restock).toBeEnabled();
  await expect(page.getByRole("textbox", { name: "实盘补仓授权口令" })).toHaveCount(0);
  await restock.click();
  const confirmation = page.getByRole("textbox", { name: "实盘补仓授权口令" });
  await confirmation.fill("AUTHORIZE LIVE REPLENISHMENT");
  await expect(page.getByRole("button", { name: "授权 60 秒", exact: true })).toBeEnabled();
  await buyChain.click();
  await expect(confirmation).toHaveCount(0);
  await expect(page.locator(".onchain-replenishment-run")).toContainText("等待提交原资金动作");
  await restock.click();
  await expect(confirmation).toHaveValue("");
  await expect(page.getByRole("button", { name: "授权 60 秒", exact: true })).toBeDisabled();
  expect(fixture.requests.filter((r) => /\/(submit|authorize)$/.test(r))).toHaveLength(0);
  expect(fixture.errors).toEqual([]);
  expect(fixture.writes).toEqual([]);
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

test("a held build stays discarded after returning and permits a fresh preview", async ({ page }) => {
  const fixture = await setup(page, { holdBuild: true });
  await page.goto(`${WEB}/#onchain`);
  await page.getByRole("button", { name: "构建交易计划", exact: true }).click();
  await expect.poll(() => fixture.requests.filter((request) => request.includes("/execution/build")).length).toBe(1);
  await page.locator('.module-tabs button[data-module="futures"]').click();
  await page.locator('.module-tabs button[data-module="onchain"]').click();
  await expect(page.getByRole("button", { name: "构建交易计划", exact: true })).toBeEnabled();
  const response = page.waitForResponse(`${API}/api/onchain/execution/build`);
  fixture.releaseBuild();
  await response;
  await page.waitForTimeout(100);
  await expect(page.getByRole("status", { name: "已构建交易计划" })).toHaveCount(0);
  await page.getByRole("button", { name: "构建交易计划", exact: true }).click();
  await expect(page.getByRole("status", { name: "已构建交易计划" })).toBeVisible();
  expect(fixture.requests.filter((request) => request.includes("/execution/build"))).toHaveLength(2);
  expect(fixture.requests.filter((request) => request.includes("/execution/submit"))).toHaveLength(0);
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
  expect((await response).status()).toBe(400);
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

test("onchain draft and configuration saves survive navigation without duplicate writes", async ({ page }, info) => {
  const fixture = await setup(page);
  await page.goto(`${WEB}/#onchain`);
  const amount = page.getByLabel("统一对比资金 (USDC)");
  const flag = page.locator(".onchain-rail-header-tools .read-only-flag");
  await expect(amount).toHaveValue("100");
  await amount.fill("123.45");
  await page.locator('.module-tabs button[data-module="futures"]').click();
  await page.locator('.module-tabs button[data-module="onchain"]').click();
  await expect(amount).toHaveValue("123.45");
  fixture.holdSave(true);
  await page.getByRole("button", { name: "应用变更", exact: true }).click();
  await expect.poll(() => fixture.requests.filter((request) => request.startsWith("PATCH")).length).toBe(1);
  const reads = fixture.requests.filter((request) => request === "GET /api/onchain/comparison").length;
  await page.locator('.module-tabs button[data-module="futures"]').click();
  await page.locator('.module-tabs button[data-module="onchain"]').click();
  await expect(flag).toHaveText("处理中");
  await expect(amount).toBeDisabled();
  expect(fixture.requests.filter((request) => request === "GET /api/onchain/comparison")).toHaveLength(reads);
  fixture.tick();
  await expect(flag).toHaveText("处理中");
  await page.locator('.module-tabs button[data-module="futures"]').click();
  const failed = page.waitForResponse(`${API}/api/onchain/comparison/config`);
  fixture.releaseSave();
  expect((await failed).status()).toBe(400);
  await page.locator('.module-tabs button[data-module="onchain"]').click();
  await expect(page.getByRole("alert").filter({ hasText: "操作未完成" })).toContainText("fixture: configuration not saved");
  await expect(amount).toBeEnabled();
  await expect(amount).toHaveValue("123.45");
  fixture.holdSave();
  await page.getByRole("button", { name: "应用变更", exact: true }).click();
  await expect.poll(() => fixture.requests.filter((request) => request.startsWith("PATCH")).length).toBe(2);
  await page.locator('.module-tabs button[data-module="futures"]').click();
  const saved = page.waitForResponse(`${API}/api/onchain/comparison/config`);
  fixture.releaseSave();
  expect((await saved).status()).toBe(200);
  await page.locator('.module-tabs button[data-module="onchain"]').click();
  await expect(flag).toHaveText("监控中");
  await expect(amount).toHaveValue("123.45");
  fixture.emit(snapshot(NOW));
  await expect(flag).toHaveText("监控中");
  await expect(amount).toHaveValue("123.45");
  await page.setViewportSize({ width: 390, height: 900 });
  await page.getByRole("navigation", { name: "链上套利工作区" }).getByRole("button", { name: "接入", exact: true }).click();
  await expect(amount).toBeVisible();
  expect(await page.locator(".onchain-page").evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBeTruthy();
  await page.getByRole("complementary", { name: "链上套利监控配置" }).screenshot({ path: info.outputPath("onchain-restored-draft-390.png") });
  expect(fixture.requests.filter((request) => request.startsWith("PATCH"))).toHaveLength(2);
  expect(fixture.requests.filter((request) => request.includes("/execution/submit"))).toHaveLength(0);
  expect(fixture.errors).toEqual([]);
  expect(fixture.writes).toEqual([]);
});

test("onchain seed and refresh timeouts recover without applying drafts or late replies", async ({ page }, info) => {
  await page.clock.install({ time: NOW });
  const fixture = await setup(page, { holdSeed: true });
  let cancelledSeeds = 0;
  page.on("requestfailed", request => { if (new URL(request.url()).pathname === "/api/onchain/comparison") cancelledSeeds++; });
  await page.goto(`${WEB}/#onchain`);
  await expect.poll(() => fixture.requests.includes("GET /api/onchain/comparison")).toBe(true);
  await page.clock.fastForward(15_100);
  const flag = page.locator(".onchain-rail-header-tools .read-only-flag");
  await expect(flag).toHaveText("读取失败");
  await expect.poll(() => cancelledSeeds).toBeGreaterThan(0);
  const retry = page.getByRole("button", { name: "立即重试", exact: true });
  await expect(retry).toBeEnabled();
  let calls = 0;
  let hold = false;
  let release: (() => void) | undefined;
  let lateReleased = false;
  await page.route(`${API}/api/onchain/comparison/refresh`, async route => {
    calls++;
    if (hold) {
      await new Promise<void>(resolve => { release = resolve; });
      const late = snapshot(NOW + 99_000);
      late.config.quoteAmountRaw = "777000000";
      await route.fulfill({ json: late });
      lateReleased = true;
    } else await route.fulfill({ json: snapshot(NOW + calls * 31_000) });
  });
  await retry.click();
  await expect(flag).toHaveText("监控中");
  const amount = page.getByLabel("统一对比资金 (USDC)");
  await amount.fill("123.450001");
  fixture.failStream();
  await expect(retry).toBeEnabled();
  hold = true;
  await retry.click();
  await expect.poll(() => calls).toBe(2);
  await expect(amount).toBeDisabled();
  await page.clock.fastForward(15_100);
  await expect(retry).toBeEnabled();
  await expect(flag).toHaveText("状态待确认");
  await expect(amount).toBeEnabled();
  await expect(amount).toHaveValue("123.450001");
  await expect(page.locator(".onchain-snapshot-warning")).toContainText("15 秒");
  await expect(page.locator(".onchain-build-action")).toBeDisabled();
  await page.screenshot({ path: info.outputPath("onchain-refresh-timeout.png") });
  hold = false;
  await retry.click();
  await expect.poll(() => calls).toBe(3);
  await expect(flag).toHaveText("草稿待应用");
  fixture.releaseSeed();
  release!();
  await expect.poll(() => lateReleased).toBe(true);
  await expect(amount).toHaveValue("123.450001");
  await expect(flag).toHaveText("草稿待应用");
  await expect(page.locator(".onchain-market-copy")).toContainText("100 USDC");
  expect(fixture.requests.some(request => /PATCH|\/execution\/submit/.test(request))).toBe(false);
  expect(fixture.errors).toEqual([]);
  expect(fixture.writes).toEqual([]);
});

test("leaving an onchain refresh immediately unlocks the retained draft and discards its late reply", async ({ page }) => {
  const fixture = await setup(page);
  await page.goto(`${WEB}/#onchain`);
  const amount = page.getByLabel("统一对比资金 (USDC)");
  await expect(amount).toBeEnabled();
  await amount.fill("123.450001");
  let release: (() => void) | undefined;
  let replied = false;
  await page.route(`${API}/api/onchain/comparison/refresh`, async route => {
    await new Promise<void>(resolve => { release = resolve; });
    const late = snapshot(NOW + 99_000);
    late.config.quoteAmountRaw = "777000000";
    await route.fulfill({ json: late });
    replied = true;
  });
  fixture.failStream();
  await page.getByRole("button", { name: "立即重试", exact: true }).click();
  await expect(amount).toBeDisabled();
  await expect.poll(() => Boolean(release)).toBe(true);
  await page.getByRole("button", { name: "切换到设置", exact: true }).click();
  await expect(page.locator(".onchain-page")).toHaveCount(0);
  await page.getByRole("button", { name: /^切换到链上套利/ }).click();
  await expect(amount).toBeEnabled();
  await expect(amount).toHaveValue("123.450001");
  release!();
  await expect.poll(() => replied).toBe(true);
  await expect(amount).toHaveValue("123.450001");
  await expect(page.locator(".onchain-market-copy")).toContainText("100 USDC");
  expect(fixture.requests.some(request => /PATCH|\/execution\/submit/.test(request))).toBe(false);
  expect(fixture.errors).toEqual([]);
  expect(fixture.writes).toEqual([]);
});

test("long execution restrictions stay readable in quoted and awaiting markets", async ({ page }, info) => {
  const fixture = await setup(page);
  await page.goto(`${WEB}/#onchain`);
  await expect(page.locator(".onchain-build-action")).toBeEnabled();
  await expect.poll(() => fixture.sockets.size).toBeGreaterThan(0);
  const reason = "当前库存不足，需要等待对应网络到账，再读取交易所可用余额、链上余额和最新双向报价。"
    + "充提网络尚未完成核对，不能根据账户总资产推断本次可用数量。"
    + "请确认目标网络、地址和合约一致，补齐费用与盘口后重新构建；当前请求没有提交订单，也没有自动划转资金。";
  for (const awaiting of [false, true]) {
    const next = snapshot(NOW + (awaiting ? 200 : 100));
    fixture.emit(Object.assign(next, { comparisons: awaiting ? [] : next.comparisons,
      providerProblem: awaiting ? reason : null,
      executionReadiness: { ...next.executionReadiness,
        directions: next.executionReadiness.directions.map(row => ({ ...row, buildReady: false, blockers: [reason] })) } }));
    const blocker = page.locator(".onchain-primary-execution .onchain-execution-blocker");
    await expect(blocker).toHaveText(reason);
    for (const width of [1440, 1024, 390, 320]) {
      await page.setViewportSize({ width, height: 900 });
      expect(await blocker.evaluate(el => el.scrollHeight <= el.clientHeight + 1)).toBe(true);
      expect(await blocker.evaluate(el => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
      expect(await blocker.evaluate(el => parseFloat(getComputedStyle(el).fontSize))).toBeGreaterThanOrEqual(11);
      await page.locator(".onchain-build-action").scrollIntoViewIfNeeded();
      await expect(page.locator(".onchain-build-action")).toBeInViewport();
      await expect(page.locator(".onchain-build-action")).toBeDisabled();
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
      if (width === 1440 || width === 320) {
        await page.screenshot({ path: info.outputPath(`onchain-restriction-${awaiting}-${width}.png`) });
      }
    }
  }
  expect(fixture.requests.some(request => request.includes("/execution/build"))).toBe(false);
  expect(fixture.errors).toEqual([]);
  expect(fixture.writes).toEqual([]);
});

test("configuration controls fit the desktop and mobile workspaces", async ({ page }, info) => {
  const fixture = await setup(page);
  await page.goto(`${WEB}/#onchain`);
  await expect(page.locator(".onchain-rail-header-tools .read-only-flag")).toHaveText("监控中");
  for (const width of [1440, 1024, 390, 320]) {
    await page.setViewportSize({ width, height: 900 });
    if (width < 721) await page.getByRole("navigation", { name: "链上套利工作区" }).getByRole("button", { name: "接入", exact: true }).click();
    await expect(page.getByLabel("统一对比资金 (USDC)")).toBeVisible();
    const identity = page.locator(".onchain-market-identity-editor");
    const trade = page.locator(".onchain-market-trade-stack");
    const identityBox = await identity.boundingBox();
    const tradeBox = await trade.boundingBox();
    expect(identityBox!.y + identityBox!.height).toBeLessThanOrEqual(tradeBox!.y + 1);
    for (const label of await page.locator(".onchain-market-identity-editor > header strong, .onchain-market-trade-stack .workbench-field > span").all()) {
      expect(await label.evaluate(el => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
      expect(await label.evaluate(el => parseFloat(getComputedStyle(el).fontSize))).toBeGreaterThanOrEqual(11);
    }
    expect(await page.locator(".onchain-page").evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBeTruthy();
    expect(await page.locator(".onchain-dex-cross-control").evaluate((el) => el.getBoundingClientRect().height)).toBeLessThan(100);
    await page.screenshot({ path: info.outputPath(`onchain-${width}.png`), fullPage: true });
    if (width < 721) await page.getByRole("navigation", { name: "链上套利工作区" }).getByRole("button", { name: "套利", exact: true }).click();
    const balance = page.locator(".onchain-readiness-fact").filter({ hasText: "余额" });
    await expect(balance).toContainText("待核对");
    await expect(balance).not.toContainText("0/0");
    await expect(page.locator(".onchain-execution-blocker")).not.toContainText("全部执行数据依据已通过");
    await expect(page.locator(".onchain-execution-blocker")).toContainText("尚未下单");
    for (const fact of await page.locator(".onchain-readiness-fact dd").all()) {
      expect(await fact.evaluate(el => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
    }
    await page.locator(".onchain-build-action").click({ trial: true });
    await page.evaluate(() => window.scrollTo({ top: 0, behavior: "instant" }));
    if (width < 721) {
      const shell = await page.locator(".mod-topbar").boundingBox();
      const tabs = await page.getByRole("navigation", { name: "链上套利工作区" }).boundingBox();
      const tape = await page.locator(".onchain-market-tape").boundingBox();
      expect(shell!.y).toBe(0);
      expect(tabs!.y).toBeGreaterThanOrEqual(shell!.y + shell!.height - 1);
      expect(tape!.y).toBeGreaterThanOrEqual(tabs!.y + tabs!.height - 1);
    }
    await page.screenshot({ path: info.outputPath(`onchain-decision-${width}.png`), fullPage: true });
  }
  expect(fixture.errors).toEqual([]);
});
