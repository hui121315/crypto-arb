import { expect, test, type Page } from "@playwright/test";
import { executionFixture, openExecution } from "./fixtures/execution-workbench";
import { API, NOW } from "./fixtures/opportunity-workbench";

async function fixture(page: Page) {
  await page.clock.install({ time: NOW });
  const f = await executionFixture(page);
  const status = await (await page.request.get(`${API}/api/trading/status`)).json();
  Object.assign(status, { adapter: "mock", environment: "paper" });
  Object.assign(status.risk, { killSwitchActive: false, liveTradingEnabled: false });
  let failed = false;
  let reads = 0;
  await page.route(`${API}/api/trading/status`, async (route) => {
    reads++;
    return failed
      ? route.fulfill({ status: 503, json: { code: "STATUS_UNAVAILABLE", message: "fixture: status unavailable" } })
      : route.fulfill({ json: status });
  });
  return { ...f, status,
    failStatus: (value: boolean) => { failed = value; },
    poll: async () => {
      const before = reads;
      await page.clock.runFor(5_100);
      await expect.poll(() => reads).toBeGreaterThan(before);
    },
  };
}

test("execution mode round trips discard old previews and approvals without changing inputs", async ({ page }) => {
  const f = await fixture(page);
  await openExecution(page);
  const amount = page.getByRole("textbox", { name: "计划本金 USD", exact: true });
  const artifact = page.locator(".execution-artifact");
  const review = artifact.getByRole("checkbox");
  const submit = page.locator(".confirm-action.primary");
  const bar = page.locator(".execution-actionbar");
  await amount.fill("10.25");
  await expect.poll(() => f.builds.length).toBe(2);
  await page.getByRole("button", { name: "检查交易计划", exact: true }).click();
  await review.check();
  await expect(submit).toBeEnabled();
  f.holdValidation();
  await page.getByRole("button", { name: "检查交易计划", exact: true }).click();
  await expect.poll(() => f.validations.length).toBe(2);
  f.status.environment = "live";
  await f.poll();
  await expect(bar).toContainText("实盘写入未启用");
  await expect(submit).toBeDisabled();
  await expect(review).toHaveCount(0);
  const lateValidation = page.waitForResponse("**/execution-artifacts/validate");
  f.releaseValidation();
  await (await lateValidation).finished();
  await expect(submit).toBeDisabled();
  expect(f.builds).toHaveLength(2);

  f.status.environment = "paper";
  await f.poll();
  await expect.poll(() => f.builds.length).toBe(3);
  await expect(review).not.toBeChecked();
  await expect(submit).toBeDisabled();
  await expect(amount).toHaveValue("10.25");
  f.holdPreview();
  await page.getByRole("button", { name: "刷新预览", exact: true }).click();
  await expect.poll(() => f.previews.length).toBe(4);
  f.status.environment = "live";
  await f.poll();
  await expect(bar).toContainText("实盘写入未启用");
  f.status.environment = "paper";
  await f.poll();
  await expect.poll(() => f.builds.length).toBe(4);
  expect(f.builds.at(-1).ticketId).toBe("ticket-5");
  const latePreview = page.waitForResponse("**/fixture-perp_cross-BTC/preview");
  f.releasePreview();
  await (await latePreview).finished();
  await expect(artifact.locator(".execution-artifact-identifiers")).toContainText("ticket-5");
  await expect(artifact.locator(".execution-artifact-status")).toContainText("待校验");
  await expect(review).not.toBeChecked();
  await expect(amount).toHaveValue("10.25");
  expect(f.previews.slice(1).every((request) => request.capitalUsd === 10.25)).toBe(true);
  f.status.openOrderCount++;
  await f.poll();
  expect(f.previews).toHaveLength(5);
  expect(f.builds).toHaveLength(4);
  await page.setViewportSize({ width: 390, height: 844 });
  await artifact.scrollIntoViewIfNeeded();
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(390);
  await page.screenshot({ path: test.info().outputPath("execution-new-mode-unreviewed-mobile.png") });
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("unknown status, changed risk and mismatched preview mode never retain an executable ticket", async ({ page }) => {
  const f = await fixture(page);
  await openExecution(page);
  const bar = page.locator(".execution-actionbar");
  const artifactStatus = page.locator(".execution-artifact-status");
  const submit = page.locator(".confirm-action.primary");
  f.failStatus(true);
  await f.poll();
  await expect(bar).toContainText("执行环境与风控状态未确认或已过期");
  await expect(submit).toBeDisabled();
  expect(f.previews).toHaveLength(1);
  f.failStatus(false);
  await f.poll();
  await expect.poll(() => f.builds.length).toBe(2);
  f.setPreviewMode("live");
  f.status.risk.maxOrderNotional = 25.5;
  await f.poll();
  await expect(bar).toContainText("交易检查返回的执行环境与当前设置不一致");
  await expect(submit).toBeDisabled();
  expect(f.previews).toHaveLength(3);
  expect(f.builds).toHaveLength(2);
  await expect(page.locator(".toast-item")).toHaveCount(0);
  await bar.scrollIntoViewIfNeeded();
  await page.screenshot({ path: test.info().outputPath("execution-mode-mismatch-desktop.png") });
  f.setPreviewMode("dry_run");
  await page.getByRole("button", { name: "刷新预览", exact: true }).click();
  await expect(artifactStatus).toContainText("待校验");
  await expect.poll(() => f.builds.length).toBe(3);
  f.status.risk.killSwitchActive = true;
  await f.poll();
  await expect(bar).toContainText("紧急停止已开启");
  await expect(submit).toBeDisabled();
  expect(f.previews).toHaveLength(4);
  f.status.risk.killSwitchActive = false;
  await f.poll();
  await expect(artifactStatus).toContainText("待校验");
  await expect.poll(() => f.builds.length).toBe(4);
  await expect(page.locator(".execution-artifact").getByRole("checkbox")).not.toBeChecked();
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("expired preview replay stays expired across mode changes and module navigation", async ({ page }) => {
  const f = await fixture(page);
  await openExecution(page);
  const artifact = page.locator(".execution-artifact");
  const status = artifact.locator(".execution-artifact-status");
  const submit = page.locator(".confirm-action.primary");
  const refresh = page.getByRole("button", { name: "刷新预览", exact: true });
  const amount = page.getByRole("textbox", { name: "计划本金 USD", exact: true });
  const originalAmount = await amount.inputValue();
  await artifact.getByRole("button", { name: "检查交易计划", exact: true }).click();
  await artifact.getByRole("checkbox").check();
  await expect(submit).toBeEnabled();
  for (let n = 0; n < 7; n++) await f.poll();
  await expect(status).toContainText("已过期");
  const originalTicket = f.builds.at(-1).ticketId;
  f.status.environment = "live";
  await f.poll();
  await expect(page.locator(".execution-actionbar")).toContainText("实盘写入未启用");
  f.replayPreview();
  f.status.environment = "paper";
  await f.poll();
  await expect.poll(() => f.builds.length).toBe(2);
  expect(f.builds.at(-1).ticketId).toBe(originalTicket);
  await expect(status).toContainText("已过期");
  await expect(artifact.getByRole("button", { name: "检查交易计划", exact: true })).toBeDisabled();
  await expect(artifact.getByRole("checkbox")).not.toBeChecked();
  await expect(submit).toBeDisabled();
  await expect(amount).toHaveValue(originalAmount);

  f.setServerTime(NOW + 45_900);
  await refresh.click();
  await expect(status).toContainText("待校验");
  const freshTicket = f.builds.at(-1).ticketId;
  expect(freshTicket).not.toBe(originalTicket);
  await artifact.getByRole("button", { name: "检查交易计划", exact: true }).click();
  await artifact.getByRole("checkbox").check();
  await expect(submit).toBeEnabled();
  await page.getByRole("button", { name: "切换到复盘", exact: true }).click();
  for (let n = 0; n < 7; n++) await f.poll();
  f.replayPreview();
  await page.getByRole("button", { name: "切换到对冲执行", exact: true }).click();
  await expect.poll(() => f.builds.length).toBe(4);
  expect(f.builds.at(-1).ticketId).toBe(freshTicket);
  await expect(status).toContainText("已过期");
  await expect(page.locator(".execution-flow-current")).toContainText("报价已过期，请刷新预览");
  await expect(page.locator(".execution-flow-current")).not.toContainText("EXPIRED");
  await expect(submit).toBeDisabled();
  await expect(amount).toHaveValue(originalAmount);
  await artifact.scrollIntoViewIfNeeded();
  await page.screenshot({ path: test.info().outputPath("execution-expired-replay-desktop.png") });
  await page.setViewportSize({ width: 390, height: 844 });
  await refresh.click({ trial: true });
  await page.screenshot({ path: test.info().outputPath("execution-expired-replay-mobile.png") });
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});
