import { expect, test } from "@playwright/test";

const API = "http://127.0.0.1:18000";
const WEB = "http://127.0.0.1:18080";
const headers = { Authorization: "Bearer isolated-paper-browser" };

test("native recovery fills leave real residuals and only close after all quantities match", async ({ page, request }, info) => {
  const errors: string[] = [], unexpected: string[] = [];
  await page.addInitScript(api => {
    localStorage.setItem("api_base", JSON.stringify(api));
    localStorage.setItem("api_auth_token", JSON.stringify("isolated-paper-browser"));
  }, API);
  page.on("pageerror", error => errors.push(error.message));
  await page.route("**/*", route => {
    const req = route.request(), url = new URL(req.url());
    if (![API, WEB].includes(url.origin) || (!["GET", "HEAD"].includes(req.method()) && url.pathname !== "/api/auth/ws-ticket")) {
      unexpected.push(`${req.method()} ${url.origin}${url.pathname}`); return route.abort();
    }
    return route.continue();
  });
  const update = async (quantity: number | null, second = false, save_stale = false) => {
    const response = await request.post(`${API}/__paper/execution-recovery`, { headers, data: { quantity, second, save_stale } });
    expect(response.ok()).toBe(true); return response.json();
  };
  const pending = await update(null);
  expect(pending.state).toBe("unwind_required");
  await page.goto(`/#execution?${new URLSearchParams({ run: pending.runId, ticket: pending.ticketId, opp: pending.opportunityId })}`);
  const queue = page.locator(".queue-overview-copy > strong");
  await expect(queue).toHaveText("需要反向处理");
  const runtime = page.locator(".execution-runtime-disclosure");
  if (await runtime.getAttribute("open") === null) await runtime.locator(":scope > summary").click();
  await expect(runtime).toContainText("未对冲金额 待核对");
  const partial = await update(0.1, false, true);
  expect(partial.state).toBe("unwind_required");
  expect(partial.netExposureUsd).toBeCloseTo(30);
  await expect(runtime).toContainText("未对冲金额 $30");
  await expect(queue).not.toHaveText("持仓已处理完");
  const next = await update(0.2, true, true);
  expect(next.netExposureUsd).toBeCloseTo(10);
  await expect(runtime).toContainText("未对冲金额 $10");
  await expect(queue).not.toHaveText("持仓已处理完");
  for (let retry = 0; retry < 2; retry++) {
    const result = await update(0.3, true, true);
    expect(result.state).toBe("closed"); expect(result.netExposureUsd).toBe(0);
    expect(result.longLeg.filledQuantity).toBe(0.4);
    expect(result.evidence.recoveryOrders).toHaveLength(2);
    await expect(queue).toHaveText("持仓已处理完");
  }
  await page.reload();
  await expect(queue).toHaveText("持仓已处理完");
  const flow = page.locator(".execution-flow-overview");
  await flow.locator("summary").click();
  await expect(flow).toContainText("本次剩余持仓已处理完");
  await expect(flow).toContainText("原交易与补救成交数量已核对");
  await expect(flow).not.toContainText("原订单和补救操作的结果待核对");
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await flow.scrollIntoViewIfNeeded();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    await page.screenshot({ path: info.outputPath(`recovery-${width}.png`) });
  }
  expect(errors).toEqual([]); expect(unexpected).toEqual([]);
});

