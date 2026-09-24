import { expect, test } from "@playwright/test";

const API = "http://127.0.0.1:18000";
const WEB = "http://127.0.0.1:18080";

test("paper webhook hands a bound code to execution without reusing an expired ticket", async ({ page, request }) => {
  test.setTimeout(65_000);
  const headers = { Authorization: "Bearer isolated-paper-browser" };
  const errors: string[] = [], unexpected: string[] = [];
  await page.addInitScript((api) => {
    localStorage.setItem("api_base", JSON.stringify(api));
    localStorage.setItem("api_auth_token", JSON.stringify("isolated-paper-browser"));
  }, API);
  page.on("pageerror", (error) => errors.push(error.stack ?? error.message));
  await page.route("**/*", (route) => {
    const req = route.request(), url = new URL(req.url());
    if (![API, WEB].includes(url.origin)) { unexpected.push(req.url()); return route.abort(); }
    if (!["GET", "HEAD"].includes(req.method()) && !(
      url.pathname === "/api/auth/ws-ticket" || /\/api\/arbitrage\/opportunities\/[^/]+\/preview$/.test(url.pathname)
      || /^\/api\/automation\/execution-artifacts\/(build|validate)$/.test(url.pathname)
    )) { unexpected.push(`${req.method()} ${url.pathname}`); return route.abort(); }
    return route.continue();
  });
  await page.goto("/#futures");
  await page.getByRole("button", { name: "构建新双腿", exact: true }).click();
  const built = page.waitForResponse(async (response) => response.url().endsWith("/execution-artifacts/build")
    && response.ok() && (await response.json()).capitalUsd === 10);
  await page.getByRole("textbox", { name: "计划本金 USD", exact: true }).fill("10");
  const artifact = await (await built).json();
  const noticeResponse = await request.post(`${API}/__paper/webhook-preview`, { headers,
    data: { idempotencyKey: artifact.idempotencyKey, ticketId: artifact.ticketId, opportunitySnapshotId: artifact.opportunitySnapshotId } });
  expect(noticeResponse.ok()).toBe(true);
  const notice = await noticeResponse.json();
  expect(notice.event.payload.deterministicOpportunity).toBe(true);
  expect(notice.body.body).toContain("预期收益不等于保证盈利");
  expect(notice.body.body).toContain("接收后须重新校验");
  expect(notice.body.body).toContain(artifact.artifactId);
  expect(notice.body.copy).toBe(notice.event.payload.handoffCode);
  expect(notice.body.autoCopy).toBeUndefined();
  expect(notice.body.copy).not.toContain("isolated-paper-browser");
  expect(artifact.validationCommand).toContain("CROSSLINE_API_TOKEN:?");
  const original = JSON.parse(notice.body.copy.slice("CROSSLINE:".length));
  expect(original.ticketId).toBe(artifact.ticketId);
  await page.goto("/#execution");
  const inbox = page.locator(".execution-artifact-inbox");
  await inbox.locator("summary").click();
  const input = page.getByLabel("Webhook 校验码", { exact: true });
  await input.fill(notice.body.copy);
  await inbox.getByRole("button", { name: "校验提醒票据", exact: true }).click();
  await expect(inbox).toContainText("提醒票据校验通过 · 未下单");
  await expect(inbox).toContainText(artifact.ticketId);
  await expect(page.locator(".confirm-action.primary")).toBeDisabled();
  await expect(page.locator(".execution-artifact-identifiers")).toContainText(artifact.ticketId);
  await expect(page.locator(".execution-artifact-status")).toContainText("待校验");
  await input.fill(`CROSSLINE:${JSON.stringify({ ...original, checksum: "0".repeat(64) })}`);
  await expect(inbox).not.toContainText("提醒票据校验通过");
  await inbox.getByRole("button", { name: "校验提醒票据", exact: true }).click();
  await expect(inbox.getByRole("alert")).toContainText("不一致");
  await expect(inbox.getByRole("link", { name: "查看当前机会", exact: true })).toHaveCount(0);
  await input.fill(notice.body.copy);
  await inbox.getByRole("button", { name: "校验提醒票据", exact: true }).click();
  await expect(inbox).toContainText("提醒票据校验通过 · 未下单");
  await page.setViewportSize({ width: 390, height: 844 });
  await inbox.getByRole("link", { name: "查看当前机会", exact: true }).click({ trial: true });
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(390);
  await page.screenshot({ path: test.info().outputPath("webhook-inbox-mobile.png"), fullPage: true });
  // Let the original 30s market-evidence window elapse; no fake clock or replacement ticket.
  await expect(inbox).toContainText("提醒票据已过期", { timeout: 35_000 });
  await inbox.getByRole("button", { name: "校验提醒票据", exact: true }).click();
  await expect(inbox).toContainText("提醒票据已过期");
  await inbox.getByRole("link", { name: "查看当前机会", exact: true }).click();
  await expect(page).toHaveURL(new RegExp(`opp=${encodeURIComponent(artifact.opportunityId)}`));
  const table = page.getByRole("table", { name: "机会扫描候选", exact: true });
  await expect(table.locator("tbody tr[id]")).toHaveCount(1);
  await expect(table).toContainText("BTC");
  const runs = await (await request.get(`${API}/api/automation/status`, { headers })).json();
  expect(runs.config.enabled).toBe(false);
  expect(runs.recentDecisions.some((row: any) => row.kind === "submitted")).toBe(false);
  expect(errors).toEqual([]); expect(unexpected).toEqual([]);
});

