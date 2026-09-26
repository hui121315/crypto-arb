import { expect, test, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";

const API = "http://127.0.0.1:18000", WEB = "http://127.0.0.1:18080";
const headers = { Authorization: "Bearer isolated-paper-browser" };

test("real BP single monitor quotes, recovers lost saves, rejects stale versions and stops pending work", async ({ page, request }) => {
  test.setTimeout(60_000);
  const errors: string[] = [], unexpected: string[] = [], writes: any[] = [];
  let loseReply = true, release = () => {};
  const current = async () => (await request.get(API + "/api/stocks/peer/plans", { headers })).json();
  const stats = async () => (await request.get(API + "/__paper/stocks", { headers })).json();
  const post = (key: string, data: any) => request.post(API + "/api/stocks/monitor", {
    headers: { ...headers, "x-request-id": key, "idempotency-key": key }, data,
  });
  await page.addInitScript(api => {
    localStorage.setItem("api_base", JSON.stringify(api));
    localStorage.setItem("api_auth_token", JSON.stringify("isolated-paper-browser"));
  }, API);
  page.on("pageerror", error => errors.push(error.message));
  await page.route("**/*", async route => {
    const req = route.request(), url = new URL(req.url());
    if (![API, WEB].includes(url.origin) || (!["GET", "HEAD"].includes(req.method())
      && !["/api/auth/ws-ticket", "/api/stocks/watch", "/api/stocks/quote", "/api/stocks/monitor"].includes(url.pathname))) {
      unexpected.push(`${req.method()} ${url.pathname}`); return route.abort();
    }
    if (url.pathname === "/api/stocks/monitor" && req.method() === "POST") {
      const entry = { body: req.postDataJSON(), headers: req.headers(), status: 0, result: null as any };
      writes.push(entry);
      const response = await route.fetch();
      entry.status = response.status(); entry.result = await response.json();
      if (loseReply) {
        loseReply = false;
        await new Promise<void>(resolve => release = resolve);
        return route.abort().catch(() => {});
      }
      return route.fulfill({ response });
    }
    return route.continue();
  });
  await page.goto("/#stocks");
  await page.getByRole("button", { name: "选择股票", exact: true }).click();
  await page.locator(".stock-security").filter({ hasText: "MU" }).click();
  await page.getByRole("button", { name: "完成选择", exact: true }).click();
  const controls = page.getByRole("region", { name: "当前股票询价参数" });
  const amount = controls.getByLabel("链买预算 USDC", { exact: true });
  const toggle = controls.getByRole("checkbox", { name: "持续询价", exact: true });
  const recovery = page.getByRole("alert", { name: "设置操作待核对" });
  await amount.fill("12.345");
  await controls.getByRole("button", { name: "更新询价", exact: true }).click();
  await expect.poll(async () => (await current()).comparison?.sell?.router).toBe("isolated-fixture");
  await expect(amount).toBeEnabled();
  expect((await current()).comparison.budgetUsdc).toBe("12.345");
  await toggle.click();
  await expect.poll(() => writes[0]?.result?.enabled).toBe(true);
  const original = writes[0];
  expect(original.result.request.budgetUsdc).toBe("12.345");
  await page.reload(); release();
  await expect(recovery).toContainText("保存股票单股监控结果待核对");
  await expect(toggle).toBeDisabled();
  await expect(amount).toBeDisabled();
  // A different client stops before the first client's lost receipt is recovered.
  const newer = await post("bp-monitor-newer", { expectedRevision: original.result.revision,
    request: { ...original.body.request, enabled: false, quote: { ...original.body.request.quote, budgetUsdc: "invalid" } } });
  expect(newer.ok()).toBe(true);
  expect((await newer.json()).request).toEqual(original.result.request);
  await recovery.getByRole("button", { name: "核对上次操作", exact: true }).click();
  await expect(recovery).toBeHidden();
  await expect(toggle).not.toBeChecked();
  await expect(controls).toContainText("持续询价已关闭");
  expect(writes).toHaveLength(1);
  const replay = await request.post(API + "/api/stocks/monitor", {
    headers: { ...headers, "x-request-id": original.headers["x-request-id"], "idempotency-key": original.headers["idempotency-key"] },
    data: { ...original.body, request: { ...original.body.request, quote: { ...original.body.request.quote, budgetUsdc: "999" } } },
  });
  expect(replay.ok()).toBe(true); expect(await replay.json()).toEqual(original.result);
  expect((await current()).monitor.enabled).toBe(false);
  const stale = await post("bp-monitor-stale", original.body);
  expect(stale.status()).toBe(409);
  expect((await stale.json()).error.code).toBe("STOCK_MONITOR_CHANGED");
  await amount.fill("invalid");
  await toggle.click();
  await expect.poll(() => writes[1]?.status).toBe(400);
  await expect(amount).toBeEnabled();
  await expect(toggle).not.toBeChecked();
  await expect(recovery).toBeHidden();
  await amount.fill("25.5");
  await toggle.click();
  await expect(toggle).toBeChecked();
  await expect.poll(async () => (await current()).monitor.completedQuotes, { timeout: 15_000 }).toBeGreaterThan(0);
  const ready = await current();
  expect(ready.comparison.budgetUsdc).toBe("25.5");
  expect(ready.comparison.sell.router).toBe("isolated-fixture");
  await request.post(API + "/__paper/stocks", { headers, data: true });
  const before = (await stats()).quotes.length;
  await expect.poll(async () => (await stats()).quotes.length, { timeout: 10_000 }).toBeGreaterThan(before);
  await amount.fill("broken draft");
  await toggle.click();
  await expect(controls).toContainText("持续询价已关闭");
  await expect(toggle).not.toBeChecked();
  expect(writes.at(-1).body.request.quote.budgetUsdc).toBe("25.5");
  const stopped = await current();
  expect(stopped.monitor.enabled).toBe(false);
  const stoppedCount = (await stats()).quotes.length;
  await request.post(API + "/__paper/stocks", { headers, data: false });
  await page.waitForTimeout(800);
  expect((await stats()).quotes.length).toBe(stoppedCount);
  // A shared configuration version admits one of two concurrent writers, not both.
  const outcomes = await Promise.all(["a", "b"].map(key => post(`bp-monitor-race-${key}`, {
    expectedRevision: stopped.monitor.revision,
    request: { ...original.body.request, enabled: true, quote: { ...original.body.request.quote, budgetUsdc: key === "a" ? "11" : "12" } },
  })));
  expect(outcomes.map(r => r.status()).sort()).toEqual([200, 409]);
  const latest = await current();
  expect((await post("bp-monitor-final-stop", { expectedRevision: latest.monitor.revision,
    request: { enabled: false, quote: latest.monitor.request, alerts: latest.monitor.alerts } })).ok()).toBe(true);
  await expect(toggle).not.toBeChecked();
  const runs = (await (await request.get(API + "/api/trading/action-runs", { headers })).json()).data;
  const run = runs.find((r: any) => r.requestId === original.headers["x-request-id"]);
  expect(run).toMatchObject({ kind: "stock_monitor_update", target: "MU.US", status: "succeeded" });
  expect(run.result).toEqual(original.result);
  expect(Object.keys(run.result).sort()).toEqual(["alerts", "asset", "enabled", "observedAtMs", "request", "revision"]);
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await controls.scrollIntoViewIfNeeded();
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
    await page.screenshot({ path: test.info().outputPath(`bp-monitor-stopped-${width}.png`) });
  }
  expect((await stats()).unexpected).toBe(0);
  expect(errors).toEqual([]); expect(unexpected).toEqual([]);
});

