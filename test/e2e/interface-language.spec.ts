import { expect, test } from "@playwright/test";
import { readFileSync } from "node:fs";
import { settingsEvidenceFixture } from "./fixtures/settings-evidence";
import { API, NOW, setup } from "./fixtures/opportunity-workbench";
import { settingsFixture } from "./fixtures/settings-workbench";
import { settingsAccountFixture } from "./fixtures/settings-account";
import { executionFixture } from "./fixtures/execution-workbench";
import { snapshot as onchainSnapshot } from "./fixtures/onchain-workbench";
import { setup as automationFixture, runtime, receipt, closeReceipt } from "./fixtures/automation-workbench";

test("plain saved-key and account-history details explain consequences without writes", async ({ page }, info) => {
  const f = await settingsAccountFixture(page);
  Object.assign(f.credentials.secretStorage, { mode: "runtime_only", persistent: false,
    atomicWrite: false, encrypted: false, message: "本次启动期间可用", warning: null, lastError: null });
  await page.goto("/#settings");
  const storage = page.locator(".runtime-health-panel").filter({ has: page.getByText("密钥保存方式", { exact: true }) });
  await page.locator(".credential-evidence-group").filter({ has: storage }).locator(":scope > summary").click();
  await expect(storage).toContainText("重启后丢失");
  await expect(storage).toContainText("未加密");
  await expect(storage).toContainText("临时内存");
  await expect(storage).not.toContainText(/持久化|仅本进程|Secret 存储/);
  Object.assign(f.credentials.secretStorage, { mode: "env_file_atomic", persistent: true,
    atomicWrite: true, label: "本地配置文件", path: "/isolated-fixture/.env", message: "配置文件可用" });
  await page.getByRole("button", { name: "刷新当前数据依据", exact: true }).click();
  await expect(storage).toContainText("重启后保留");
  await expect(storage).toContainText("配置文件 (.env)");
  await expect(storage).toContainText("未加密");
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await storage.scrollIntoViewIfNeeded();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    await page.screenshot({ path: info.outputPath(`key-saving-${width}.png`) });
  }
  const history = { count: 1, rows: [{ occurredAtMs: NOW - 60_000, navUsd: 100 }],
    source: "portfolio_nav_history", observedAtMs: NOW, latestAtMs: NOW - 60_000, freshnessMs: 60_000,
    backendStatus: { backend: "sqlite", enabled: true, durable: true, fallback: false,
      appendSuccessTotal: 1, appendErrorTotal: 0, querySuccessTotal: 1, queryErrorTotal: 0, observedAtMs: NOW },
    storageHealth: null, problem: { code: "ACCOUNT_FIELD_UNKNOWN", message: "account-level equity coverage incomplete",
      source: "portfolio_nav_store", details: { latestSampleSource: "account_equity_missing" } } };
  await page.route(API + "/api/trading/portfolio/nav-history?*", route => route.fulfill({ json: history }));
  await page.locator('.module-tabs [data-module="positions"]').click();
  await page.getByRole("tab", { name: "资产", exact: true }).click();
  const nav = page.locator(".positions-nav-region");
  await expect(nav).toContainText("暂不能记录账户净值");
  await expect(nav).toContainText("还没有读到所有账户的完整资产数据");
  await expect(nav).not.toContainText("暂停采样");
  await expect(nav).toContainText("记录可保存");
  await expect(nav).not.toContainText("持久化");
  await nav.getByText("技术诊断", { exact: true }).click();
  await expect(nav).toContainText("ACCOUNT_FIELD_UNKNOWN");
  await expect(nav).toContainText("account-level equity coverage incomplete");
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await nav.scrollIntoViewIfNeeded();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    await page.screenshot({ path: info.outputPath(`missing-account-data-${width}.png`) });
  }
  expect(f.calls.every(call => call.key.startsWith("GET "))).toBe(true);
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});

