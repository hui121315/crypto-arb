import { expect, test, type WebSocketRoute } from "@playwright/test";

const API = "http://127.0.0.1:18000";
const WEB = "http://127.0.0.1:18080";

test("paper BP batch drives the real worker through start, viewer pause and cancelled quotes", async ({ page, request }) => {
  test.setTimeout(75_000);
  const headers = { Authorization: "Bearer isolated-paper-browser" };
  const state = async () => (await (await request.get(`${API}/api/stocks/peer/plans`, { headers })).json()).batch;
  const stats = async () => (await request.get(`${API}/__paper/stocks`, { headers })).json();
  const errors: string[] = [], unexpected: string[] = [];
  await page.addInitScript((api) => {
    localStorage.setItem("api_base", JSON.stringify(api));
    localStorage.setItem("api_auth_token", JSON.stringify("isolated-paper-browser"));
  }, API);
  page.on("pageerror", error => errors.push(error.message));
  await page.route("**/*", route => {
    const req = route.request(), url = new URL(req.url());
    if (![API, WEB].includes(url.origin) || (!["GET", "HEAD"].includes(req.method())
      && !["/api/auth/ws-ticket", "/api/stocks/batch"].includes(url.pathname))) {
      unexpected.push(`${req.method()} ${url.origin}${url.pathname}`);
      return route.abort();
    }
    return route.continue();
  });
  await page.goto("/#stocks");
  const panel = page.getByRole("region", { name: "批量链上监控" });
  const status = panel.locator(".stock-batch-state");
  const coverage = panel.getByLabel("双向新鲜报价", { exact: true });
  await expect(panel).toContainText("已选 2 / 32");
  await panel.getByLabel("批量更新间隔").selectOption("60");
  await panel.getByRole("button", { name: "开始批量轮询", exact: true }).click();
  await expect(panel.getByLabel("批量轮询时效")).toContainText("本轮已耗时");
  await expect.poll(async () => (await state()).roundStartedAtMs).toBeGreaterThan(0);
  const row = panel.getByRole("row").filter({ hasText: "Micron Technology" });
  await expect(row.locator("td").nth(3)).toHaveText("49 / 51");
  await expect.poll(async () => (await state()).completedRounds, { timeout: 15_000 }).toBe(1);
  await expect(row.locator("td").nth(1)).toHaveText("50");
  await expect(row.locator("td").nth(2)).toHaveText("48");
  const first = await stats();
  expect(first.quotes).toHaveLength(4);
  expect(first.rpcBatches).toHaveLength(1);
  expect(first.rpcBatches[0]).toHaveLength(4); // Two stocks + USDC + the same chain clock.
  expect(first.maxWs).toBe(1);
  await expect(coverage).toHaveText("双向新鲜 2/2");
  // The actual request starts after the shared quota queue, not before it.
  const firstState = await state();
  expect(firstState.roundStartedAtMs).toBeNull();
  expect(firstState.lastRoundElapsedMs).toBeGreaterThanOrEqual(5_850);
  await expect(panel.getByLabel("批量轮询时效")).toContainText("上轮耗时");
  await expect(panel.getByLabel("批量轮询时效")).not.toContainText("—");
  for (const quote of firstState.rows.flatMap((r: any) => [r.buy, r.sell])) {
    const index = first.quotes.findIndex((q: any) => q.inputMint === quote.inputMint
      && q.outputMint === quote.outputMint && q.amount === quote.inputRaw);
    expect(index).toBeGreaterThanOrEqual(0);
    expect(first.quoteTimesMs[index] - quote.requestedAtMs).toBeGreaterThanOrEqual(0);
    expect(first.quoteTimesMs[index] - quote.requestedAtMs).toBeLessThan(500);
  }
  expect(first.quoteTimesMs.slice(1).every((at: number, i: number) => at - first.quoteTimesMs[i] >= 1_950)).toBe(true);
  await page.screenshot({ path: test.info().outputPath("bp-worker-desktop.png"), fullPage: true });

  await page.goto("/#review");
  await expect.poll(async () => (await state()).waitingForViewers).toBe(true);
  expect((await state()).roundStartedAtMs).toBeNull();
  await expect.poll(async () => (await stats()).activeWs).toBe(0);
  await page.goto("/#stocks");
  // Rejoining before the next round must clear the cached "no viewers" WS state immediately.
  await expect(status).toHaveText("监控中 · 1 轮", { timeout: 2_000 });
  expect((await stats()).quotes).toHaveLength(4);
  await panel.getByRole("button", { name: "暂停", exact: true }).click();
  await expect(status).toHaveText("已暂停");
  expect((await state()).roundStartedAtMs).toBeNull();
  expect((await state()).lastRoundElapsedMs).toBeNull();
  await expect(panel.getByLabel("批量轮询时效")).toContainText("未运行");

  expect((await request.post(`${API}/__paper/stocks`, { headers, data: true })).status()).toBe(204);
  await panel.getByLabel("批量询价金额").fill("25.5");
  await panel.getByRole("button", { name: "开始批量轮询", exact: true }).click();
  await expect.poll(async () => (await stats()).quotes.length).toBe(5);
  await panel.getByRole("button", { name: "暂停", exact: true }).click();
  await expect(status).toHaveText("已暂停");
  // Observe cancellation across the 500ms worker tick and a full public quota interval.
  await page.waitForTimeout(700);
  await request.post(`${API}/__paper/stocks`, { headers, data: false });
  await page.waitForTimeout(2_300);
  expect((await stats()).quotes).toHaveLength(5);
  expect((await state()).rows.every((r: any) => r.buy === null && r.sell === null)).toBe(true);
  expect((await state()).completedRounds).toBe(1);
  expect((await state()).lastRoundElapsedMs).toBeNull();

  await panel.getByRole("button", { name: "开始批量轮询", exact: true }).click();
  await expect.poll(async () => (await state()).completedRounds, { timeout: 15_000 }).toBe(2);
  const resumed = await state();
  expect(resumed.request.budgetUsdc).toBe("25.5");
  expect(resumed.rows.every((r: any) => r.buy.inputRaw === "25500000" && r.sell !== null)).toBe(true);
  await expect(row.locator("td").nth(1)).toHaveText("50");
  await panel.getByRole("button", { name: "暂停", exact: true }).click();
  await expect(status).toHaveText("已暂停");
  await expect.poll(async () => (await stats()).activeWs).toBe(0);
  const final = await stats();
  expect(final.quotes).toHaveLength(9);
  expect(final.rpcBatches).toHaveLength(3);
  expect(final.maxWs).toBe(1);
  expect(final.unexpected).toBe(0);
  // Fresh coverage ages independently of round count, including while paused.
  await expect(coverage).toHaveText("双向新鲜 0/2", { timeout: 12_000 });
  await expect(status).toHaveText("已暂停");
  await expect(row.locator("td").nth(1)).toHaveText("—");
  await expect(row.locator("td").nth(2)).toHaveText("—");

  expect((await request.post(`${API}/__paper/stocks/quote-failure`, {
    headers: { ...headers, "Content-Type": "application/json" }, data: JSON.stringify("expiring_buys"),
  })).status()).toBe(204);
  await panel.getByRole("button", { name: "开始批量轮询", exact: true }).click();
  await expect.poll(async () => (await state()).completedRounds, { timeout: 15_000 }).toBe(3);
  const expired = await state();
  // Both responses exist, but their validity windows never overlap.
  expect(expired.problem).toContain("未取得完整双向");
  expect(expired.rows.every((r: any) => r.buy && r.sell && r.problem.includes("未同时有效")
    && r.buy.expiresAtMs < r.sell.requestedAtMs)).toBe(true);
  await expect(status).toContainText("等待重试");
  await expect(coverage).toHaveText("双向新鲜 0/2");
  await expect(row).toContainText("双向时效未齐");
  await expect(row.locator("td").nth(1)).toHaveText("—");
  await expect(row.locator("td").nth(2)).toHaveText("48");
  await page.screenshot({ path: test.info().outputPath("bp-non-overlapping-quotes.png"), fullPage: true });
  await panel.getByRole("button", { name: "暂停", exact: true }).click();
  await expect(status).toHaveText("已暂停");
  await expect.poll(async () => (await stats()).activeWs).toBe(0);
  const expiredStats = await stats();
  expect(expiredStats.quotes).toHaveLength(13);
  expect(expiredStats.rpcBatches).toHaveLength(4);
  expect(expiredStats.maxWs).toBe(1);
  expect(expiredStats.unexpected).toBe(0);
  expect((await request.post(`${API}/__paper/stocks/quote-failure`, {
    headers: { ...headers, "Content-Type": "application/json" }, data: JSON.stringify("none"),
  })).status()).toBe(204);
  await page.setViewportSize({ width: 390, height: 844 });
  await panel.scrollIntoViewIfNeeded();
  expect(await panel.evaluate(el => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
  await page.screenshot({ path: test.info().outputPath("bp-worker-mobile.png") });
  expect(errors).toEqual([]); expect(unexpected).toEqual([]);
});


test("paper settlement review joins original receipts without treating missing fees as profit", async ({ page, request }) => {
  const headers = { Authorization: "Bearer isolated-paper-browser" };
  const path = `${API}/api/review/settlements`;
  expect((await request.get(path)).status()).toBe(401);
  expect((await request.get(`${path}?record=missing`, { headers })).status()).toBe(400);
  const snapshot = await (await request.get(path, { headers })).json();
  const stock = snapshot.rows.find((r: any) => r.source === "stocks");
  const peer = snapshot.rows.find((r: any) => r.source === "stock_peer");
  expect(stock.accountingState).toBe("两腿原币收支已核对");
  expect(stock.amounts).toContainEqual({ label: "扣除所选换币费后差额", asset: "USDC", amount: "1.978003" });
  expect(peer.amounts).toContainEqual({ label: "已知原币现金变化", asset: "USD", amount: "-0.064" });
  expect(peer.amounts).toContainEqual({ label: "已知原币现金变化", asset: "USDC", amount: "3" });
  const errors: string[] = [], unexpected: string[] = [];
  await page.addInitScript((api) => {
    localStorage.setItem("api_base", JSON.stringify(api));
    localStorage.setItem("api_auth_token", JSON.stringify("isolated-paper-browser"));
  }, API);
  page.on("pageerror", (error) => errors.push(error.stack ?? error.message));
  await page.route("**/*", (route) => {
    const req = route.request(), url = new URL(req.url());
    if (![API, WEB].includes(url.origin) || (!["GET", "HEAD"].includes(req.method()) && url.pathname !== "/api/auth/ws-ticket")) {
      unexpected.push(`${req.method()} ${url.origin}${url.pathname}`); return route.abort();
    }
    return route.continue();
  });
  await page.goto("/#onchain");
  await page.getByRole("link", { name: "查看收支复盘", exact: true }).click();
  await expect(page).toHaveURL(/#review\?source=onchain&record=paper-onchain-review$/);
  const panel = page.getByRole("region", { name: "链上与股票收支复盘", exact: true });
  const row = panel.locator(".review-settlement-row");
  await expect(page.getByRole("tab", { name: /链上 \/ 股票/ })).toHaveAttribute("aria-selected", "true");
  await expect(row).toHaveCount(1);
  await expect(panel.getByRole("combobox", { name: "收支记录来源", exact: true })).toHaveValue("onchain");
  await expect(row).toContainText("全部腿已完成");
  await expect(row).toContainText("成交收支待核算");
  await row.locator("summary").click();
  await expect(row).toContainText("交易所 实际手续费待确认");
  await expect(row).not.toContainText("5 USD");
  let fail = true;
  await page.route("**/api/review/settlements?**", route => {
    if (fail) { fail = false; return route.abort("failed"); }
    return route.fallback();
  });
  await page.getByRole("button", { name: "刷新复盘记录", exact: true }).click();
  await expect(page.locator(".review-task-context")).toContainText("刷新失败 · 显示上次记录");
  await expect(row).toHaveCount(1);
  await page.getByRole("button", { name: "刷新复盘记录", exact: true }).click();
  await expect(page.locator(".review-task-context")).toContainText("记录来源异常");
  await expect(page.locator(".review-task-context")).not.toContainText("刷新失败");
  await expect(panel).toContainText("链上执行恢复日志未配置");
  await expect(row.locator("details")).toHaveAttribute("open", "");
  await page.goto("/#review?source=onchain&record=missing-original");
  await expect(row).toHaveCount(0);
  await expect(panel).toContainText("未找到这条原记录");
  await page.goto(`/#review?source=stocks&record=${encodeURIComponent(stock.id)}`);
  await expect(row).toHaveCount(1);
  await expect(row).toContainText("1.978003 USDC");
  await expect(row).toContainText("0.00375 股");
  await page.screenshot({ path: test.info().outputPath("settlement-review-desktop.png"), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(`/#review?source=stock_peer&record=${encodeURIComponent(peer.id)}`);
  await expect(row).toHaveCount(1);
  await expect(row).toContainText("-0.064 USD");
  await expect(row).toContainText("3 USDC");
  await panel.getByRole("button", { name: "查看该来源记录", exact: true }).click();
  await expect(panel).toContainText("最近本地保留记录");
  await expect(row).toHaveCount(1);
  await expect(row).toContainText("-0.064 USD");
  await expect(row).toContainText("3 USDC");
  await expect(page.getByRole("button", { name: "刷新复盘记录", exact: true })).toBeEnabled();
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(390);
  await page.screenshot({ path: test.info().outputPath("settlement-review-mobile.png"), fullPage: true });
  expect(errors).toEqual([]); expect(unexpected).toEqual([]);
});

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
  const panel = page.getByRole("region", { name: "自动化交易记录", exact: true });
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
    await page.getByRole("tab", { name: "交易记录", exact: true }).click();
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
    await page.getByRole("tab", { name: "处理流程", exact: true }).click();
    await expect(page.locator(".automation-flow-panel li").nth(5)).toContainText("模拟双腿平仓已确认");
    await page.getByRole("tab", { name: "交易记录", exact: true }).click();
    await panel.getByRole("link", { name: "关联持仓", exact: true }).click();
    await expect(page.locator(".positions-run-scope")).toContainText(runId);
    await expect(page.locator(".positions-table .row-close-button")).toHaveCount(0);
    await page.goto("/#automation");
    await expect(page.locator(".automation-command-status strong")).toHaveText("已暂停");
    await page.getByRole("tab", { name: "交易记录", exact: true }).click();
    await panel.getByRole("link", { name: "关联复盘", exact: true }).click();
    await expect(page.locator(".review-record-scope")).toContainText(runId);
    const review = await (await request.get(`${API}/api/review/executed?runId=${encodeURIComponent(runId)}&days=365`, { headers })).json();
    expect(review.rows.some((row: any) => row.evidence?.closeRunEvidence?.some((e: any) => e.runId === runId && e.closeRunId === close.id))).toBe(true);
    await page.goto("/#automation");
  }
  await page.getByRole("button", { name: "立即急停", exact: true }).click();
  await expect(page.locator(".automation-command-status strong")).toHaveText("已关闭");
  await page.getByRole("tab", { name: "交易记录", exact: true }).click();
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

for (const lostReply of [false, true]) test(lostReply
  ? "paper opportunity recovers a lost submit reply after reload without opening another pair"
  : "paper opportunity opens a pair, closes both legs and reaches its exact review", async ({ page, request }) => {
  const errors: string[] = [], unexpected: string[] = [];
  const confirms: any[] = [];
  const sockets = new Set<WebSocketRoute>();
  let sent = false, allowRecovery = !lostReply, submitted: any;
  const headers = { Authorization: "Bearer isolated-paper-browser" };
  const status = await (await request.get(`${API}/api/trading/status`, { headers })).json();
  expect(status.environment).toBe("paper"); expect(status.adapter).toBe("mock");
  await page.addInitScript((api) => {
    (Error as ErrorConstructor & { stackTraceLimit: number }).stackTraceLimit = 80;
    localStorage.setItem("api_base", JSON.stringify(api));
    localStorage.setItem("api_auth_token", JSON.stringify("isolated-paper-browser"));
  }, API);
  page.on("pageerror", (error) => errors.push(error.stack ?? error.message));
  if (lostReply) await page.routeWebSocket("**/ws**", (socket) => {
    if (sent) return socket.close();
    sockets.add(socket);
    socket.connectToServer();
  });
  await page.route("**/*", async (route) => {
    const req = route.request(), url = new URL(req.url());
    if (![API, WEB].includes(url.origin)) { unexpected.push(req.url()); return route.abort(); }
    if (!["GET", "HEAD"].includes(req.method()) && !(
      url.pathname === "/api/auth/ws-ticket" || /\/api\/arbitrage\/opportunities\/[^/]+\/(preview|confirm)$/.test(url.pathname)
      || /^\/api\/automation\/execution-artifacts\/(build|validate)$/.test(url.pathname)
      || /^\/api\/trading\/portfolio\/positions\/[^/]+\/[^/]+\/close-pair$/.test(url.pathname)
    )) { unexpected.push(`${req.method()} ${url.pathname}`); return route.abort(); }
    if (url.pathname.endsWith("/confirm")) {
      confirms.push(req.postDataJSON());
      sent = true;
      if (lostReply) {
        for (const socket of sockets) await socket.close();
        sockets.clear();
        // Execute the production paper path once, then lose only the browser's reply.
        const response = await route.fetch();
        expect(response.ok()).toBe(true);
        submitted = await response.json();
        return route.abort("failed");
      }
    }
    if (sent && !allowRecovery && ["/api/trading/execution-runs", "/api/trading/action-runs"].includes(url.pathname))
      return route.abort("failed");
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
  if (lostReply) {
    const query = page.getByRole("button", { name: "查询提交结果", exact: true });
    await expect(query).toBeEnabled();
    expect(submitted.executionRun.state).toBe("hedged");
    await page.reload();
    await expect(query).toBeEnabled();
    await expect(page.locator(".confirm-action.primary")).toHaveCount(0);
    expect(confirms).toHaveLength(1);
    allowRecovery = true;
    await query.click();
    const next = page.getByRole("navigation", { name: "历史执行后续操作", exact: true });
    const positions = next.getByRole("link", { name: "去持仓平仓", exact: true });
    const review = next.getByRole("link", { name: "关联复盘", exact: true });
    await expect(positions).toBeVisible();
    for (const link of [positions, review]) {
      const target = new URL((await link.getAttribute("href"))!, WEB);
      const params = new URLSearchParams(target.hash.split("?")[1]);
      expect(params.get("run")).toBe(submitted.executionRun.runId);
      expect(params.get("ticket")).toBe(submitted.executionRun.ticketId);
      expect(params.get("opp")).toBe(submitted.executionRun.opportunityId);
    }
    await expect(query).toHaveCount(0);
    await expect(page.locator(".confirm-action.primary")).toHaveCount(0);
    await expect(page.locator(".execution-status-bar .execution-section-head strong")).toHaveText("双腿完成");
    await page.screenshot({ path: test.info().outputPath("paper-recovered-desktop.png"), fullPage: true });
    await page.setViewportSize({ width: 390, height: 844 });
    await positions.click({ trial: true });
    await review.click({ trial: true });
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(390);
    expect(await page.locator(".execution-status-bar").evaluate((node) => {
      const right = node.getBoundingClientRect().right;
      return Array.from(node.querySelectorAll(".execution-section-head em, .execution-cost-summary span, .execution-fill-grid > div, .execution-timeline-row em"))
        .every((item) => item.scrollWidth <= item.clientWidth + 1 && item.getBoundingClientRect().right <= right + 1);
    })).toBe(true);
    await page.screenshot({ path: test.info().outputPath("paper-recovered-mobile.png"), fullPage: true });
    await page.setViewportSize({ width: 1440, height: 900 });
  }
  if (!lostReply) {
    await page.getByRole("link", { name: "关联复盘", exact: true }).click();
    const expected = new URLSearchParams(new URL(page.url()).hash.split("?")[1]);
    const original = page.getByRole("navigation", { name: "原运行后续操作", exact: true });
    await expect(original).toHaveCount(1);
    await expect(original).toContainText(expected.get("run")!);
    const linked = original.getByRole("link", { name: "关联持仓", exact: true });
    const destination = new URLSearchParams((await linked.getAttribute("href"))!.split("?")[1]);
    expect(destination.get("run")).toBe(expected.get("run"));
    expect(destination.get("ticket")).toBe(expected.get("ticket"));
    await linked.click();
  } else {
    await page.getByRole("link", { name: "去持仓平仓", exact: true }).click();
  }
  const scope = page.locator(".positions-run-scope");
  await expect(scope).toBeVisible();
  const runId = new URLSearchParams(new URL(page.url()).hash.split("?")[1]).get("run");
  expect(runId).toBeTruthy();
  if (lostReply) expect(runId).toBe(submitted.executionRun.runId);
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
  const closeLayout = await page.locator(".positions-table-wrap").evaluate((node) => {
    const bounds = node.getBoundingClientRect();
    return {
      overflow: node.scrollWidth - node.clientWidth,
      clipped: Array.from(node.querySelectorAll(".position-close-confirmation-row:not([hidden]) .position-close-confirmation-copy > span, .position-close-confirmation-row:not([hidden]) .position-close-confirmation-facts, .position-close-confirmation-row:not([hidden]) button"))
        .filter((item) => item.getBoundingClientRect().left < bounds.left || item.getBoundingClientRect().right > bounds.right)
        .map((item) => ({ text: item.textContent, left: item.getBoundingClientRect().left, right: item.getBoundingClientRect().right })),
    };
  });
  expect(closeLayout).toEqual({ overflow: 0, clipped: [] });
  await page.evaluate(() => window.scrollTo({ top: 0, behavior: "instant" }));
  await page.screenshot({ path: test.info().outputPath("paper-close-mobile.png"), fullPage: true });
  const close = page.waitForResponse((response) => response.url().endsWith("/close-pair"));
  await page.getByRole("button", { name: "模拟平配对", exact: true }).click();
  const closed = await (await close).json();
  expect(closed.status).toBe("succeeded");
  await expect(page.locator(".positions-table .row-close-button")).toHaveCount(0);
  await page.getByRole("tab", { name: "平仓", exact: true }).click();
  await page.locator(".position-history-record summary").first().click();
  await page.getByRole("link", { name: "关联复盘", exact: true }).click();
  await expect(page.locator(".review-record-scope[role='status']")).toContainText(closed.id);
  await expect(page.locator(".review-page")).toContainText("平仓");
  const review = await (await request.get(`${API}/api/review/executed?runId=${encodeURIComponent(runId!)}&days=365`, { headers })).json();
  expect(review.rows.length).toBeGreaterThan(0);
  expect(review.rows.some((row: any) => row.evidence?.closeRunEvidence?.some((e: any) => e.runId === runId))).toBe(true);
  expect(confirms).toHaveLength(1);
  expect(errors).toEqual([]); expect(unexpected).toEqual([]);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.screenshot({ path: test.info().outputPath("paper-cycle-review.png"), fullPage: true });
  if (!lostReply) {
    const original = page.getByRole("navigation", { name: "原运行后续操作", exact: true });
    await expect(original).toHaveCount(1);
    await expect(original).toContainText(runId!);
    await original.scrollIntoViewIfNeeded();
    await page.screenshot({ path: test.info().outputPath("paper-review-return-desktop.png") });
    await page.setViewportSize({ width: 390, height: 844 });
    await original.scrollIntoViewIfNeeded();
    await original.getByRole("link", { name: "查看原执行", exact: true }).click({ trial: true });
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(390);
    await page.screenshot({ path: test.info().outputPath("paper-review-return-mobile.png") });
    await original.getByRole("link", { name: "查看原执行", exact: true }).click();
    await expect(page.locator(".execution-runtime-disclosure")).toContainText(runId!);
    await expect(page.locator(".execution-page")).toContainText("执行已收口");
    await expect(page.locator(".confirm-action.primary")).toHaveCount(0);
    expect(confirms).toHaveLength(1);
    expect(errors).toEqual([]); expect(unexpected).toEqual([]);
  }
});

test("paper automation saves protection, opens once, rejects an old close environment and follows its closed pair", async ({ page, request }) => {
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
  await page.getByRole("tab", { name: "交易记录", exact: true }).click();
  const panel = page.getByRole("region", { name: "自动化交易记录", exact: true });
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
  const snapshot = async () => (await (await request.get(`${API}/api/trading/portfolio/snapshot`, { headers })).json()).snapshot;
  const beforeSwitch = await snapshot();
  const originalOrders = await (await request.get(`${API}/api/trading/orders`, { headers })).json();
  expect(originalOrders.rows).toHaveLength(2);
  const select = await request.post(`${API}/api/trading/adapters/select`, {
    headers: { ...headers, "Idempotency-Key": "paper-close-reselect" }, data: { adapterId: "mock" },
  });
  expect(select.ok()).toBe(true);
  const leg = beforeSwitch.positions[0];
  const closePath = `${API}/api/trading/portfolio/positions/${encodeURIComponent(leg.venue)}/${encodeURIComponent(leg.symbol)}/close-pair`;
  const rejected = await request.post(closePath, {
    headers: { ...headers, "Idempotency-Key": "paper-close-old-context" },
    data: { side: leg.side, snapshotVersion: beforeSwitch.snapshotVersion, expectedLegCount: 2 },
  });
  expect(rejected.status()).toBe(409);
  expect((await rejected.json()).error.code).toBe("CLOSE_RUN_STALE_SNAPSHOT");
  expect((await (await request.get(`${API}/api/trading/orders`, { headers })).json()).rows).toEqual(originalOrders.rows);
  await expect.poll(async () => (await snapshot()).snapshotVersion).not.toBe(beforeSwitch.snapshotVersion);
  // The same real positions remain; reload gets a new confirmation context before closing.
  await page.reload();
  await expect(positions).toHaveCount(2);
  await positions.first().click();
  await page.getByRole("button", { name: "模拟平配对", exact: true }).click();
  await expect(positions).toHaveCount(0);
  await page.goto("/#automation");
  await expect(page.locator(".automation-command-status strong")).toHaveText("已暂停");
  await expect.poll(async () => (await readStatus()).activeRunCount).toBe(0);
  await page.getByRole("tab", { name: "交易记录", exact: true }).click();
  await expect(panel).toContainText(runId);
  await expect(panel.locator(".automation-close-receipt")).toHaveCount(1);
  await panel.locator(".automation-close-receipt summary").click();
  await expect(panel).toContainText("本次平仓已成交");
  await page.getByRole("tab", { name: "处理流程", exact: true }).click();
  await expect(page.locator(".automation-flow-panel li").nth(5)).toContainText("双腿平仓已确认");
  await expect(page.locator(".automation-flow-panel header")).toContainText(runId);
  await page.getByRole("button", { name: "立即急停", exact: true }).click();
  await expect(page.locator(".automation-command-status strong")).toHaveText("已关闭");
  await expect(page.locator(".automation-flow-panel li").nth(5)).toContainText("双腿平仓已确认");
  expect((await readStatus()).config.enabled).toBe(false);
  await page.screenshot({ path: test.info().outputPath("automation-paper-closed-flow.png"), fullPage: true });
  await page.getByRole("tab", { name: "交易记录", exact: true }).click();
  expect((await readStatus()).recentDecisions.filter((row: any) => row.kind === "submitted")).toHaveLength(1);
  await page.screenshot({ path: test.info().outputPath("automation-paper-receipt.png"), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await panel.getByRole("link", { name: "关联复盘", exact: true }).click({ trial: true });
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(390);
  await panel.screenshot({ path: test.info().outputPath("automation-paper-receipt-mobile.png") });
  await panel.getByRole("link", { name: "关联复盘", exact: true }).click();
  await expect(page.locator('.review-record-scope[role="status"]')).toContainText(runId);
  await expect(page.locator(".review-page")).toContainText("BTCUSDT");
  expect(errors).toEqual([]); expect(unexpected).toEqual([]);
});
