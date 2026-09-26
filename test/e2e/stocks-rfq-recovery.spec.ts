import { expect, test, type Page, type WebSocketRoute } from "@playwright/test";

// Isolated public-shaped fixtures: no exchange, wallet, or notification traffic.
const API = "http://127.0.0.1:18997";
const session = { name: "Regular", minQuantity: "0.01", maxQuantity: "10", stepSize: "0.01" };
const securities = ["MU", "SNDK"].map((ticker) => ({
  asset: `${ticker}.US`, ticker, name: ticker === "MU" ? "Micron" : "Sandisk",
  cusip: ticker === "MU" ? "595112103" : "80004C200", sessions: [session],
  orderBooks: [], rfqSymbol: `${ticker}.US_USDC_RFQ`,
}));

async function setup(page: Page) {
  const errors: string[] = [];
  const writes: { path: string; body: any }[] = [];
  const sockets = new Set<WebSocketRoute>();
  let selected = securities[0];
  let records: any[] = [];
  let failSubmit = true;
  let omitReceipt = false;
  let revision = Date.now();
  const snapshot = () => ({
    security: selected, tokens: [], books: [], connected: true,
    reference: null, problem: null, tokenMetadataAtMs: null, tokenMetadataProblem: null,
    rfqs: records, rfqConnected: true, observedAtMs: ++revision,
    tradingRoute: { kind: "rfq", session, symbol: selected.rfqSymbol, reason: "fixture",
      timezone: "America/New_York", calendarAtMs: Date.now(), validUntilMs: Date.now() + 600_000 },
  });
  const record = (request: any) => ({
    request: { ...request, quantity: Number(request.quantity).toString() },
    clientId: 42, accountFingerprint: "isolated-fixture", symbol: `${request.asset}_USDC_RFQ`,
    rfqId: "1001", phase: "awaiting_quotes", needsRecheck: false, cancelRequested: false,
    createdAtMs: Date.now(), updatedAtMs: Date.now(),
  });
  const publish = () => {
    const frame = JSON.stringify({ type: "message", channel: "stocks", payload: snapshot() });
    for (const socket of sockets) socket.send(frame);
  };
  page.on("pageerror", (e) => errors.push(e.message));
  await page.addInitScript((api) => {
    localStorage.setItem("api_base", JSON.stringify(api));
    localStorage.setItem("api_auth_token", JSON.stringify("isolated-fixture-token"));
  }, API);
  await page.routeWebSocket(/.*/, (socket) => {
    if (!socket.url().startsWith(API.replace("http:", "ws:"))) {
      socket.close();
      return;
    }
    socket.onMessage((raw) => {
      const msg = JSON.parse(raw.toString());
      if (msg.type === "subscribe") {
        socket.send(JSON.stringify({ type: "ack", subscribed: msg.channels }));
        if (msg.channels.includes("stocks")) {
          sockets.add(socket);
          publish();
        }
      } else if (msg.type === "ping") socket.send(JSON.stringify({ type: "pong" }));
    });
    socket.onClose(() => sockets.delete(socket));
  });
  await page.route("**/*", async (route) => {
    const req = route.request();
    const url = new URL(req.url());
    if (!req.url().startsWith(API)) {
      if (url.hostname === "127.0.0.1" && !url.pathname.startsWith("/api/")) return route.continue();
      return route.abort();
    }
    const path = url.pathname;
    const json = (body: any, status = 200) => route.fulfill({ status, json: body });
    if (req.method() === "OPTIONS") return route.fulfill({ status: 204 });
    if (path === "/api/auth/ws-ticket") return json({ ticket: "fixture", expiresAtMs: Date.now() + 60_000 });
    if (path === "/api/stocks/catalog") return json({ rows: securities, observedAtMs: Date.now() });
    if (path === "/api/stocks/peer-markets") return json({ rows: [], observedAtMs: Date.now() });
    if (path === "/api/stocks" && req.method() === "GET") return json(snapshot());
    if (req.method() === "POST" && path.startsWith("/api/stocks/")) {
      const body = req.postDataJSON();
      writes.push({ path, body });
      if (path === "/api/stocks/watch") {
        selected = securities.find((s) => s.asset === body.asset)!;
        return json(snapshot());
      }
      if (path === "/api/stocks/rfq") {
        if (failSubmit) return route.abort("connectionreset");
        if (!omitReceipt) records = [record(body)];
        return json(snapshot());
      }
      if (path === "/api/stocks/rfq/recheck") return json(snapshot());
      if (path === "/api/stocks/rfq/finish-unsent") {
        if (!records.some((r) => r.request.requestId === body.requestId)) {
          records = [{ ...record(body), clientId: 0, rfqId: null, phase: "not_sent", symbol: "", accountFingerprint: "" }];
        }
        return json(snapshot());
      }
      if (path === "/api/stocks/rfq/cancel") {
        records = records.map((r) => r.request.requestId === body.requestId ? { ...r, phase: "cancelled" } : r);
        return json(snapshot());
      }
      throw new Error(`Unexpected stock mutation: ${path}`);
    }
    return json({ error: { code: "FIXTURE_NOT_CONFIGURED", message: "isolated fixture", status: 404 } }, 404);
  });
  return {
    errors, writes, publish, record,
    submit: (fail: boolean, omit = false) => { failSubmit = fail; omitReceipt = omit; },
    records: (rows: any[]) => { records = rows; publish(); },
  };
}