test("plain onchain alert rules retain percentage inputs and do not claim profit or send messages", async ({ page }, info) => {
  const f = await settingsFixture(page);
  const market = onchainSnapshot();
  await page.route(API + "/api/onchain/comparison", route => route.fulfill({ json: market }));
  await page.goto("/#onchain");
  await page.locator("#onchain-config-tab-alerts").click();
  const rules = page.locator(".onchain-spread-alert");
  await expect(rules).toContainText("不保证实际成交或盈利");
  const net = rules.getByLabel("扣费后价差达到 (%)", { exact: true });
  await expect(net).toHaveValue("0.2");
  await net.fill("0.125");
  await expect(net).toHaveValue("0.125");
  await rules.getByRole("button", { name: "只看价格差", exact: true }).click();
  await expect(rules).toContainText("不扣费用、不代表利润，也不会下单");
  await expect(rules.getByLabel("原始价差达到 (%)", { exact: true })).toHaveValue("0.2");
  await expect(rules.getByLabel("重复提醒间隔 (秒)", { exact: true })).toHaveValue("300");
  await rules.getByRole("button", { name: "扣费后价差", exact: true }).click();
  await expect(net).toHaveValue("0.125");
  await rules.locator(".onchain-alert-runtime-disclosure > summary").click();
  await expect(rules).toContainText("通知发送情况");
  await expect(rules).not.toContainText(/投递运行数据依据|Quote|双源新鲜/);
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    if (width === 390) await page.getByRole("navigation", { name: "链上套利工作区", exact: true })
      .getByRole("button", { name: "接入", exact: true }).click();
    await rules.scrollIntoViewIfNeeded();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    for (const field of await rules.locator(".workbench-field > span").all())
      expect(await field.evaluate(el => el.scrollWidth <= el.clientWidth + 1 && el.scrollHeight <= el.clientHeight + 1)).toBe(true);
    await page.screenshot({ path: info.outputPath(`onchain-notification-rules-${width}.png`) });
  }
  expect(f.requests.every(request => request.method === "GET")).toBe(true);
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});

test("plain expanded diagnostics and risk inputs preserve read-only checks and validation", async ({ page }, info) => {
  const f = await settingsEvidenceFixture(page, "diagnostics");
  await page.goto("/#settings");
  const scope = page.getByRole("tablist", { name: "诊断范围", exact: true });
  await scope.getByRole("tab", { name: "交易", exact: true }).click();
  const ticket = page.locator('[data-settings-table="ticket-venue-health"]');
  await expect(ticket).toContainText("两边交易所状态");
  await expect(ticket).toContainText("还没有交易计划");
  await expect(ticket).not.toContainText("HedgeTicket");
  await scope.getByRole("tab", { name: "运行数据依据", exact: true }).click();
  const diagnostics = page.getByRole("tabpanel", { name: "运行数据依据诊断", exact: true });
  await expect(diagnostics).toContainText("连接状态与耗时");
  await expect(diagnostics).toContainText("订单耗时是从创建订单到最后一次更新的时间，不是网络延迟");
  await expect(diagnostics).toContainText("没有测量数据时不估算");
  await scope.getByRole("tab", { name: "行情", exact: true }).click();
  const market = page.getByRole("tabpanel", { name: "行情诊断", exact: true });
  const marketSummary = market.locator(".settings-summary-grid");
  await expect(marketSummary).toContainText("本地行情读取");
  await expect(marketSummary).toContainText("暂用上次行情");
  await expect(marketSummary).toContainText("闲置查询清理");
  await expect(marketSummary).not.toContainText(/singleflight|in-flight|stale|Baseline|Orderbook Guard/);
  await expect(market.locator('td[title="funding_rates"]').first()).toHaveText("资金费率");
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await marketSummary.scrollIntoViewIfNeeded();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    for (const label of await marketSummary.locator("strong, span, em").all())
      expect(await label.evaluate(el => el.scrollWidth <= el.clientWidth + 1 && el.scrollHeight <= el.clientHeight + 1)).toBe(true);
    for (const item of await marketSummary.locator(":scope > div").all()) {
      const heading = await item.locator("strong").boundingBox();
      const value = await item.locator("span").boundingBox();
      const note = await item.locator("em").boundingBox();
      expect(heading!.y + heading!.height).toBeLessThanOrEqual(value!.y);
      expect(value!.y + value!.height).toBeLessThanOrEqual(note!.y);
    }
    await page.screenshot({ path: info.outputPath(`market-plain-details-${width}.png`) });
  }
  const symbol = page.getByRole("textbox", { name: "币种（留空查询全部）", exact: true });
  await symbol.fill("SOL");
  await page.getByRole("button", { name: "查询现货行情", exact: true }).click();
  await expect(page.getByRole("tabpanel", { name: "行情诊断", exact: true })).toContainText("现货行情查询失败");
  await expect(page.getByRole("tabpanel", { name: "行情诊断", exact: true })).toContainText("SPOT_FIXTURE_DISABLED");
  expect(f.calls.filter(call => call.key === "GET /api/v1/spot/ticks")).toHaveLength(1);
  await page.getByRole("tab", { name: "风控", exact: true }).click();
  await expect(page.getByLabel(/^单笔交易金额上限 USD/)).toBeVisible();
  for (const label of ["两边金额允许偏差 %", "连续确认次数", "再次触发间隔 秒"])
    await expect(page.getByLabel(label)).toBeVisible();
  const protection = page.locator(".settings-protection-stack");
  await expect(protection).toContainText("不是保证金的 1%");
  await expect(protection).toContainText("不按保证金计算");
  await page.getByLabel("连续确认次数").fill("1");
  await page.getByRole("button", { name: "保存风控", exact: true }).click();
  await expect(page.locator(".settings-risk-save-actions")).toContainText("连续确认次数");
  await page.getByLabel("连续确认次数").fill("2");
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await protection.scrollIntoViewIfNeeded();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    for (const hint of await protection.locator("label > span, label > em").all()) {
      expect(await hint.evaluate(el => el.scrollWidth <= el.clientWidth + 1 && el.scrollHeight <= el.clientHeight + 1)).toBe(true);
      expect(await hint.evaluate(el => getComputedStyle(el).whiteSpace)).toBe("normal");
    }
    await page.getByLabel("连续确认次数").scrollIntoViewIfNeeded();
    await page.screenshot({ path: info.outputPath(`risk-plain-details-${width}.png`) });
  }
  expect(f.requests.every(request => request.method === "GET")).toBe(true);
  expect(f.writes).toEqual([]); expect(f.errors).toEqual([]);
});