async function changeLogin(page: Page, token: string) {
  await page.getByRole("button", { name: "切换到设置", exact: true }).click();
  await page.getByRole("tab", { name: "诊断", exact: true }).click();
  await page.getByRole("tab", { name: "连接", exact: true }).click();
  await page.locator(".settings-api-token-task input").fill(token);
  await page.getByRole("button", { name: "保存 Token", exact: true }).click();
  await page.getByRole("button", { name: "切换到股票套利", exact: true }).click();
}

test("BP single quote rejects mismatched replies and isolates late responses across stock and login changes", async ({ page }) => {
  const source = JSON.parse(readFileSync(new URL("../../shared-types/fixtures/stocks_plan_build.json", import.meta.url), "utf8"));
  const now = source.observedAtMs;
  let revision = now, release = () => {}, held = false, mismatch = false;
  let selected = source.security.asset;
  const errors: string[] = [], writes: any[] = [];
  const stocks = [source.security, { ...source.security, asset: "SNDK.US", ticker: "SNDK", name: "Sandisk" }];
  const snapshot = () => ({ ...source, security: stocks.find(s => s.asset === selected),
    comparison: null, plans: [], observedAtMs: ++revision,
    batch: { revision: "fixture-batch", request: null, running: false, waitingForViewers: false,
      rows: [], completedRounds: 0, nextAtMs: null, metadataAtMs: null, problem: null } });
  await page.clock.setFixedTime(now);
  await page.addInitScript(api => {
    localStorage.setItem("api_base", JSON.stringify(api));
    localStorage.setItem("api_auth_token", JSON.stringify("quote-original"));
  }, API);
  page.on("pageerror", error => errors.push(error.message));
  await page.routeWebSocket(/.*/, socket => socket.close());
  await page.route("**/*", async route => {
    const req = route.request(), url = new URL(req.url());
    if (url.origin === WEB) return route.continue();
    if (url.origin !== API) return route.abort();
    const json = (body: any, status = 200) => route.fulfill({ status, json: body });
    if (url.pathname === "/api/auth/ws-ticket") return json({ ticket: "fixture", expiresAtMs: now + 60_000 });
    if (url.pathname === "/api/stocks/catalog") return json({ rows: stocks, observedAtMs: ++revision });
    if (url.pathname === "/api/stocks/peer/plans") return json(snapshot());
    if (req.method() === "POST" && ["/api/stocks/watch", "/api/stocks/quote"].includes(url.pathname)) {
      const body = req.postDataJSON(); writes.push({ path: url.pathname, body, auth: req.headers().authorization });
      if (url.pathname === "/api/stocks/watch") { selected = body.asset; return json(snapshot()); }
      const reply = { ...snapshot(), comparison: { ...source.comparison, budgetUsdc: body.budgetUsdc, keyed: body.keyed } };
      if (held) { await new Promise<void>(resolve => release = resolve); held = false; }
      reply.observedAtMs = ++revision;
      if (mismatch) reply.security = stocks[1];
      return json(reply);
    }
    return json({ error: { code: "FIXTURE_NOT_CONFIGURED", message: "isolated fixture", status: 404 } }, 404);
  });
  await page.goto("/#stocks");
  const controls = page.getByRole("region", { name: "当前股票询价参数" });
  const quote = () => controls.getByRole("button", { name: "更新询价", exact: true }).click();
  const amount = controls.getByLabel("链买预算 USDC", { exact: true });
  await amount.fill("10.25");
  mismatch = true;
  await quote();
  await expect(page.getByRole("alert").filter({ hasText: "询价回复与当前股票或参数不匹配" })).toBeVisible();
  await expect(page.locator(".stock-heading h2")).toHaveText("MU");
  await expect(controls).toContainText("尚无链上报价");
  mismatch = false; held = true;
  await quote();
  await expect(amount).toBeDisabled();
  await page.getByRole("button", { name: "选择股票", exact: true }).click();
  await page.locator(".stock-security").filter({ hasText: "SNDK" }).click();
  await page.locator(".stock-security").filter({ hasText: "MU" }).click();
  await page.getByRole("button", { name: "完成选择", exact: true }).click();
  const oldQuote = page.waitForResponse("**/api/stocks/quote");
  release(); await (await oldQuote).finished();
  await expect(controls).toContainText("尚无链上报价");
  await expect(amount).toBeEnabled();
  held = true;
  await quote();
  await expect(amount).toBeDisabled();
  await changeLogin(page, "quote-other");
  const oldLogin = page.waitForResponse("**/api/stocks/quote");
  release(); await (await oldLogin).finished();
  await expect(amount).toBeEnabled();
  await expect(controls).toContainText("尚无链上报价");
  await quote();
  await expect(amount).toBeEnabled();
  await expect(controls).not.toContainText("尚无链上报价");
  expect(writes.filter(w => w.path.endsWith("/quote")).map(w => w.auth))
    .toEqual(["Bearer quote-original", "Bearer quote-original", "Bearer quote-original", "Bearer quote-other"]);
  expect(errors).toEqual([]);
});