test("RFQ lost receipt survives selection and reload without changing the original request", async ({ page }) => {
  const f = await setup(page);
  await page.goto("/#stocks");
  await page.getByRole("button", { name: "询价与执行", exact: true }).click();
  const rfq = page.getByRole("region", { name: "Backpack 股票 询价" });
  await rfq.getByLabel("询价股数").fill("1.00");
  await rfq.getByRole("button", { name: "发送询价（不成交）", exact: true }).click();
  await expect(rfq.getByText("上次请求尚未取得处理结果", { exact: false })).toBeVisible();
  const original = f.writes.find((w) => w.path === "/api/stocks/rfq")!.body;
  await expect(rfq.getByRole("button", { name: "重试原询价", exact: true })).toBeEnabled();
  await expect(rfq.getByLabel("询价股数")).toBeDisabled();
  await expect(rfq.getByLabel("交易所方向")).toBeDisabled();
  await page.getByRole("button", { name: "选择股票", exact: true }).click();
  const sndk = page.locator(".stock-security").filter({ hasText: "SNDK" });
  await sndk.click();
  await expect(sndk).toHaveAttribute("aria-pressed", "true");
  await page.reload();
  await expect(sndk).toHaveAttribute("aria-pressed", "true");
  await page.getByRole("button", { name: "询价与执行", exact: true }).click();
  await expect(rfq.locator(".stock-rfq-attempt")).toContainText("MU.US");
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await rfq.scrollIntoViewIfNeeded();
    expect(await rfq.evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
    await rfq.screenshot({ path: test.info().outputPath(`rfq-recovery-${width}.png`) });
  }
  f.submit(false);
  await rfq.getByRole("button", { name: "重试原询价", exact: true }).click();
  await expect(rfq.locator(".stock-rfq-record")).toContainText("MU.US · 卖 1 股");
  expect(f.writes.filter((w) => w.path === "/api/stocks/rfq").map((w) => w.body)).toEqual([original, original]);
  await expect(rfq.getByLabel("询价股数")).toBeEnabled();
  await rfq.getByRole("button", { name: "取消询价", exact: true }).click();
  await expect(rfq.locator(".stock-rfq-record")).toContainText("已取消");
  await page.reload();
  await page.getByRole("button", { name: "询价与执行", exact: true }).click();
  await expect(rfq.locator(".stock-rfq-record")).toContainText("已取消");
  expect(f.errors).toEqual([]);
  expect(f.writes.every((w) => ["/api/stocks/watch", "/api/stocks/rfq", "/api/stocks/rfq/cancel"].includes(w.path))).toBe(true);
});