test("plain shared risk and quote details retain warnings, stale data and technical identifiers", async ({ page }, info) => {
  const f = await setup(page);
  await page.goto("/#futures");
  await page.locator(".futures-feed-status > summary").click();
  const detail = page.locator(".futures-diagnostics");
  await expect(detail).toContainText("距更新");
  await expect(detail).toContainText("资金费");
  await expect(detail).toContainText("行情问题");
  await expect(detail).toContainText("scope=main_p0");
  await expect(detail).not.toContainText(/快照年龄|market issues|history ok|candidates|Degraded/);
  f.stale(true);
  await expect(detail).toContainText("数据已过期");
  await expect(detail).toContainText("OPPORTUNITY_SNAPSHOT_STALE");
  await expect(page.getByRole("button", { name: "创建交易计划", exact: true })).toBeDisabled();
  f.stale(false);
  await expect(page.getByRole("button", { name: "创建交易计划", exact: true })).toBeEnabled();
  await page.locator(".futures-feed-status > summary").click();
  await page.locator(".status-summary").click();
  const risk = page.getByRole("group", { name: "风险与资金状态", exact: true }).getByRole("button");
  const { data: health } = await (await page.request.get(API + "/api/system/health")).json();
  await expect.poll(() => f.channelSockets.get("system")?.size ?? 0).toBeGreaterThan(0);
  let revision = 1;
  for (const [state, label] of [["ok", "正常"], ["warn", "需留意"], ["block", "交易受限"]]) {
    for (const socket of f.channelSockets.get("system")!) socket.send(JSON.stringify({
      type: "message", channel: "system", payload: { ...health, risk: state, updatedAtMs: NOW + revision++ },
    }));
    await expect(risk).toContainText(label);
    await expect(risk).not.toContainText(/OK|WARN|BLOCK/);
    if (state === "block") await expect(page.locator(".status-summary")).toContainText("风控已限制交易");
  }
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await risk.scrollIntoViewIfNeeded();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    expect(await risk.evaluate(el => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
    await page.screenshot({ path: info.outputPath(`shared-risk-language-${width}.png`) });
  }
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});

test("plain automation recovery and notification labels preserve pending and failed outcomes", async ({ page }, info) => {
  const f = await automationFixture(page);
  const current = runtime();
  current.config.enabled = true;
  current.state = "submitting";
  current.activeRunCount = 1;
  const record = receipt();
  Object.assign(record, { mode: "live" });
  current.lastDecision = { id: "language-decision", kind: "submitted", symbol: "SOL", reason: "fixture: submitted",
    executionRunId: record.run.runId, occurredAtMs: NOW };
  current.recentDecisions = [current.lastDecision];
  f.setStatus(current); f.setReceipt(record);
  Object.assign(f.webhook, { recentDeliveries: ["opportunity", "compensation", "system_degradation"].map((kind, index) => ({
    eventId: `language-notice-${index}`, kind, status: "queued", attempts: 0, applicationAck: "transport_only",
    updatedAtMs: NOW,
  })) });
  await page.goto("/#automation");
  await expect(page.locator(".automation-metric-strip")).toContainText("最低预计净收益");
  await expect(page.locator(".automation-metric-strip")).toContainText("进行中的交易");
  await page.locator(".automation-entry-config > summary").click();
  const count = page.getByLabel("最多同时交易数", { exact: true });
  const interval = page.getByLabel("再次开仓间隔 (秒)", { exact: true });
  await expect(count).toBeVisible();
  await expect(interval).toBeVisible();
  await count.fill("0");
  await page.getByRole("button", { name: "保存门槛", exact: true }).click();
  await expect(page.locator(".automation-action-notice")).toContainText("最多同时交易数");
  await count.fill("1");
  await interval.fill("1");
  await expect(interval).toHaveValue("1");
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await interval.scrollIntoViewIfNeeded();
    for (const field of await page.locator(".automation-entry-config label > span").all())
      expect(await field.evaluate(el => el.scrollWidth <= el.clientWidth + 1 && el.scrollHeight <= el.clientHeight + 1)).toBe(true);
    await page.screenshot({ path: info.outputPath(`automation-entry-language-${width}.png`) });
  }
  await page.locator(".automation-entry-config > summary").click();
  const protection = page.locator(".automation-protection");
  if (await protection.getAttribute("open") === null) await protection.locator("summary").click();
  await expect(protection.getByRole("checkbox", { name: /^防强平退出/ })).toBeVisible();
  await expect(protection).toContainText("净利润和收益率都达到设定值后");
  await expect(protection).toContainText("净亏损或亏损率达到任一设定值后");
  await page.locator(".webhook-monitor-disclosure > summary").click();
  await page.locator(".webhook-monitor-history > summary").click();
  const notices = page.getByLabel("最近通知发送记录");
  for (const label of ["符合条件的机会", "交易补救结果", "部分功能不可用"])
    await expect(notices).toContainText(label);
  await expect(notices).not.toContainText("确定性机会");
  await expect(notices).not.toContainText("已确认接收");
  await page.getByRole("button", { name: "查看处理结果", exact: true }).click();
  const panel = page.getByRole("region", { name: "自动化交易记录", exact: true });
  await expect(panel).toContainText("受理确认，非成交最终结果");
  const summary = panel.locator(".automation-receipt-summary");
  for (const [state, label] of [
    ["submitting_first_leg", "第一笔订单提交中"], ["first_leg_partial", "第一笔订单部分成交"],
    ["submitting_second_leg", "第二笔订单提交中"], ["unwind_required", "需要处理未对冲持仓"],
    ["unwinding", "正在处理未对冲持仓"],
  ]) {
    record.run.state = state;
    f.execution({ event: "execution_run_updated", executionRun: record.run, timestampMs: ++record.run.updatedAtMs });
    await expect(summary).toContainText(label);
  }
  const close = closeReceipt(record.run);
  for (const [state, label] of [
    ["compensation_submitted", "补救订单待确认"], ["compensation_failed", "补救失败"],
  ]) {
    close.status = state;
    f.execution({ event: "close_run_updated", closeRun: close, timestampMs: ++close.updatedAtMs });
    await expect(panel.locator(".automation-close-receipt > summary")).toContainText(label);
    await expect(panel).not.toContainText("本次平仓已成交");
  }
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await panel.scrollIntoViewIfNeeded();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    await page.screenshot({ path: info.outputPath(`automation-recovery-language-${width}.png`) });
    await protection.scrollIntoViewIfNeeded();
    await page.screenshot({ path: info.outputPath(`automation-protection-language-${width}.png`) });
  }
  await page.locator('.module-tabs [data-module="settings"]').click();
  await page.getByRole("tab", { name: "风控", exact: true }).click();
  const settingsProtection = page.locator(".settings-protection-stack");
  await expect(settingsProtection).toContainText("防强平退出");
  const section = settingsProtection.locator("xpath=ancestor::details[1]");
  if (await section.count() && await section.getAttribute("open") === null)
    await section.locator(":scope > summary").click();
  const guard = settingsProtection.getByRole("checkbox", { name: /^任一边接近强平时，平掉两边持仓/ });
  await guard.scrollIntoViewIfNeeded();
  await expect(guard).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
  await page.screenshot({ path: info.outputPath("settings-protection-language-390.png") });
  expect(f.requests.every(request => request.method === "GET")).toBe(true);
  expect(f.writes).toEqual([]); expect(f.errors).toEqual([]);
});

