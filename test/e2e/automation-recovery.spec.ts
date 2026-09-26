import { expect, test, type Page } from "@playwright/test";
import { API, WEB, NOW, setup } from "./fixtures/automation-workbench";

const recovery = (page: Page) => page.getByRole("alert", { name: "设置操作待核对" });
const recheck = (page: Page) => recovery(page).getByRole("button", { name: "核对上次操作", exact: true });
const capital = (page: Page) => page.getByLabel("资金 (USD)", { exact: true });
const save = (page: Page) => page.getByRole("button", { name: "保存门槛", exact: true });
const go = (page: Page, module: string) => page.locator(`.module-tabs button[data-module="${module}"]`).click();
async function rules(page: Page) {
  const details = page.locator(".automation-entry-config");
  if (await details.getAttribute("open") === null) await details.locator("summary").click();
}
async function protection(page: Page) {
  const details = page.locator(".automation-protection");
  if (await details.getAttribute("open") === null) await details.locator("summary").click();
}
async function capture(page: Page) {
  await page.setViewportSize({ width: 1440, height: 900 });
  await recovery(page).scrollIntoViewIfNeeded();
  await page.screenshot({ path: test.info().outputPath("automation-recovery-desktop.png") });
  await page.setViewportSize({ width: 390, height: 844 });
  await recheck(page).scrollIntoViewIfNeeded();
  await expect(recheck(page)).toBeInViewport();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
  await page.screenshot({ path: test.info().outputPath("automation-recovery-mobile.png") });
  await page.setViewportSize({ width: 1440, height: 900 });
}

