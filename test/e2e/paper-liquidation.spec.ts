import { test, expect } from "@playwright/test";

const API = "http://127.0.0.1:18000";
const WEB = "http://127.0.0.1:18080";

test("paper liquidation closes both paused legs from one at-risk leg and links its exact review", async ({ page, request }) => {
  test.skip(process.env.CROSSLINE_E2E_LIQUIDATION !== "1", "Requires the explicit synthetic liquidation fixture");
  const headers = { Authorization: "Bearer isolated-paper-browser" };
  const errors: string[] = [], unexpected: string[] = [];
  const read = async (path: string) => {
    const response = await request.get(API + path, { headers });
    expect(response.ok()).toBe(true);
    return response.json();
  };
  const readStatus = () => read("/api/automation/status");
  const readPortfolio = () => read("/api/trading/portfolio/snapshot");
  const setMarket = async (phase: string) => {
    const response = await request.post(API + "/__paper/market", {
      headers: { ...headers, "Content-Type": "application/json" }, data: JSON.stringify(phase),
    });
    expect(response.status()).toBe(204);
  };
  const trading = await read("/api/trading/status");
  expect(trading.environment).toBe("paper"); expect(trading.adapter).toBe("mock");
  expect((await readStatus()).config.enabled).toBe(false);
  await expect.poll(async () => (await readPortfolio()).source).toBe("isolated_paper_liquidation_fixture");
  expect((await request.post(API + "/__paper/market", {
    headers: { "Content-Type": "application/json" }, data: JSON.stringify("liquidation_near"),
  })).status()).toBe(401);
  await page.addInitScript((api) => {
    localStorage.setItem("api_base", JSON.stringify(api));
    localStorage.setItem("api_auth_token", JSON.stringify("isolated-paper-browser"));
  }, API);
  page.on("pageerror", (error) => errors.push(error.stack ?? error.message));
  await page.route("**/*", (route) => {
    const req = route.request(), url = new URL(req.url());
    if (![API, WEB].includes(url.origin)) { unexpected.push(req.url()); return route.abort(); }
    // No browser close endpoint is allowed: only the actual protection worker can close.
    if (!["GET", "HEAD"].includes(req.method()) && ![
      "/api/auth/ws-ticket", "/api/trading/risk-config", "/api/automation/config", "/api/automation/control",
    ].includes(url.pathname)) { unexpected.push(req.method() + " " + url.pathname); return route.abort(); }
    return route.continue();
  });
  await page.goto("/#automation");
  await page.locator(".automation-entry-config > summary").click();
  await page.getByLabel("资金 (USD)", { exact: true }).fill("12.75");
  await page.getByLabel("入场冷却 (秒)", { exact: true }).fill("1");
  await page.getByRole("button", { name: "保存门槛", exact: true }).click();
  await expect(page.locator(".automation-action-notice")).toHaveText("自动化配置已保存");
  await page.getByRole("checkbox", { name: /^单腿强平保护/ }).check();
  await page.getByLabel("退出距离 %", { exact: true }).fill("7.5");
  await page.getByRole("button", { name: "保存退出保护", exact: true }).click();
  await expect(page.locator(".automation-protection-message")).toHaveText("退出保护已保存");
  const protection = (await read("/api/trading/status")).risk.autoProfitClose;
  expect(protection.enabled).toBe(false); expect(protection.stopLossEnabled).toBe(false);
  expect(protection.liquidationGuardEnabled).toBe(true);
  expect(protection.liquidationExitDistancePct).toBe(7.5);
  expect(protection.confirmationSamples).toBeGreaterThanOrEqual(2);
  await page.getByRole("button", { name: "启动模拟自动化", exact: true }).click();
  await expect.poll(async () => (await readStatus()).recentDecisions.filter((row: any) => row.kind === "submitted").length).toBe(1);
  const runId = (await readStatus()).recentDecisions.find((row: any) => row.kind === "submitted").executionRunId;
  const readReceipt = () => read("/api/automation/execution-runs/" + encodeURIComponent(runId));
  await page.getByRole("button", { name: "暂停模拟新入场", exact: true }).click();
  await expect(page.locator(".automation-command-status strong")).toHaveText("已暂停");
  await expect.poll(async () => (await readReceipt()).run.state).toBe("hedged");

  for (const phase of ["baseline", "liquidation_safe"]) {
    await setMarket(phase);
    const observed = new Set<number>();
    await expect.poll(async () => {
      const envelope = await readPortfolio(), snapshot = envelope.snapshot;
      expect(envelope.source).toBe("isolated_paper_liquidation_fixture");
      if (snapshot.positions.length !== 2) return observed.size;
      const long = snapshot.positions.find((row: any) => row.venue === "binance");
      const short = snapshot.positions.find((row: any) => row.venue === "okx");
      expect(short.liquidationDistancePct).toBeNull();
      const expected = phase === "baseline" ? long.liquidationDistancePct == null
        : Math.abs(long.liquidationDistancePct - 20) < 0.001;
      if (expected) observed.add(snapshot.serverNowMs);
      const receipt = await readReceipt();
      expect(receipt.run.state).toBe("hedged");
      expect(receipt.closeRuns).toHaveLength(0);
      return observed.size;
    }, { timeout: 8_000, intervals: [200] }).toBeGreaterThanOrEqual(protection.confirmationSamples + 1);
  }

  await setMarket("liquidation_near");
  await expect.poll(async () => (await readReceipt()).run.state, { timeout: 15_000 }).toBe("closed");
  const receipt = await readReceipt(), close = receipt.closeRuns[0];
  expect(receipt.mode).toBe("dry_run"); expect(receipt.closeRuns).toHaveLength(1);
  expect(close.status).toBe("succeeded");
  expect(close.reason).toContain("auto_pair_exit trigger=liquidation_guard");
  expect(close.idempotencyKey).toContain("auto-pair-exit:" + runId + ":liquidation_guard:");
  expect(close.legs).toHaveLength(2);
  expect(close.legs.every((leg: any) => leg.status === "filled" && leg.order.intent.mode === "dry_run"
    && leg.pairEvidence.runId === runId)).toBe(true);
  await expect.poll(async () => (await readPortfolio()).snapshot.positions.length).toBe(0);
  await page.getByRole("tab", { name: "交易记录", exact: true }).click();
  const panel = page.getByRole("region", { name: "自动化交易记录", exact: true });
  await expect(panel).toContainText(runId);
  await panel.locator(".automation-close-receipt summary").click();
  await expect(panel).toContainText("退出原因：强平距离保护");
  await expect(panel).toContainText("本次平仓已成交");
  await page.screenshot({ path: test.info().outputPath("liquidation-exit-receipt.png"), fullPage: true });
  await panel.getByRole("link", { name: "关联持仓", exact: true }).click();
  await expect(page.locator(".positions-run-scope")).toContainText(runId);
  await expect(page.locator(".positions-table .row-close-button")).toHaveCount(0);
  await page.goto("/#automation");
  await expect(page.locator(".automation-command-status strong")).toHaveText("已暂停");
  await page.getByRole("tab", { name: "交易记录", exact: true }).click();
  await panel.getByRole("link", { name: "关联复盘", exact: true }).click();
  await expect(page.getByRole("tabpanel", { name: /^执行记录/ }).locator(".review-record-scope")).toContainText(runId);
  await expect.poll(async () => {
    const review = await read("/api/review/executed?runId=" + encodeURIComponent(runId) + "&days=365");
    return review.rows.some((row: any) => row.evidence?.closeRunEvidence?.some(
      (e: any) => e.runId === runId && e.closeRunId === close.id));
  }).toBe(true);
  await page.goto("/#automation");
  await page.getByRole("button", { name: "立即急停", exact: true }).click();
  await expect(page.locator(".automation-command-status strong")).toHaveText("已关闭");
  await page.getByRole("tab", { name: "交易记录", exact: true }).click();
  await panel.locator(".automation-close-receipt summary").click();
  await page.setViewportSize({ width: 390, height: 844 });
  await panel.getByRole("link", { name: "关联复盘", exact: true }).click({ trial: true });
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(390);
  await page.evaluate(() => window.scrollTo({ top: 0, behavior: "instant" }));
  await page.screenshot({ path: test.info().outputPath("liquidation-exit-mobile.png"), fullPage: true });
  expect((await readReceipt()).closeRuns).toHaveLength(1);
  expect((await readStatus()).recentDecisions.filter((row: any) => row.kind === "submitted")).toHaveLength(1);
  expect(errors).toEqual([]); expect(unexpected).toEqual([]);
});