test("plain account amounts and notification limits remain distinct and editable", async ({ page }, info) => {
  const f = await settingsFixture(page);
  await page.goto("/#positions");
  const cards = page.locator(".portfolio-summary .summary-cards");
  for (const label of ["账户净值", "多空净差额", "未配对持仓金额", "今日已结算盈亏"])
    await expect(cards.getByText(label, { exact: true })).toBeVisible();
  await expect(cards).not.toContainText("PnL");
  await expect(cards).not.toContainText("NAV");
  const risks = page.locator(".compact-risk-list");
  await expect(risks).toContainText("交易急停");
  await expect(risks).toContainText("单日损失估计");
  await expect(risks).not.toContainText("Kill switch");
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await cards.scrollIntoViewIfNeeded();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    for (const label of await cards.locator(".summary-card > span").all())
      expect(await label.evaluate(el => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
    for (const note of await risks.locator("em").all()) {
      expect(await note.evaluate(el => el.scrollWidth <= el.clientWidth + 1 && el.scrollHeight <= el.clientHeight + 1)).toBe(true);
      expect(await note.evaluate(el => getComputedStyle(el).whiteSpace)).toBe("normal");
    }
    await page.screenshot({ path: info.outputPath(`account-language-${width}.png`) });
  }
  await page.locator('.module-tabs [data-module="settings"]').click();
  await page.locator(".webhook-advanced-settings summary").click();
  const expected = [
    ["单次等待上限 (ms)", "15000"], ["最多发送次数", "3"],
    ["重试基础等待 (ms)", "500"], ["最多待发消息", "128"],
  ];
  for (const [label, value] of expected) await expect(page.getByLabel(label, { exact: true })).toHaveValue(value);
  await page.getByLabel("重试基础等待 (ms)", { exact: true }).fill("750");
  await expect(page.getByLabel("重试基础等待 (ms)", { exact: true })).toHaveValue("750");
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    const advanced = page.locator(".webhook-advanced-settings");
    await advanced.scrollIntoViewIfNeeded();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    await page.screenshot({ path: info.outputPath(`notification-limits-${width}.png`) });
  }
  expect(f.requests.every(r => r.method === "GET")).toBe(true);
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});