test("unknown automation writes recover current rules and shared protection without resubmission", async ({ page }) => {
  await page.clock.install({ time: NOW });
  const f = await setup(page);
  const ledger = await (await page.request.get(`${API}/api/trading/action-runs`)).json();
  let run: any, reject = true, hide = false, writes = 0;
  await page.route(`${API}/api/trading/action-runs**`, route => route.fulfill({ json:
    new URL(route.request().url()).pathname.endsWith("/action-runs")
      ? { ...ledger, status: "ready", data: hide || !run ? [] : [run] } : run }));
  await page.route(`${API}/api/automation/config`, route => {
    writes++;
    if (reject) return route.fulfill({ status: 400, json: { code: "INVALID_INPUT", message: "fixture: invalid rule" } });
    const headers = route.request().headers();
    run = { id: "automation-original", kind: "automation_config_update", target: "automated-arbitrage",
      requestId: headers["x-request-id"], idempotencyKey: headers["idempotency-key"], actor: "fixture",
      status: "accepted", message: "fixture: processing", startedAtMs: NOW, updatedAtMs: NOW, result: null, problem: null };
    return route.fulfill({ status: 504, json: { code: "TIMEOUT", message: "fixture: response lost" } });
  });
  await page.goto(`${WEB}/#automation`);
  await rules(page);
  await capital(page).fill("12.75"); await save(page).click();
  await expect(page.locator(".automation-action-notice")).toContainText("INVALID_INPUT");
  await expect(capital(page)).toHaveValue("12.75"); await expect(save(page)).toBeEnabled();
  reject = false; await save(page).click();
  await expect(recovery(page)).toContainText("结果待核对"); await expect(capital(page)).toBeDisabled();
  await page.getByRole("link", { name: "查看风控总闸", exact: true }).click();
  await expect(page.getByRole("tab", { name: "风控", exact: true })).toHaveAttribute("aria-selected", "true");
  await go(page, "futures"); await go(page, "automation"); await rules(page);
  await expect(capital(page)).toHaveValue("12.75"); await expect(capital(page)).toBeDisabled();
  const records = await page.evaluate(() => Object.entries(sessionStorage).filter(([key]) => key.startsWith("crossline.settings.pending.v1:automation:")));
  expect(records).toHaveLength(1);
  expect(Object.keys(JSON.parse(records[0][1])).sort()).toEqual(["context", "kind", "run_id", "target", "version"]);
  expect(records[0][1]).not.toContain("capitalUsd");
  await page.reload(); await expect(recovery(page)).toContainText("保存自动化配置结果待核对");
  await expect(page.locator(".automation-runtime-board")).toContainText("操作结果待核对");
  await expect(page.locator(".status-summary")).not.toContainText("读取失败");
  hide = true; await recheck(page).click(); await expect(recovery(page)).toContainText("SETTINGS_RECEIPT_NOT_FOUND");
  hide = false; await recheck(page).click(); await expect(recovery(page)).toContainText("后端已受理");
  const id = run.requestId; run.requestId = "other";
  await recheck(page).click(); await expect(recovery(page)).toContainText("SETTINGS_RECEIPT_MISMATCH");
  run.requestId = id; run.status = "succeeded";
  await recheck(page).click(); await expect(recovery(page)).toContainText("SETTINGS_RECEIPT_MISSING");
  run.result = { ...f.status(), updatedAtMs: 0 };
  await recheck(page).click(); await expect(recovery(page)).toContainText("SETTINGS_RECEIPT_MISMATCH");
  await capture(page);
  run.result = f.status(); run.result.config.capitalUsd = 12.75;
  const current = f.status(); current.updatedAtMs += 50; current.config.capitalUsd = 25;
  f.setStatus(current); f.failRead();
  await recheck(page).click(); await expect(recovery(page)).toBeHidden();
  await expect(page.getByRole("alert", { name: "自动化当前配置待同步" })).toBeVisible();
  await rules(page); await expect(save(page)).toBeDisabled();
  f.failRead(false);
  await page.getByRole("button", { name: "重新读取当前配置", exact: true }).click();
  await expect(capital(page)).toHaveValue("25"); await expect(save(page)).toBeEnabled();
  expect(writes).toBe(2);
  await page.unroute(`${API}/api/automation/config`);
  // Worker failure still permits a protective pause after an uncertain write is resolved.
  await page.getByRole("button", { name: "启动模拟自动化", exact: true }).click();
  f.setWorker("blocked"); await page.clock.runFor(5000);
  await page.getByRole("button", { name: "暂停新入场（状态待确认）", exact: true }).click();
  expect(f.status().config.paused).toBe(true);
  f.setWorker("ok"); await page.clock.runFor(5000);
  await expect(page.locator(".automation-command-status strong")).toHaveText("已暂停");
  // Exit protection uses the same request journal as Settings, not a second writer.
  let riskWrites = 0;
  const trading = await (await page.request.get(`${API}/api/trading/status`)).json();
  trading.risk.autoProfitClose = { enabled: true, minNetProfitUsd: 0.25, minRoiBps: 10,
    stopLossEnabled: false, maxNetLossUsd: 2, maxLossRoiBps: 100, liquidationGuardEnabled: false,
    liquidationExitDistancePct: 8, exitBufferBps: 5, confirmationSamples: 3, cooldownSecs: 60 };
  await page.route(`${API}/api/trading/risk-config`, route => {
    riskWrites++; const headers = route.request().headers();
    run = { ...run, id: "risk-original", kind: "trading_risk_config_update", target: "risk-config",
      requestId: headers["x-request-id"], idempotencyKey: headers["idempotency-key"], status: "succeeded",
      result: { ...trading, requestId: headers["x-request-id"], idempotencyKey: headers["idempotency-key"], actionRunId: "risk-original" } };
    return route.fulfill({ status: 504, json: { code: "TIMEOUT", message: "fixture: risk response lost" } });
  });
  await protection(page); await page.getByLabel("最低净利润 USD", { exact: true }).fill("0.25");
  await page.getByRole("button", { name: "保存退出保护", exact: true }).click();
  await expect(recovery(page)).toContainText("保存风控结果待核对");
  await go(page, "settings"); await page.getByRole("tab", { name: "风控", exact: true }).click();
  await expect(recovery(page)).toContainText("保存风控结果待核对");
  await go(page, "automation"); await page.reload();
  await page.route(`${API}/api/trading/status`, route => route.fulfill({ json: trading }));
  await recheck(page).click(); await expect(recovery(page)).toBeHidden();
  await protection(page); await expect(page.getByLabel("最低净利润 USD", { exact: true })).toHaveValue("0.25");
  await expect(page.getByRole("button", { name: "恢复模拟自动提交", exact: true })).toBeEnabled();
  expect(riskWrites).toBe(1); expect(f.errors).toEqual([]);
});

