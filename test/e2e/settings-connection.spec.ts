import { expect, test, type Page } from "@playwright/test";
import { settingsAccountFixture } from "./fixtures/settings-account";
import { API } from "./fixtures/opportunity-workbench";

const original = "isolated-fixture-token";
const changed = "settings-other-login";
const envPath = "/api/trading/credentials/env-template";
const tokenInput = (page: Page) => page.locator(".settings-api-token-task input");
const feedback = (page: Page) => page.locator(".settings-content-panel .settings-message[role=status]").first();
async function token(page: Page, value: string) {
  await page.getByRole("tab", { name: "连接", exact: true }).click();
  await tokenInput(page).fill(value);
  await page.getByRole("button", { name: "保存 Token", exact: true }).click();
}

test("connection storage failures preserve runtime and draft, successful retry survives reload", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const f = await settingsAccountFixture(page, "diagnostics");
  const requests: { path: string; auth: string }[] = [];
  page.on("request", request => {
    const url = new URL(request.url());
    if (url.origin === API) requests.push({ path: url.pathname, auth: request.headers().authorization ?? "" });
  });
  // A second local API base exercises address changes without contacting a real server.
  await page.route(`${API}/e2e-connection/**`, async route => {
    const req = route.request();
    if (req.method() !== "GET" && !req.url().endsWith("/api/auth/ws-ticket")) return route.abort();
    const response = await route.fetch({ url: req.url().replace("/e2e-connection", "") });
    await route.fulfill({ response });
  });
  await page.goto("/#settings");
  await expect(tokenInput(page)).toBeVisible();
  await page.evaluate(() => {
    const set = Storage.prototype.setItem, remove = Storage.prototype.removeItem;
    (window as any).storageFailure = "token-write";
    Storage.prototype.setItem = function (key, value) {
      const mode = (window as any).storageFailure;
      if ((mode === "token-write" && key === "api_auth_token") || (mode === "base-write" && key === "api_base"))
        throw new Error(`fixture storage denied: ${value}`);
      if (mode === "token-silent" && key === "api_auth_token") return;
      return set.call(this, key, value);
    };
    Storage.prototype.removeItem = function (key) {
      if ((window as any).storageFailure === "token-remove" && key === "api_auth_token")
        throw new Error("fixture remove denied");
      return remove.call(this, key);
    };
  });
  await token(page, changed);
  await expect(feedback(page)).toContainText("当前连接未切换");
  await expect(tokenInput(page)).toHaveValue(changed);
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem("api_auth_token")!))).toBe(original);
  await expect(page.locator("body")).not.toContainText(changed);
  const first = requests.length;
  await page.getByRole("button", { name: "刷新全部诊断", exact: true }).click();
  await expect.poll(() => requests.slice(first).filter(r => r.path === envPath).length).toBe(1);
  expect(requests.slice(first).filter(r => r.path === envPath).every(r => r.auth === `Bearer ${original}`)).toBe(true);
  await page.evaluate(() => { (window as any).storageFailure = "token-silent"; });
  await page.getByRole("button", { name: "保存 Token", exact: true }).click();
  await expect(feedback(page)).toContainText("未能确认浏览器保存");
  await expect(tokenInput(page)).toHaveValue(changed);
  await page.evaluate(() => { (window as any).storageFailure = ""; });
  const second = requests.length;
  await page.getByRole("button", { name: "保存 Token", exact: true }).click();
  await expect(feedback(page)).toContainText("Token 已应用");
  await expect(tokenInput(page)).toHaveValue("");
  await expect.poll(() => requests.slice(second).filter(r => r.path === envPath && r.auth === `Bearer ${changed}`).length).toBe(1);
  await page.evaluate(() => { (window as any).storageFailure = "token-remove"; });
  await page.getByRole("button", { name: "清空 Token", exact: true }).click();
  await expect(feedback(page)).toContainText("当前连接未切换");
  await expect(page.locator(".settings-api-token-task header")).toContainText("Token 已填写");
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem("api_auth_token")!))).toBe(changed);
  await page.evaluate(() => { (window as any).storageFailure = ""; });
  await page.getByRole("button", { name: "清空 Token", exact: true }).click();
  await expect(feedback(page)).toContainText("Token 已清空");
  expect(await page.evaluate(() => localStorage.getItem("api_auth_token"))).toBeNull();
  await expect(page.getByRole("button", { name: "清空 Token", exact: true })).toBeDisabled();
  await token(page, changed);
  const address = page.getByRole("textbox", { name: "API Base", exact: true });
  const confirmation = page.getByRole("textbox", { name: "确认应用", exact: true });
  const next = `${API}/e2e-connection`;
  await address.fill(`${next}///`);
  await confirmation.fill("apply");
  await page.evaluate(() => { (window as any).storageFailure = "base-write"; });
  await page.getByRole("button", { name: "保存并应用", exact: true }).click();
  await expect(feedback(page)).toContainText("当前连接未切换");
  await expect(address).toHaveValue(`${next}///`);
  await expect(confirmation).toHaveValue("apply");
  await expect(page.locator(".settings-api-task").first().locator("header em")).toHaveText(API);
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem("api_base")!))).toBe(API);
  await page.screenshot({ path: test.info().outputPath("connection-storage-desktop.png"), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  const save = page.getByRole("button", { name: "保存并应用", exact: true });
  await save.scrollIntoViewIfNeeded();
  expect(await save.evaluate(el => {
    const r = el.getBoundingClientRect();
    return el.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2));
  })).toBe(true);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
  await page.screenshot({ path: test.info().outputPath("connection-storage-mobile.png"), fullPage: true });
  await page.evaluate(() => { (window as any).storageFailure = ""; });
  await save.click();
  await expect(page.locator(".settings-api-task").first().locator("header em")).toHaveText(next);
  await expect.poll(() => requests.some(r => r.path === `/e2e-connection${envPath}` && r.auth === `Bearer ${changed}`)).toBe(true);
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem("api_base")!))).toBe(next);
  await address.fill(API);
  await confirmation.fill("apply");
  await save.click();
  await page.reload();
  await expect(page.locator(".settings-api-token-task header")).toContainText("Token 已填写");
  await expect(tokenInput(page)).toHaveValue("");
  const afterReload = requests.length;
  await page.getByRole("button", { name: "刷新全部诊断", exact: true }).click();
  await expect.poll(() => requests.slice(afterReload).some(r => r.path === envPath && r.auth === `Bearer ${changed}`)).toBe(true);
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});