test("plain trading directions and onchain estimates never claim completed profit", async ({ page }, info) => {
  const f = await executionFixture(page);
  const market = onchainSnapshot();
  await page.route(API + "/api/onchain/comparison", route => route.fulfill({ json: market }));
  await page.goto("/#futures");
  await page.getByRole("button", { name: "创建交易计划", exact: true }).click();
  await expect(page.locator(".long-leg .leg-control-head")).toContainText("买入一边");
  await expect(page.locator(".short-leg .leg-control-head")).toContainText("卖出一边");
  await expect(page.locator(".long-leg")).toContainText("交易金额 USD");
  await expect(page.locator(".short-leg")).toContainText("交易金额 USD");
  await expect(page.locator(".execution-ticket")).not.toContainText("名义金额");
  await expect(page.locator(".execution-ticket")).toContainText("预计净收益");
  await page.locator(".execution-evidence-details > summary").click();
  await expect(page.locator(".execution-checks")).toContainText("预计成交价偏差成本");
  await expect(page.locator(".slippage-section")).toContainText("不同价格下的可成交金额");
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    const checks = page.locator(".execution-evidence-details");
    await checks.locator(".risk-notes > div").filter({ hasText: "成本拆解" }).scrollIntoViewIfNeeded();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    for (const note of await checks.locator(".risk-notes strong").all()) {
      expect(await note.evaluate(el => el.scrollWidth <= el.clientWidth + 1 && el.scrollHeight <= el.clientHeight + 1)).toBe(true);
      expect(await note.evaluate(el => getComputedStyle(el).whiteSpace)).toBe("normal");
    }
    await page.screenshot({ path: info.outputPath(`execution-check-language-${width}.png`) });
  }
  await expect.poll(() => f.previews.length).toBe(1);
  expect(f.previews[0].longPrice).toBe(f.rows[0].longLeg.price);
  expect(f.previews[0].shortPrice).toBe(f.rows[0].shortLeg.price);
  await page.locator('.module-tabs [data-module="onchain"]').click();
  const badge = page.locator(".onchain-market-quality");
  await expect(badge).toHaveText("可创建计划");
  const chartState = page.locator(".onchain-chart-heading > span");
  await expect(chartState).toHaveAttribute("title", "正在根据两边实时报价更新走势");
  const readiness = page.locator(".onchain-readiness-fact");
  await expect(readiness.filter({ hasText: "路径" })).toContainText("2笔交易待核对");
  await expect(readiness.filter({ hasText: "深度" })).toContainText("创建计划时");
  await expect.poll(() => f.channelSockets.get("onchain")?.size ?? 0).toBe(1);
  for (const [quality, label] of [
    ["raw_cross_quote", "计价币不同"], ["mapping_invalid", "资产待核对"],
    ["no_net_profit", "暂无机会"], ["pending", "报价读取中"],
  ]) {
    Object.assign(market, { quality, observedAtMs: market.observedAtMs + 1 });
    for (const socket of f.channelSockets.get("onchain")!) socket.send(JSON.stringify({
      type: "message", channel: "onchain", payload: market,
    }));
    await expect(badge).toHaveText(label);
    await expect(badge).not.toContainText("盈利");
    expect(await badge.getAttribute("title")).not.toMatch(/Quote|Base|退避|双源|映射/);
  }
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await badge.scrollIntoViewIfNeeded();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    await page.screenshot({ path: info.outputPath(`onchain-state-language-${width}.png`) });
  }
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});