test("native cancellation outcomes reach the browser without erasing partial fills", async ({ page, request }, info) => {
  const errors: string[] = [], unexpected: string[] = [];
  await page.addInitScript(api => {
    localStorage.setItem("api_base", JSON.stringify(api));
    localStorage.setItem("api_auth_token", JSON.stringify("isolated-paper-browser"));
  }, API);
  page.on("pageerror", error => errors.push(error.message));
  await page.route("**/*", route => {
    const req = route.request(), url = new URL(req.url());
    if (![API, WEB].includes(url.origin) || (!["GET", "HEAD"].includes(req.method()) && url.pathname !== "/api/auth/ws-ticket")) {
      unexpected.push(`${req.method()} ${url.origin}${url.pathname}`); return route.abort();
    }
    return route.continue();
  });
  for (const scenario of ["zero", "partial", "balanced", "unknown"]) {
    const update = async (complete: boolean) => {
      const response = await request.post(`${API}/__paper/execution-cancel`, { headers, data: { scenario, complete } });
      expect(response.ok()).toBe(true);
      return response.json();
    };
    const pending = await update(false);
    expect(pending.state).toBe("unwind_required");
    expect(pending.shortLeg.filledQuantity).toBeNull();
    const params = new URLSearchParams({ run: pending.runId, ticket: pending.ticketId, opp: pending.opportunityId });
    await page.goto(`/#execution?${params}`);
    const queue = page.locator(".queue-overview-copy > strong");
    await expect(queue).toHaveText("需要反向处理");
    await expect(page.locator(".execution-history-context[role='status']")).toContainText("不创建新交易计划");
    const flow = page.locator(".execution-flow-overview");
    await flow.locator("summary").click();
    const runtime = page.locator(".execution-runtime-disclosure");
    if (await runtime.getAttribute("open") === null) await runtime.locator(":scope > summary").click();
    await expect(runtime).toContainText("成交数量待核对");
    await expect(runtime).toContainText("未对冲金额 待核对");
    const result = await update(true);
    if (scenario === "zero") {
      expect(result.state).toBe("failed_safe"); expect(result.recoveryAction).toBeNull();
      expect(result.longLeg.filledQuantity).toBe(0); expect(result.shortLeg.filledQuantity).toBe(0);
      await expect(queue).toHaveText("订单已结束，未成交");
      await expect(flow).toContainText("本次没有新增持仓，无需平仓");
      await expect(flow).not.toContainText("两边订单的最终结果已确认");
      await expect(runtime).not.toContainText("需要人工复核");
    } else {
      expect(result.state).toBe("unwind_required");
      expect(result.netExposureUsd).toBe(scenario === "partial" ? 40 : 0);
      expect(result.recoveryAction).toBe(scenario === "partial" ? "unwind_long_leg" : "manual_review");
      await expect(runtime.locator(".execution-status-bar > p")).toHaveText(result.statusReason);
      await expect(flow).not.toContainText("无需平仓");
      if (scenario !== "unknown") expect(result.longLeg.filledQuantity).toBe(0.4);
      if (scenario === "partial") await expect(runtime).toContainText("未对冲金额 $40");
    }
    const persisted = await (await request.get(`${API}/api/trading/execution-runs?runId=${result.runId}`, { headers })).json();
    expect(persisted.rows[0].state).toBe(result.state);
    expect(persisted.rows[0].longLeg.filledQuantity).toBe(result.longLeg.filledQuantity);
    await page.reload();
    await expect(queue).toHaveText(scenario === "zero" ? "订单已结束，未成交" : "需要反向处理");
    if (scenario === "zero" || scenario === "partial") {
      if (await flow.locator("details").getAttribute("open") === null) await flow.locator("summary").click();
      if (await runtime.getAttribute("open") === null) await runtime.locator(":scope > summary").click();
      for (const width of [1440, 390]) {
        await page.setViewportSize({ width, height: 900 });
        await flow.scrollIntoViewIfNeeded();
        expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
        await page.screenshot({ path: info.outputPath(`${scenario}-${width}.png`) });
      }
    }
  }
  expect(errors).toEqual([]); expect(unexpected).toEqual([]);
});