test("settings reads reject old login replies including A-B-A, and environment waits for current evidence", async ({ page }) => {
  const f = await settingsAccountFixture(page, "execution");
  const template = await (await page.request.get(`${API}${envPath}`)).json();
  const watchlist = await (await page.request.get(`${API}/e2e-watchlist-alerts-runtime/api/watchlist`)).json();
  const alerts = await (await page.request.get(`${API}/e2e-watchlist-alerts-runtime/api/alerts/rules`)).json();
  const pending: { path: string; auth: string; release: () => void }[] = [];
  let hold = false, generation = 0, fail = false;
  const reads: { path: string; auth: string }[] = [];
  await page.route("**/api/**", async route => {
    const req = route.request(), url = new URL(req.url());
    if (url.origin !== API || req.method() !== "GET" || ![envPath, "/api/v1/spot/ticks", "/api/trading/adapters", "/api/watchlist", "/api/alerts/rules"].includes(url.pathname))
      return route.fallback();
    const auth = req.headers().authorization ?? "";
    reads.push({ path: url.pathname, auth });
    const isOther = auth === `Bearer ${changed}`;
    if (url.pathname === "/api/watchlist" && auth === "Bearer disabled-login")
      return route.fulfill({ status: 404, json: { error: { code: "NOT_ENABLED", message: "fixture optional route disabled" } } });
    const snapshot = structuredClone(url.pathname === "/api/watchlist" ? watchlist
      : url.pathname === "/api/alerts/rules" ? alerts
      : url.pathname.endsWith("/adapters") ? f.adapters : template);
    snapshot.text = `${isOther ? "OTHER" : "ORIGINAL"}-GEN-${generation}`;
    if (snapshot.items) snapshot.items[0].symbol = `WATCH-${snapshot.text}`;
    if (isOther && snapshot.options) {
      const live = snapshot.options.find((row: any) => row.environment === "live");
      live.enabled = false; live.disabledReason = "fixture current login has no live permission";
    }
    const failure = fail;
    if (hold || url.pathname === "/api/v1/spot/ticks")
      await new Promise<void>(release => pending.push({ path: url.pathname, auth, release }));
    if (failure || url.pathname === "/api/v1/spot/ticks") return route.fulfill({ status: 503, json: { error: {
      code: "OLD_OR_FAILED_READ", message: "fixture read unavailable", source: "fixture.settings" } } });
    await route.fulfill({ json: snapshot });
  });
  await page.goto("/#settings");
  const enable = page.getByRole("button", { name: "启用实盘", exact: true });
  await expect(enable).toBeEnabled();
  await page.getByRole("tab", { name: "诊断", exact: true }).click();
  await page.getByRole("tab", { name: "运行数据依据", exact: true }).click();
  const env = page.locator(".env-template-box textarea");
  await expect(env).toHaveValue("ORIGINAL-GEN-0");
  const evidence = page.getByRole("tabpanel", { name: "运行数据依据诊断", exact: true });
  await expect(evidence).toContainText("WATCH-ORIGINAL-GEN-0");
  hold = true;
  await page.getByRole("button", { name: "刷新全部诊断", exact: true }).click();
  await expect.poll(() => pending.length).toBe(1);
  await page.getByRole("tab", { name: "行情", exact: true }).last().click();
  await page.getByRole("textbox", { name: "Symbol（可空=全部）", exact: true }).fill("BTC");
  await page.getByRole("button", { name: "查询 Spot Ticks", exact: true }).click();
  await expect.poll(() => pending.length).toBe(2);
  await token(page, changed);
  await expect.poll(() => pending.filter(r => r.auth === `Bearer ${changed}`).length).toBe(2);
  await page.getByRole("tab", { name: "运行数据依据", exact: true }).click();
  await expect(env).toHaveCount(0);
  await expect(page.getByText("正在生成 .env 模板", { exact: true })).toBeVisible();
  await expect(evidence).not.toContainText("WATCH-ORIGINAL-GEN-0");
  await expect(page.getByText("正在检测自选与提醒功能", { exact: true })).toBeVisible();
  hold = false; generation = 2;
  await token(page, original);
  await page.getByRole("tab", { name: "运行数据依据", exact: true }).click();
  await expect(env).toHaveValue("ORIGINAL-GEN-2");
  await expect(evidence).toContainText("WATCH-ORIGINAL-GEN-2");
  for (const item of pending.splice(0)) {
    const reply = page.waitForResponse(r => new URL(r.url()).pathname === item.path
      && r.request().headers().authorization === item.auth);
    item.release(); await (await reply).finished();
  }
  await expect(env).toHaveValue("ORIGINAL-GEN-2");
  await expect(evidence).toContainText("WATCH-ORIGINAL-GEN-2");
  await expect(evidence).not.toContainText("WATCH-OTHER-GEN-0");
  expect(reads.some(r => r.path === "/api/alerts/rules" && r.auth === `Bearer ${changed}`)).toBe(false);
  const push = structuredClone(watchlist);
  push.items[0].symbol = "CURRENT-WS-WATCH";
  await expect.poll(() => f.channelSockets.get("watchlist")?.size ?? 0).toBeGreaterThan(0);
  for (const socket of f.channelSockets.get("watchlist")!) socket.send(JSON.stringify({
    type: "message", channel: "watchlist", payload: { event: "watchlist_changed", envelope: push, timestampMs: Date.now() },
  }));
  await expect(evidence).toContainText("CURRENT-WS-WATCH");
  for (const socket of f.channelSockets.get("alerts")!) socket.send(JSON.stringify({
    type: "message", channel: "alerts", payload: { event: "alert_triggered", notification: {
      id: "current-alert", ruleId: 101, watchlistId: 41, opportunityId: "current-opportunity",
      symbol: "CURRENT-WS-ALERT", strategy: "perp_cross", longExchange: "binance", shortExchange: "okx",
      oneCycleNetBps: 4, netSingleYield: 0.04, queuedAtMs: Date.now(),
    } },
  }));
  await expect(evidence).toContainText("CURRENT-WS-ALERT");
  await expect(page.locator(".toast-stack")).toContainText("CURRENT-WS-ALERT");
  await page.getByRole("tab", { name: "行情", exact: true }).last().click();
  await expect(page.getByText("尚未发起 spot 手动查询", { exact: true })).toBeVisible();
  await expect(page.getByRole("tabpanel", { name: "行情诊断", exact: true })).not.toContainText("OLD_OR_FAILED_READ");
  await token(page, changed);
  await page.getByRole("tab", { name: "运行数据依据", exact: true }).click();
  await expect(env).toHaveValue("OTHER-GEN-2");
  await expect(evidence).toContainText("WATCH-OTHER-GEN-2");
  await expect(evidence).not.toContainText("CURRENT-WS-ALERT");
  fail = true;
  await page.getByRole("button", { name: "刷新全部诊断", exact: true }).click();
  await expect(page.getByRole("tabpanel", { name: "运行数据依据诊断", exact: true })).toContainText(".env 模板刷新失败");
  await expect(env).toHaveValue("OTHER-GEN-2");
  fail = false; generation = 3;
  await page.getByRole("button", { name: "刷新全部诊断", exact: true }).click();
  await expect(env).toHaveValue("OTHER-GEN-3");
  hold = true;
  await page.getByRole("tab", { name: "执行环境", exact: true }).click();
  await expect.poll(() => pending.length).toBe(1);
  await expect(page.locator(".settings-environment-state")).toContainText("状态待确认");
  await expect(enable).toBeDisabled();
  const item = pending.shift()!;
  const response = page.waitForResponse(r => new URL(r.url()).pathname === item.path);
  item.release(); await (await response).finished();
  await expect(page.locator(".settings-environment-state")).toContainText("模拟");
  await expect(enable).toBeDisabled();
  await expect(enable).toHaveAttribute("title", "fixture current login has no live permission");
  expect(reads.filter(r => r.path.endsWith("/adapters")).map(r => r.auth)).toEqual([`Bearer ${original}`, `Bearer ${changed}`]);
  hold = false;
  await page.getByRole("tab", { name: "诊断", exact: true }).click();
  await token(page, "disabled-login");
  await page.getByRole("tab", { name: "运行数据依据", exact: true }).click();
  await expect(page.getByText("自选与提醒为可选功能，当前未启用", { exact: true })).toBeVisible();
  await token(page, changed);
  await page.getByRole("tab", { name: "运行数据依据", exact: true }).click();
  await expect(evidence).toContainText("WATCH-OTHER-GEN-3");
  expect(f.calls.filter(r => r.key.startsWith("POST"))).toEqual([]);
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});