test("stock reminders explain queued and accepted messages without claiming device delivery", async ({ page }, info) => {
  const f = await setup(page);
  const market = JSON.parse(readFileSync(new URL("../../shared-types/fixtures/stocks_plan_build.json", import.meta.url), "utf8"));
  Object.assign(market, { plans: [], observedAtMs: NOW });
  market.monitor.enabled = true;
  market.monitor.alerts.enabled = true;
  const row = { eventId: "fixture-stock-language", direction: "链买 / 交易所卖", grossUsdc: "0.5",
    spreadPct: "1", queuedAtMs: NOW, delivery: null as any };
  market.alerts = { phase: "queued", problem: null, recent: [row] };
  for (const path of ["/api/stocks", "/api/stocks/peer/plans"])
    await page.route(API + path, route => route.fulfill({ json: market }));
  await page.route(API + "/api/stocks/catalog", route => route.fulfill({ json: { rows: [market.security], observedAtMs: NOW } }));
  await page.goto("/#stocks");
  await page.getByRole("navigation", { name: "股票详情视图" }).getByRole("button", { name: "行情与提醒", exact: true }).click();
  const quotes = page.locator("details.stock-secondary").filter({ has: page.getByText("盘口与参考行情", { exact: true }) });
  await quotes.locator(":scope > summary").click();
  await expect(quotes.getByRole("heading", { name: "交易所最优买卖报价", exact: true })).toBeVisible();
  await expect(quotes).toContainText("仅供参考，不保证按此价格成交");
  await expect(quotes).not.toContainText(/买卖一档|非 询价 成交承诺/);
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await quotes.scrollIntoViewIfNeeded();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    await page.screenshot({ path: info.outputPath(`stock-quotes-language-${width}.png`) });
  }
  await quotes.locator(":scope > summary").click();
  const alerts = page.locator(".stock-alerts");
  const history = page.getByLabel("股票提醒记录", { exact: true });
  await page.locator("details.stock-secondary").filter({ has: alerts }).locator(":scope > summary").click();
  await expect(alerts).toBeVisible();
  await expect(alerts).toContainText("股票价差提醒");
  await expect(history).toContainText("排队发送中 · 尚未确认送达");
  await expect.poll(() => f.channelSockets.get("stocks")?.size ?? 0).toBe(1);
  const emit = () => {
    market.observedAtMs++;
    for (const socket of f.channelSockets.get("stocks")!) socket.send(JSON.stringify({
      type: "message", channel: "stocks", payload: market,
    }));
  };
  row.delivery = { eventId: row.eventId, kind: "stock_spread", status: "delivered", attempts: 1,
    responseStatus: 200, applicationAck: "transport_only", error: null, updatedAtMs: NOW };
  emit();
  await expect(history).toContainText("网络请求成功 · 推送服务尚未确认接收");
  row.delivery.applicationAck = "accepted"; emit();
  await expect(history).toContainText("推送服务已确认");
  await expect(history).not.toContainText("手机已收到");
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await alerts.scrollIntoViewIfNeeded();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    await page.screenshot({ path: info.outputPath(`stock-reminder-language-${width}.png`) });
  }
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("plain reminder states distinguish temporary settings, saved settings and unsent notifications", async ({ page }, info) => {
  const f = await settingsEvidenceFixture(page, "diagnostics");
  const watchlist = await (await page.request.get(`${API}/e2e-watchlist-alerts-runtime/api/watchlist`)).json();
  const alerts = await (await page.request.get(`${API}/e2e-watchlist-alerts-runtime/api/alerts/rules`)).json();
  Object.assign(watchlist.items[0], { persistStatus: "volatile", version: 1 });
  Object.assign(watchlist.runtime, { volatile: true });
  await page.route(`${API}/api/watchlist`, route => route.fulfill({ json: watchlist }));
  await page.route(`${API}/api/alerts/rules`, route => route.fulfill({ json: alerts }));
  await page.goto("/#settings");
  await page.getByRole("tab", { name: "运行数据依据", exact: true }).click();
  const panel = page.getByRole("tabpanel", { name: "运行数据依据诊断", exact: true });
  const row = panel.locator("tbody tr").filter({ hasText: watchlist.items[0].symbol });
  await expect(row).toContainText("临时生效，重启后丢失");
  await expect(row).toContainText("超出数量限制");
  await expect(row).toContainText("WATCHLIST_PREWARM_CAPPED");
  await expect(panel).toContainText("排队发送中");
  await expect(panel).toContainText("暂不能发送");
  await expect(panel).not.toContainText("已发送成功");
  const headings = panel.locator("th, h3, .settings-summary-line strong");
  expect((await headings.allTextContents()).join("\n")).not.toMatch(/持久化|Prewarm|Watchlist|venue/);
  await expect.poll(() => f.channelSockets.get("watchlist")?.size ?? 0).toBe(1);
  Object.assign(watchlist.items[0], { persistStatus: "persisted", version: 2 });
  Object.assign(watchlist.runtime, { volatile: false, storage: {
    ...watchlist.runtime.storage, backend: "sqlite", configured: true, status: "ready", revision: 2,
  } });
  for (const socket of f.channelSockets.get("watchlist")!) socket.send(JSON.stringify({
    type: "message", channel: "watchlist", payload: {
      event: "watchlist_changed", envelope: watchlist, timestampMs: NOW + 1,
    },
  }));
  await expect(row).toContainText("已保存");
  await expect(row).not.toContainText("临时生效");
  await expect(panel).toContainText("重启后保留 · 保存正常 · 版本 2");
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    await page.screenshot({ path: info.outputPath(`reminder-language-${width}.png`), fullPage: true });
  }
  expect(f.calls.every(r => r.key.startsWith("GET "))).toBe(true);
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});
