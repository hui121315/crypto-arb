import { expect, test } from "@playwright/test";
import { executionFixture, openExecution } from "./fixtures/execution-workbench";

test("backend preflight can adopt a newer bound snapshot without a duplicate preview", async ({ page }) => {
  const f = await executionFixture(page);
  f.rebindSnapshot("current-snapshot");
  await openExecution(page);
  expect(f.previews).toHaveLength(1);
  expect(f.previews[0].opportunitySnapshotId).toBe("futures-1");
  expect(f.builds[0].opportunitySnapshotId).toBe("current-snapshot");
  const capital = page.getByRole("textbox", { name: "计划本金 USD", exact: true });
  await capital.fill("10.25");
  await expect.poll(() => f.builds.length).toBe(2);
  expect(f.previews).toHaveLength(2);
  expect(f.previews[1].opportunitySnapshotId).toBe("current-snapshot");
  expect(f.previews[1].capitalUsd).toBe(10.25);
  await expect(capital).toHaveValue("10.25");
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});

test("a snapshot rebound that names another request cannot build an artifact", async ({ page }) => {
  const f = await executionFixture(page);
  f.rebindSnapshot("current-snapshot", "wrong-original-request");
  await page.goto("/#futures");
  await page.getByRole("button", { name: "构建新双腿", exact: true }).click();
  await expect(page.locator(".execution-actionbar")).toContainText("后端交易检查不属于当前机会快照");
  await expect(page.locator(".confirm-action.primary")).toBeDisabled();
  expect(f.builds).toEqual([]);
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});

test("snapshot recovery keeps the user's amount and cannot reuse an old confirmation", async ({ page }) => {
  const f = await executionFixture(page);
  await openExecution(page);
  await page.getByRole("button", { name: "校验票据" }).click();
  await page.locator(".execution-artifact").getByRole("checkbox").check();
  let stale = true;
  await page.route("**/api/arbitrage/opportunities/*/preview", async (route) => {
    if (!stale) return route.fallback();
    stale = false;
    return route.fulfill({ status: 409, json: { error: { code: "OPPORTUNITY_SNAPSHOT_STALE",
      message: "fixture: snapshot advanced", details: { actualSnapshotId: "futures-2" } } } });
  });
  const capital = page.getByRole("textbox", { name: "计划本金 USD", exact: true });
  await capital.fill("10.25");
  await expect.poll(() => f.previews.length).toBe(2);
  expect(f.previews[1].opportunitySnapshotId).toBe("futures-2");
  expect(f.previews[1].capitalUsd).toBe(10.25);
  expect(f.previews[1].longNotionalUsd).toBe(10.25 * f.previews[1].leverage);
  expect(f.previews[1].shortNotionalUsd).toBe(10.25 * f.previews[1].leverage);
  await expect(capital).toHaveValue("10.25");
  await expect(page.locator(".execution-artifact-status")).toContainText("待校验");
  await expect(page.locator(".execution-artifact").getByRole("checkbox")).not.toBeChecked();
  await expect(page.locator(".confirm-action.primary")).toBeDisabled();
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});

test("refresh while the same preview is pending coalesces and does not hang", async ({ page }) => {
  const f = await executionFixture(page);
  await openExecution(page);
  f.holdPreview();
  const refresh = page.getByRole("button", { name: "刷新预览", exact: true });
  await refresh.click();
  await expect.poll(() => f.previews.length).toBe(2);
  await refresh.click();
  const response = page.waitForResponse("**/fixture-perp_cross-BTC/preview");
  f.releasePreview();
  await (await response).finished();
  await expect.poll(() => f.builds.length, { timeout: 2500 }).toBe(2);
  await expect(page.locator(".execution-artifact-status")).toContainText("待校验");
  expect(f.previews).toHaveLength(2);
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});

test("continuously changing snapshots stop after two automatic retries and can be refreshed manually", async ({ page }) => {
  const f = await executionFixture(page);
  await openExecution(page);
  let count = 0, stable = false;
  await page.route("**/api/arbitrage/opportunities/*/preview", async (route) => {
    if (stable) return route.fallback();
    count++;
    return route.fulfill({ status: 409, json: { error: { code: "OPPORTUNITY_SNAPSHOT_STALE",
      message: "fixture: changing snapshot", details: { actualSnapshotId: `changed-${count}` } } } });
  });
  const capital = page.getByRole("textbox", { name: "计划本金 USD", exact: true });
  await capital.fill("10.25");
  await expect(page.locator(".execution-actionbar")).toContainText("已停止自动重试");
  expect(count).toBe(3);
  await expect(capital).toHaveValue("10.25");
  await expect(page.locator(".confirm-action.primary")).toBeDisabled();
  await page.setViewportSize({ width: 390, height: 844 });
  await page.getByRole("button", { name: "刷新预览", exact: true }).click({ trial: true });
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(390);
  await page.screenshot({ path: test.info().outputPath("snapshot-retry-stopped-mobile.png"), fullPage: true });
  stable = true;
  await page.getByRole("button", { name: "刷新预览", exact: true }).click();
  await expect.poll(() => f.builds.length).toBe(2);
  expect(f.previews[1].capitalUsd).toBe(10.25);
  expect(f.previews[1].opportunitySnapshotId).toBe("changed-2");
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});

test("scanner handoff uses the current selected market and refreshed prices reset a previous draft", async ({ page }) => {
  const f = await executionFixture(page);
  await page.goto("/#opportunities");
  await page.getByRole("table", { name: "机会扫描候选", exact: true })
    .getByRole("button", { name: "构建对冲", exact: true }).first().click();
  await expect(page.locator(".execution-artifact-status")).toContainText("待校验");
  expect(f.previews[0].opportunityId).toBe(f.rows[0].id);
  expect(f.previews[0].longPrice).toBe(f.rows[0].longLeg.price);
  expect(f.previews[0].shortPrice).toBe(f.rows[0].shortLeg.price);
  await expect(page.locator(".execution-ticket")).toContainText("机会扫描");
  await page.getByRole("textbox", { name: "计划本金 USD", exact: true }).fill("10.25");
  await expect.poll(() => f.builds.length).toBe(2);
  await page.getByRole("button", { name: "切换到期货套利", exact: true }).click();
  await expect.poll(() => f.sockets.size).toBeGreaterThan(0);
  f.tick();
  await expect(page.locator(".futures-data-row").first()).toContainText("60000.5");
  await page.getByRole("button", { name: "构建新双腿", exact: true }).click();
  await expect.poll(() => f.builds.length).toBe(3);
  expect(f.previews[2].longPrice).toBe(60000.5);
  expect(f.previews[2].capitalUsd).toBe(f.previews[0].capitalUsd);
  await expect(page.locator(".execution-ticket")).toContainText("期货套利");
  await expect(page.locator(".execution-artifact").getByRole("checkbox")).not.toBeChecked();
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});