test("RFQ only a matching receipt unlocks the draft, not HTTP 200 or a different request", async ({ page }) => {
  const f = await setup(page);
  f.submit(false, true);
  await page.goto("/#stocks");
  await page.getByRole("button", { name: "询价与执行", exact: true }).click();
  const rfq = page.getByRole("region", { name: "Backpack 股票 询价" });
  await rfq.getByLabel("询价股数").fill("1.00");
  await rfq.getByRole("button", { name: "发送询价（不成交）", exact: true }).click();
  await expect(rfq.locator(".stock-rfq-attempt")).toBeVisible();
  const original = f.writes.find((w) => w.path === "/api/stocks/rfq")!.body;
  f.records([f.record({ ...original, quantity: "2" })]);
  await expect(rfq.locator(".stock-rfq-record")).toContainText("2 股");
  await expect(rfq.getByLabel("询价股数")).toBeDisabled();
  f.records([f.record(original)]);
  await expect(rfq.getByLabel("询价股数")).toBeEnabled();
  await expect(rfq.locator(".stock-rfq-attempt")).toHaveCount(0);
  expect(f.writes.filter((w) => w.path === "/api/stocks/rfq")).toHaveLength(1);
  expect(f.errors).toEqual([]);
});

test("RFQ invalid session quantity never creates an attempt or a network request", async ({ page }) => {
  const f = await setup(page);
  await page.goto("/#stocks");
  await page.getByRole("button", { name: "询价与执行", exact: true }).click();
  const rfq = page.getByRole("region", { name: "Backpack 股票 询价" });
  for (const quantity of ["0.001", "0.015", "11"]) {
    await rfq.getByLabel("询价股数").fill(quantity);
    await expect(rfq.getByRole("button", { name: "发送询价（不成交）", exact: true })).toBeDisabled();
  }
  await rfq.getByLabel("询价股数").fill("1.00");
  await expect(rfq.getByRole("button", { name: "发送询价（不成交）", exact: true })).toBeEnabled();
  expect(f.writes).toEqual([]);
  expect(f.errors).toEqual([]);
});

test("RFQ does not send when original-request storage is unavailable", async ({ page }) => {
  const f = await setup(page);
  await page.goto("/#stocks");
  await page.getByRole("button", { name: "询价与执行", exact: true }).click();
  const rfq = page.getByRole("region", { name: "Backpack 股票 询价" });
  await rfq.getByLabel("询价股数").fill("1");
  await page.evaluate(() => {
    const original = Storage.prototype.setItem;
    Storage.prototype.setItem = function (key, value) {
      if (key.startsWith("stocks.rfq.attempt.")) throw new DOMException("fixture quota", "QuotaExceededError");
      return original.call(this, key, value);
    };
  });
  await rfq.getByRole("button", { name: "发送询价（不成交）", exact: true }).click();
  await expect(page.getByRole("alert").filter({ hasText: "无法保存原询价" })).toBeVisible();
  await expect(rfq.getByLabel("询价股数")).toBeEnabled();
  expect(f.writes).toEqual([]);
  expect(f.errors).toEqual([]);
});

test("RFQ unsent failure can be ended before editing a new request", async ({ page }) => {
  const f = await setup(page);
  await page.goto("/#stocks");
  await page.getByRole("button", { name: "询价与执行", exact: true }).click();
  const rfq = page.getByRole("region", { name: "Backpack 股票 询价" });
  await rfq.getByLabel("询价股数").fill("1");
  await rfq.getByRole("button", { name: "发送询价（不成交）", exact: true }).click();
  await rfq.getByRole("button", { name: "结束未发送请求", exact: true }).click();
  await expect(rfq.locator(".stock-rfq-record")).toContainText("询价未发送");
  await expect(rfq.getByLabel("询价股数")).toBeEnabled();
  const original = f.writes.find((w) => w.path === "/api/stocks/rfq")!.body;
  expect(f.writes.find((w) => w.path === "/api/stocks/rfq/finish-unsent")!.body).toEqual(original);
  await rfq.getByLabel("询价股数").fill("2");
  f.submit(false);
  await rfq.getByRole("button", { name: "发送询价（不成交）", exact: true }).click();
  await expect(rfq.locator(".stock-rfq-record")).toContainText("2 股");
  const next = f.writes.filter((w) => w.path === "/api/stocks/rfq")[1].body;
  expect(next.requestId).not.toBe(original.requestId);
  expect(next.quantity).toBe("2");
  expect(f.errors).toEqual([]);
});