test("paper account replacement rejects old confirmation and validation without placing orders", async ({ page, request }) => {
  const errors: string[] = [], unexpected: string[] = [];
  let confirms = 0;
  await page.addInitScript(api => {
    localStorage.setItem("api_base", JSON.stringify(api));
    localStorage.setItem("api_auth_token", JSON.stringify("isolated-paper-browser"));
  }, API);
  page.on("pageerror", error => errors.push(error.message));
  await page.route("**/*", route => {
    const req = route.request(), url = new URL(req.url());
    const allowed = url.pathname === "/api/auth/ws-ticket"
      || /^\/api\/arbitrage\/opportunities\/[^/]+\/(preview|confirm)$/.test(url.pathname)
      || /^\/api\/automation\/execution-artifacts\/(build|validate)$/.test(url.pathname);
    if (![API, WEB].includes(url.origin) || (!["GET", "HEAD"].includes(req.method()) && !allowed)) {
      unexpected.push(`${req.method()} ${url.origin}${url.pathname}`); return route.abort();
    }
    if (url.pathname.endsWith("/confirm")) confirms++;
    return route.continue();
  });
  await page.goto("/#futures");
  await page.getByRole("button", { name: "创建交易计划", exact: true }).click();
  const built = page.waitForResponse(r => r.url().endsWith("/preview") && r.request().postDataJSON()?.capitalUsd === 10);
  await page.getByRole("textbox", { name: "计划本金 USD", exact: true }).fill("10");
  const preview = await (await built).json();
  expect(preview.executionBinding.adapter).toBe("mock");
  expect(preview.executionBinding.mode).toBe("dry_run");
  const artifact = page.locator(".execution-artifact");
  await expect(artifact.locator(".execution-artifact-identifiers")).toContainText(preview.ticket.ticketId);
  await artifact.getByRole("button", { name: "检查交易计划", exact: true }).click();
  await artifact.getByRole("checkbox").check();
  // The real settings route replaces the paper account. No credentials or live adapter exist here.
  for (const suffix of ["a", "b"]) {
    const selected = await request.post(`${API}/api/trading/adapters/select`, {
      headers: { ...headers, "Idempotency-Key": `paper-context-${suffix}` }, data: { adapterId: "mock" },
    });
    expect(selected.ok()).toBe(true);
  }
  const rejected = page.waitForResponse(r => r.url().endsWith("/confirm"));
  await page.locator(".confirm-action.primary").click();
  const response = await rejected;
  expect(response.status()).toBe(409);
  const problem = (await response.json()).error;
  expect(problem.code).toBe("HEDGE_EXECUTION_CONTEXT_CHANGED");
  expect(problem.details.confirmContext.ticketId).toBe(preview.ticket.ticketId);
  expect(problem.details.current.accountEpoch).toBeGreaterThan(preview.executionBinding.accountEpoch);
  await expect(page.locator(".execution-page")).toContainText("执行账户或环境已改变");
  await expect(page.getByRole("button", { name: "查询提交结果", exact: true })).toHaveCount(0);
  await expect(page.locator(".confirm-action.primary")).toBeDisabled();
  await expect(artifact.locator(".execution-artifact-status")).toContainText("校验未通过");
  await expect(page.getByTestId("top-status-bar")).toContainText("账户已改变，待重建");
  await expect(page.getByTestId("top-status-bar")).not.toContainText("读取失败");
  await expect(artifact.getByRole("checkbox")).not.toBeChecked();
  const orders = await (await request.get(`${API}/api/trading/orders`, { headers })).json();
  expect(orders.rows.filter((row: any) => [preview.longLeg.id, preview.shortLeg.id].includes(row.intent.id))).toHaveLength(0);
  // Revalidation of the same artifact must also describe it as blocked, never ready.
  const validation = page.waitForResponse(r => r.url().endsWith("/execution-artifacts/validate"));
  await artifact.getByRole("button", { name: "检查交易计划", exact: true }).click();
  const result = await (await validation).json();
  expect(result.valid).toBe(false); expect(result.status).toBe("blocked");
  expect(result.blockers.join(" ")).toContain("执行账户或环境已改变");
  await expect(artifact.locator(".execution-artifact-status")).toContainText("校验未通过");
  await expect(page.locator(".confirm-action.primary")).toBeDisabled();
  await page.screenshot({ path: test.info().outputPath("account-changed.png"), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.getByRole("button", { name: "刷新预览", exact: true }).scrollIntoViewIfNeeded();
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(390);
  await page.screenshot({ path: test.info().outputPath("account-changed-mobile.png") });
  const rebuilt = page.waitForResponse(r => r.url().endsWith("/preview") && r.ok());
  await page.getByRole("button", { name: "刷新预览", exact: true }).click();
  const current = await (await rebuilt).json();
  expect(current.ticket.ticketId).not.toBe(preview.ticket.ticketId);
  expect(current.executionBinding.accountEpoch).toBe(problem.details.current.accountEpoch);
  await expect(artifact.locator(".execution-artifact-status")).toContainText("待校验");
  expect(confirms).toBe(1);
  expect(errors).toEqual([]); expect(unexpected).toEqual([]);
});