test("paper protection closes paused runs for profit and loss without manual close requests", async ({ page, request }) => {
  const headers = { Authorization: "Bearer isolated-paper-browser" };
  const errors: string[] = [], unexpected: string[] = [];
  const readStatus = async () => (await request.get(`${API}/api/automation/status`, { headers })).json();
  const readReceipt = async (id: string) => (await request.get(`${API}/api/automation/execution-runs/${encodeURIComponent(id)}`, { headers })).json();
  const setMarket = async (phase: string) => {
    const response = await request.post(`${API}/__paper/market`, { headers: { ...headers, "Content-Type": "application/json" }, data: JSON.stringify(phase) });
    expect(response.status()).toBe(204);
  };
  expect((await request.post(`${API}/__paper/market`, { headers: { "Content-Type": "application/json" }, data: JSON.stringify("take_profit") })).status()).toBe(401);
  const trading = await (await request.get(`${API}/api/trading/status`, { headers })).json();
  expect(trading.environment).toBe("paper"); expect(trading.adapter).toBe("mock");
  expect((await readStatus()).config.enabled).toBe(false);
  await page.addInitScript((api) => {
    (Error as unknown as { stackTraceLimit: number }).stackTraceLimit = 80;
    localStorage.setItem("api_base", JSON.stringify(api));
    localStorage.setItem("api_auth_token", JSON.stringify("isolated-paper-browser"));
  }, API);
  page.on("pageerror", (error) => errors.push(error.stack ?? error.message));
  await page.route("**/*", (route) => {
    const req = route.request(), url = new URL(req.url());
    if (![API, WEB].includes(url.origin)) { unexpected.push(req.url()); return route.abort(); }
    if (!["GET", "HEAD"].includes(req.method()) && ![
      "/api/auth/ws-ticket", "/api/trading/risk-config", "/api/automation/config", "/api/automation/control",
    ].includes(url.pathname)) { unexpected.push(`${req.method()} ${url.pathname}`); return route.abort(); }
    return route.continue();
  });
  await page.goto("/#automation");
  await page.locator(".automation-entry-config > summary").click();
  await page.getByLabel("资金 (USD)", { exact: true }).fill("12.75");
  await page.getByLabel("入场冷却 (秒)", { exact: true }).fill("1");
  await page.getByRole("button", { name: "保存门槛", exact: true }).click();
  await expect(page.locator(".automation-action-notice")).toHaveText("自动化配置已保存");
  await page.getByRole("button", { name: "应用推荐组合", exact: true }).click();
  await page.getByLabel("最低净利润 USD", { exact: true }).fill("0.025");
  await page.getByRole("button", { name: "保存退出保护", exact: true }).click();
  await expect(page.locator(".automation-protection-message")).toHaveText("退出保护已保存");
  const panel = page.getByRole("region", { name: "自动化运行回执", exact: true });
  const runIds: string[] = [];
  for (const [phase, label] of [["take_profit", "自动止盈"], ["stop_loss", "自动止损"]]) {
    await setMarket("baseline");
    await page.getByRole("button", { name: runIds.length ? "恢复模拟自动提交" : "启动模拟自动化", exact: true }).click();
    await expect.poll(async () => (await readStatus()).recentDecisions.filter((row: any) => row.kind === "submitted").length).toBe(runIds.length + 1);
    const status = await readStatus();
    const runId = status.recentDecisions.find((row: any) => row.kind === "submitted" && !runIds.includes(row.executionRunId)).executionRunId;
    runIds.push(runId);
    await page.getByRole("button", { name: "暂停模拟新入场", exact: true }).click();
    await expect(page.locator(".automation-command-status strong")).toHaveText("已暂停");
    await page.getByRole("tab", { name: "运行回执", exact: true }).click();
    await expect(panel).toContainText(runId);
    expect((await readReceipt(runId)).run.state).toBe("hedged");
    expect((await readReceipt(runId)).closeRuns).toHaveLength(0);
    await setMarket(phase);
    await expect.poll(async () => (await readReceipt(runId)).run.state, { timeout: 15_000 }).toBe("closed");
    const receipt = await readReceipt(runId);
    expect(receipt.mode).toBe("dry_run"); expect(receipt.closeRuns).toHaveLength(1);
    const close = receipt.closeRuns[0];
    expect(close.status).toBe("succeeded");
    expect(close.reason).toContain(`auto_pair_exit trigger=${phase}`);
    expect(close.idempotencyKey).toContain(`auto-pair-exit:${runId}:${phase}:`);
    expect(close.legs).toHaveLength(2);
    expect(close.legs.every((leg: any) => leg.status === "filled" && leg.order.intent.mode === "dry_run" && leg.pairEvidence.runId === runId)).toBe(true);
    await expect(panel.locator(".automation-close-receipt")).toHaveCount(1);
    await panel.locator(".automation-close-receipt summary").click();
    await expect(panel).toContainText(`退出原因：${label}`);
    await expect(panel).toContainText("本次平仓已成交");
    await page.getByRole("tab", { name: "当前闭环", exact: true }).click();
    await expect(page.locator(".automation-flow-panel li").nth(5)).toContainText("模拟双腿平仓已确认");
    await page.getByRole("tab", { name: "运行回执", exact: true }).click();
    await panel.getByRole("link", { name: "关联持仓", exact: true }).click();
    await expect(page.locator(".positions-run-scope")).toContainText(runId);
    await expect(page.locator(".positions-table .row-close-button")).toHaveCount(0);
    await page.goto("/#automation");
    await expect(page.locator(".automation-command-status strong")).toHaveText("已暂停");
    await page.getByRole("tab", { name: "运行回执", exact: true }).click();
    await panel.getByRole("link", { name: "关联复盘", exact: true }).click();
    await expect(page.locator(".review-record-scope")).toContainText(runId);
    const review = await (await request.get(`${API}/api/review/executed?runId=${encodeURIComponent(runId)}&days=365`, { headers })).json();
    expect(review.rows.some((row: any) => row.evidence?.closeRunEvidence?.some((e: any) => e.runId === runId && e.closeRunId === close.id))).toBe(true);
    await page.goto("/#automation");
  }
  await page.getByRole("button", { name: "立即急停", exact: true }).click();
  await expect(page.locator(".automation-command-status strong")).toHaveText("已关闭");
  await page.getByRole("tab", { name: "运行回执", exact: true }).click();
  for (const id of runIds) {
    const receipt = await readReceipt(id);
    expect(receipt.closeRuns).toHaveLength(1); expect(receipt.run.state).toBe("closed");
  }
  expect((await readStatus()).recentDecisions.filter((row: any) => row.kind === "submitted")).toHaveLength(2);
  await panel.getByLabel("选择自动化运行记录").selectOption(runIds[0]);
  await expect(panel.locator(".automation-receipt-summary")).toContainText(runIds[0]);
  await panel.locator(".automation-close-receipt summary").click();
  await expect(panel).toContainText("退出原因：自动止盈");
  await panel.getByLabel("选择自动化运行记录").selectOption(runIds[1]);
  await expect(panel.locator(".automation-receipt-summary")).toContainText(runIds[1]);
  await panel.locator(".automation-close-receipt summary").click();
  await expect(panel).toContainText("退出原因：自动止损");
  await page.screenshot({ path: test.info().outputPath("automatic-exit-receipt.png"), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await panel.getByRole("link", { name: "关联复盘", exact: true }).click({ trial: true });
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(390);
  await page.evaluate(() => window.scrollTo({ top: 0, behavior: "instant" }));
  await expect.poll(() => page.evaluate(() => window.scrollY)).toBe(0);
  await page.screenshot({ path: test.info().outputPath("automatic-exit-mobile.png"), fullPage: true });
  expect(errors).toEqual([]); expect(unexpected).toEqual([]);
});

test("paper opportunity opens a pair, closes both legs and reaches its exact review", async ({ page, request }) => {
  const errors: string[] = [], unexpected: string[] = [];
  const headers = { Authorization: "Bearer isolated-paper-browser" };
  const status = await (await request.get(`${API}/api/trading/status`, { headers })).json();
  expect(status.environment).toBe("paper"); expect(status.adapter).toBe("mock");
  await page.addInitScript((api) => {
    localStorage.setItem("api_base", JSON.stringify(api));
    localStorage.setItem("api_auth_token", JSON.stringify("isolated-paper-browser"));
  }, API);
  page.on("pageerror", (error) => errors.push(error.message));
  await page.route("**/*", (route) => {
    const req = route.request(), url = new URL(req.url());
    if (![API, WEB].includes(url.origin)) { unexpected.push(req.url()); return route.abort(); }
    if (!["GET", "HEAD"].includes(req.method()) && !(
      url.pathname === "/api/auth/ws-ticket" || /\/api\/arbitrage\/opportunities\/[^/]+\/(preview|confirm)$/.test(url.pathname)
      || /^\/api\/automation\/execution-artifacts\/(build|validate)$/.test(url.pathname)
      || /^\/api\/trading\/portfolio\/positions\/[^/]+\/[^/]+\/close-pair$/.test(url.pathname)
    )) { unexpected.push(`${req.method()} ${url.pathname}`); return route.abort(); }
    return route.continue();
  });
  await page.goto("/#futures");
  await page.getByRole("button", { name: "构建新双腿", exact: true }).click();
  const previewResponse = page.waitForResponse((response) => response.url().endsWith("/preview")
    && response.request().postDataJSON()?.capitalUsd === 10);
  await page.getByRole("textbox", { name: "计划本金 USD", exact: true }).fill("10");
  const preview = await (await previewResponse).json();
  const artifact = page.locator(".execution-artifact");
  await expect(artifact.locator(".execution-artifact-status")).toContainText("待校验");
  await expect(artifact.locator(".execution-artifact-identifiers")).toContainText(preview.ticket.ticketId);
  await artifact.getByRole("button", { name: "校验票据" }).click();
  await artifact.getByRole("checkbox").check();
  await page.locator(".confirm-action.primary").click();
  await page.getByRole("link", { name: "去持仓平仓", exact: true }).click();
  const scope = page.locator(".positions-run-scope");
  await expect(scope).toBeVisible();
  const runId = new URLSearchParams(new URL(page.url()).hash.split("?")[1]).get("run");
  expect(runId).toBeTruthy();
  const closeButtons = page.locator(".positions-table .row-close-button");
  await expect(closeButtons).toHaveCount(2);
  await closeButtons.first().click();
  const confirmation = page.getByRole("group", { name: /^确认平配对：/ });
  await expect(confirmation).toHaveCount(1);
  await expect(closeButtons.last()).toHaveAttribute("aria-expanded", "false");
  await confirmation.getByRole("button", { name: "取消", exact: true }).press("Escape");
  await expect(closeButtons.first()).toBeFocused();
  await closeButtons.last().click();
  await expect(confirmation).toHaveCount(1);
  await expect(closeButtons.first()).toHaveAttribute("aria-expanded", "false");
  expect(await closeButtons.first().getAttribute("aria-controls")).not.toBe(await closeButtons.last().getAttribute("aria-controls"));
  await page.setViewportSize({ width: 390, height: 844 });
  await confirmation.getByRole("button", { name: "模拟平配对", exact: true }).click({ trial: true });
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(390);
  await page.screenshot({ path: test.info().outputPath("paper-close-mobile.png"), fullPage: true });
  const close = page.waitForResponse((response) => response.url().endsWith("/close-pair"));
  await page.getByRole("button", { name: "模拟平配对", exact: true }).click();
  const closed = await (await close).json();
  expect(closed.status).toBe("succeeded");
  await expect(page.locator(".positions-table .row-close-button")).toHaveCount(0);
  await page.getByRole("tab", { name: "平仓", exact: true }).click();
  await page.locator(".position-history-record summary").first().click();
  await page.getByRole("link", { name: "关联复盘", exact: true }).click();
  await expect(page.locator(".review-record-scope")).toContainText(closed.id);
  await expect(page.locator(".review-page")).toContainText("平仓");
  const review = await (await request.get(`${API}/api/review/executed?runId=${encodeURIComponent(runId!)}&days=365`, { headers })).json();
  expect(review.rows.length).toBeGreaterThan(0);
  expect(review.rows.some((row: any) => row.evidence?.closeRunEvidence?.some((e: any) => e.runId === runId))).toBe(true);
  expect(errors).toEqual([]); expect(unexpected).toEqual([]);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.screenshot({ path: test.info().outputPath("paper-cycle-review.png"), fullPage: true });
});

test("paper automation saves protection, opens once, pauses and follows its closed pair", async ({ page, request }) => {
  const headers = { Authorization: "Bearer isolated-paper-browser" };
  const errors: string[] = [], unexpected: string[] = [];
  const readStatus = async () => (await request.get(`${API}/api/automation/status`, { headers })).json();
  const trading = await (await request.get(`${API}/api/trading/status`, { headers })).json();
  expect(trading.environment).toBe("paper"); expect(trading.adapter).toBe("mock");
  expect((await readStatus()).config.enabled).toBe(false);
  await page.addInitScript((api) => {
    localStorage.setItem("api_base", JSON.stringify(api));
    localStorage.setItem("api_auth_token", JSON.stringify("isolated-paper-browser"));
  }, API);
  page.on("pageerror", (error) => errors.push(error.message));
  await page.route("**/*", (route) => {
    const req = route.request(), url = new URL(req.url());
    if (![API, WEB].includes(url.origin)) { unexpected.push(req.url()); return route.abort(); }
    if (!["GET", "HEAD"].includes(req.method()) && !(
      url.pathname === "/api/auth/ws-ticket" || url.pathname === "/api/trading/risk-config"
      || url.pathname === "/api/automation/config" || url.pathname === "/api/automation/control"
      || /^\/api\/trading\/portfolio\/positions\/[^/]+\/[^/]+\/close-pair$/.test(url.pathname)
    )) { unexpected.push(`${req.method()} ${url.pathname}`); return route.abort(); }
    return route.continue();
  });
  await page.goto("/#automation");
  const start = page.getByRole("button", { name: "启动模拟自动化", exact: true });
  await expect(page.getByRole("button", { name: "先配置退出保护", exact: true })).toBeDisabled();
  await page.getByRole("button", { name: "应用推荐组合", exact: true }).click();
  await page.getByRole("button", { name: "保存退出保护", exact: true }).click();
  await expect(page.locator(".automation-protection-message")).toHaveText("退出保护已保存");
  await page.locator(".automation-entry-config > summary").click();
  await page.getByLabel("资金 (USD)", { exact: true }).fill("12.75");
  await page.getByLabel("入场冷却 (秒)", { exact: true }).fill("1");
  await page.getByRole("button", { name: "保存门槛", exact: true }).click();
  await expect(page.locator(".automation-action-notice")).toHaveText("自动化配置已保存");
  await start.click();
  await expect.poll(async () => (await readStatus()).activeRunCount).toBe(1);
  let status = await readStatus();
  const submitted = status.recentDecisions.filter((row: any) => row.kind === "submitted");
  expect(submitted).toHaveLength(1);
  const runId = submitted[0].executionRunId;
  const receipt = async () => (await request.get(`${API}/api/automation/execution-runs/${encodeURIComponent(runId)}`, { headers })).json();
  await page.getByRole("tab", { name: "运行回执", exact: true }).click();
  const panel = page.getByRole("region", { name: "自动化运行回执", exact: true });
  await expect(panel).toContainText(runId);
  await expect(panel.locator(".automation-receipt-leg")).toHaveCount(2);
  const opened = await receipt();
  expect(opened.run.state).toBe("hedged");
  expect(opened.mode).toBe("dry_run");
  expect(opened.run.longLeg.finalitySource).toBe("adapter_ack");
  expect(opened.run.shortLeg.finalitySource).toBe("adapter_ack");
  await expect(panel).toContainText("模拟记录，非实盘成交");
  await page.getByRole("button", { name: "暂停模拟新入场", exact: true }).click();
  await expect(page.locator(".automation-command-status strong")).toHaveText("已暂停");
  status = await readStatus();
  expect(status.config.paused).toBe(true); expect(status.activeRunCount).toBe(1);
  await panel.getByRole("link", { name: "关联持仓", exact: true }).click();
  await expect(page.locator(".positions-run-scope")).toContainText(runId);
  const positions = page.locator(".positions-table .row-close-button");
  await expect(positions).toHaveCount(2);
  await positions.first().click();
  await page.getByRole("button", { name: "模拟平配对", exact: true }).click();
  await expect(positions).toHaveCount(0);
  await page.goto("/#automation");
  await expect(page.locator(".automation-command-status strong")).toHaveText("已暂停");
  await expect.poll(async () => (await readStatus()).activeRunCount).toBe(0);
  await page.getByRole("tab", { name: "运行回执", exact: true }).click();
  await expect(panel).toContainText(runId);
  await expect(panel.locator(".automation-close-receipt")).toHaveCount(1);
  await panel.locator(".automation-close-receipt summary").click();
  await expect(panel).toContainText("本次平仓已成交");
  await page.getByRole("tab", { name: "当前闭环", exact: true }).click();
  await expect(page.locator(".automation-flow-panel li").nth(5)).toContainText("双腿平仓已确认");
  await expect(page.locator(".automation-flow-panel header")).toContainText(runId);
  await page.getByRole("button", { name: "立即急停", exact: true }).click();
  await expect(page.locator(".automation-command-status strong")).toHaveText("已关闭");
  await expect(page.locator(".automation-flow-panel li").nth(5)).toContainText("双腿平仓已确认");
  expect((await readStatus()).config.enabled).toBe(false);
  await page.screenshot({ path: test.info().outputPath("automation-paper-closed-flow.png"), fullPage: true });
  await page.getByRole("tab", { name: "运行回执", exact: true }).click();
  expect((await readStatus()).recentDecisions.filter((row: any) => row.kind === "submitted")).toHaveLength(1);
  await page.screenshot({ path: test.info().outputPath("automation-paper-receipt.png"), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await panel.getByRole("link", { name: "关联复盘", exact: true }).click({ trial: true });
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(390);
  await panel.screenshot({ path: test.info().outputPath("automation-paper-receipt-mobile.png") });
  await panel.getByRole("link", { name: "关联复盘", exact: true }).click();
  await expect(page.locator(".review-record-scope")).toContainText(runId);
  await expect(page.locator(".review-page")).toContainText("BTCUSDT");
  expect(errors).toEqual([]); expect(unexpected).toEqual([]);
});