test("real isolated automation persists rules and control receipts across a lost response and reload", async ({ page, request }) => {
  const headers = { Authorization: "Bearer isolated-paper-browser" };
  const errors: string[] = [], unexpected: string[] = [], writes: any[] = [];
  let lost = "/api/automation/config", release = () => {};
  const allowed = ["/api/automation/config", "/api/automation/control", "/api/trading/risk-config"];
  await page.addInitScript(api => {
    localStorage.setItem("api_base", JSON.stringify(api));
    localStorage.setItem("api_auth_token", JSON.stringify("isolated-paper-browser"));
  }, API);
  page.on("pageerror", error => errors.push(error.message));
  await page.route("**/*", async route => {
    const req = route.request(), url = new URL(req.url());
    if (![API, WEB].includes(url.origin) || (!["GET", "HEAD"].includes(req.method())
      && !["/api/auth/ws-ticket", ...allowed].includes(url.pathname))) {
      unexpected.push(`${req.method()} ${url.pathname}`); return route.abort();
    }
    if (req.method() !== "GET" && allowed.includes(url.pathname)) {
      const row = { path: url.pathname, headers: req.headers(), body: req.postDataJSON(), result: null as any };
      writes.push(row);
      const response = await route.fetch(); expect(response.ok()).toBe(true); row.result = await response.json();
      if (url.pathname === lost) {
        lost = ""; await new Promise<void>(resolve => release = resolve);
        return route.abort().catch(() => {});
      }
      return route.fulfill({ response });
    }
    return route.continue();
  });
  const current = async () => (await request.get(`${API}/api/automation/status`, { headers })).json();
  await page.goto(`${WEB}/#automation`); await rules(page);
  await capital(page).fill("12.75");
  // Keep this configuration/control check free of even simulated orders.
  await page.getByLabel("规范币种（留空为全部）", { exact: true }).fill("NO-FIXTURE-MARKET");
  await save(page).click();
  await expect.poll(() => writes[0]?.result?.config.capitalUsd).toBe(12.75);
  await go(page, "futures"); await go(page, "automation"); await rules(page);
  await expect(capital(page)).toBeDisabled();
  await page.reload(); release(); await expect(recovery(page)).toBeVisible();
  await capture(page);
  const newer = await request.patch(`${API}/api/automation/config`, { headers: {
    ...headers, "x-request-id": "automation-newer", "idempotency-key": "automation-newer" }, data: { capitalUsd: 20 } });
  expect(newer.ok()).toBe(true);
  await recheck(page).click(); await expect(recovery(page)).toBeHidden(); await rules(page);
  await expect(capital(page)).toHaveValue("20");
  const duplicate = await request.patch(`${API}${writes[0].path}`, { headers: {
    ...headers, "x-request-id": writes[0].headers["x-request-id"], "idempotency-key": writes[0].headers["idempotency-key"] }, data: { capitalUsd: 90 } });
  expect(duplicate.ok()).toBe(true); expect(await duplicate.json()).toEqual(writes[0].result);
  expect((await current()).config.capitalUsd).toBe(20);
  await protection(page);
  const takeProfit = page.locator(".automation-protection-toggle").filter({ hasText: "止盈" }).getByRole("checkbox");
  await takeProfit.check();
  await page.getByLabel("最低净利润 USD", { exact: true }).fill("0.25");
  await page.getByRole("button", { name: "保存退出保护", exact: true }).click();
  await expect(page.locator(".automation-protection-message")).toHaveText("退出保护已保存");
  await page.getByRole("button", { name: "启动模拟自动化", exact: true }).click();
  await expect(page.getByRole("button", { name: "暂停模拟新入场", exact: true })).toBeEnabled();
  lost = "/api/automation/control";
  await page.getByRole("button", { name: "暂停模拟新入场", exact: true }).click();
  await expect.poll(() => writes.at(-1)?.result?.config.paused).toBe(true);
  await page.reload(); release(); await recheck(page).click();
  await expect(page.locator(".automation-command-status strong")).toHaveText("已暂停");
  await page.getByRole("button", { name: "恢复模拟自动提交", exact: true }).click();
  await expect(page.getByRole("button", { name: "暂停模拟新入场", exact: true })).toBeEnabled();
  await page.getByRole("button", { name: "立即急停", exact: true }).click();
  await expect(page.locator(".automation-command-status strong")).toHaveText("已关闭");
  const actions = await (await request.get(`${API}/api/trading/action-runs`, { headers })).json();
  for (const write of writes) {
    const match = actions.data.filter((row: any) => row.requestId === write.headers["x-request-id"]);
    expect(match).toHaveLength(1); expect(match[0].status).toBe("succeeded");
    expect(match[0].result).toEqual(write.result);
  }
  expect(writes).toHaveLength(6);
  expect((await current()).activeRunCount).toBe(0);
  expect(errors).toEqual([]); expect(unexpected).toEqual([]);
});
