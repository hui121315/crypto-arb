import { expect, test } from "@playwright/test";
import { submissionFixture, reviewAndSubmit, confirmRecoveryKey } from "./fixtures/execution-submission";
import { API, NOW } from "./fixtures/opportunity-workbench";

for (const stalled of ["run", "journal"] as const) {
  test(`a stalled ${stalled} read times out without losing or resending the original submission`, async ({ page }) => {
    await page.clock.install({ time: NOW });
    const f = await submissionFixture(page);
    f.setMode("timeout");
    f.holdConfirm();
    await reviewAndSubmit(page);
    await expect.poll(() => f.confirms.length).toBe(1);

    const path = stalled === "run" ? "/api/trading/execution-runs" : "/api/trading/action-runs";
    let held = false, hold = true, aborted = false;
    let release: (() => void) | undefined;
    const original = await (await page.request.get(`${API}${path}`)).json();
    const empty = stalled === "run"
      ? { ...original, rows: [], problems: [], status: "fresh", page: { ...original.page, returnedCount: 0, totalRows: 0 } }
      : { ...original, data: [] };
    page.on("requestfailed", (request) => {
      if (new URL(request.url()).pathname === path) aborted = true;
    });
    await page.route(`${API}${path}*`, async (route) => {
      if (!hold) return route.fallback();
      hold = false;
      held = true;
      await new Promise<void>((resolve) => { release = resolve; });
      await route.fulfill({ json: empty });
    });
    f.releaseConfirm();
    await expect.poll(() => held).toBe(true);
    await expect(page.locator(".execution-actionbar")).toContainText("提交结果待核对");
    await expect(page.locator(".execution-flow-current")).toContainText("原提交待核对");
    await expect(page.locator(".execution-flow-overview")).not.toContainText("尚未提交");
    await expect(page.locator(".queue-overview")).toContainText("原提交待核对");
    await expect(page.locator(".queue-overview")).not.toContainText("尚未创建");
    await expect(page.locator(".queue-count")).toHaveText("待确认");
    const saved = await page.evaluate((key) => localStorage.getItem(key), confirmRecoveryKey());
    expect(saved).not.toBeNull();
    const reads = f.reads.length;
    await page.clock.runFor(15_100);
    await expect(page.locator(".execution-page")).toContainText("核对原执行超过 15 秒", { timeout: 1800 });
    await expect(page.locator(".execution-status-bar .execution-section-head")).toContainText("原提交待核对");
    await expect.poll(() => aborted).toBe(true);
    await expect(page.locator(".confirm-action.primary")).toBeDisabled();
    expect(await page.evaluate((key) => localStorage.getItem(key), confirmRecoveryKey())).toBe(saved);
    expect(f.reads).toHaveLength(reads);
    expect(f.confirms).toHaveLength(1);
    await page.setViewportSize({ width: 390, height: 844 });
    const query = page.getByRole("button", { name: "查询提交结果", exact: true });
    await query.scrollIntoViewIfNeeded();
    await expect(query).toBeInViewport();
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(390);
    await page.screenshot({ path: test.info().outputPath(`${stalled}-read-timeout-mobile.png`) });

    if (stalled === "run") {
      f.setRuns([f.makeRun(undefined, "hedged", NOW + 30)]);
    } else {
      await page.reload();
      await expect(page.locator(".execution-flow-current")).toContainText("原提交待核对");
      await expect(page.locator(".execution-ticket")).toHaveCount(0);
      expect(await page.evaluate((key) => localStorage.getItem(key), confirmRecoveryKey())).toBe(saved);
      f.setActions([{ id: "recovered-rejection", kind: "hedge_confirm", status: "failed", actor: "fixture",
        target: "fixture-perp_cross-BTC", idempotencyKey: f.confirms[0].idempotencyKey,
        message: "fixture journal rejection", startedAtMs: NOW, updatedAtMs: NOW + 30,
        problem: { code: "HEDGE_PRE_TRADE_REJECTED", message: "fixture journal rejection" } }]);
    }
    await query.click();
    const recovered = stalled === "run" ? "双腿成交已确认" : "已核实：下单前被拒绝";
    await expect(page.locator(".execution-actionbar")).toContainText(recovered);
    expect(await page.evaluate((key) => localStorage.getItem(key), confirmRecoveryKey())).toBeNull();
    release?.();
    await page.clock.runFor(100);
    await expect(page.locator(".execution-actionbar")).toContainText(recovered);
    await expect(page.locator(".execution-page")).not.toContainText("核对原执行超过 15 秒");
    await page.setViewportSize({ width: 1280, height: 900 });
    await page.locator(".execution-actionbar").scrollIntoViewIfNeeded();
    const overviewBox = await page.locator(".execution-flow-overview").boundingBox();
    expect(overviewBox!.height).toBeLessThan(85);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(1280);
    await page.screenshot({ path: test.info().outputPath(`${stalled}-read-recovered-desktop.png`) });
    expect(f.confirms).toHaveLength(1);
    expect(f.errors).toEqual([]);
    expect(f.writes).toEqual([]);
  });
}
