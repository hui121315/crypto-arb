import { expect, test, type Page, type WebSocketRoute } from "@playwright/test";

const API = "http://127.0.0.1:18997";
const NOW = 1_790_000_000_000;
const USDC = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
const issuerMints: Record<string, string> = {
  MU: "MUxEsUKSMACyw5fZf68wxf5FLnZVhtU9CwH8uNNGay1",
  SNDK: "SNDKbwMUQvZhnLnxLduradgLHG5KrPuKwpnrkkGRhfH",
};
const securities = ["MU", "SNDK", "AAPL"].map((ticker, index) => ({
  asset: `${ticker}.US`, ticker, name: ["Micron Technology", "Sandisk", "Apple Inc."][index],
  cusip: ["595112103", "80004C200", "037833100"][index], sessions: [],
  orderBooks: [], rfqSymbol: `${ticker}.US_USDC_RFQ`,
}));

// All application traffic is intercepted, including WS; no credentials or funds are used.
async function setup(page: Page) {
  const writes: any[] = [];
  const errors: string[] = [];
  const sockets = new Set<WebSocketRoute>();
  let revision = NOW;
  let batchReply: Promise<void> | undefined;
  let rejectBatch = false;
  let loseBatchReply = false;
  let watchReply: Promise<void> | undefined;
  let watchOutcome: "ok" | "rejected" | "mismatch" = "ok";
  const actions: any[] = [];
  let security: typeof securities[number] | null = null;
  let batch: any = { revision: "fixture-start", request: null, running: false, waitingForViewers: false,
    rows: [], problem: null, completedRounds: 0, nextAtMs: null, metadataAtMs: null };
  const snapshot = () => ({ security, tokens: [], books: [], connected: false,
    reference: null, problem: null, observedAtMs: ++revision, batch });
  const row = (s: typeof securities[number]) => {
    const mint = issuerMints[s.ticker] ?? `fixture-${s.ticker}`;
    const buy = { inputMint: USDC, outputMint: mint, inputRaw: "100000000",
      outputRaw: "2100000", minimumOutputRaw: "2000000", router: "fixture",
      feeBps: null, feeMint: null, requestedAtMs: NOW, receivedAtMs: NOW,
      expiresAtMs: NOW + 10_000 };
    return { security: s, token: { blockchain: "Solana", contractAddress: mint,
      nativeDecimals: 6, depositEnabled: false, withdrawEnabled: null }, issuerVerified: s.ticker in issuerMints,
      mint: { address: mint, decimals: 6, uiMultiplier: "2", slot: 1,
        chainTimeMs: NOW, checkedAtMs: NOW, nextChangeAtMs: null, extensions: [] },
      buy, sell: { ...buy, inputMint: mint, outputMint: USDC, inputRaw: "2000000",
        outputRaw: "97000000", minimumOutputRaw: "96000000" }, books: [],
      connected: false, refreshing: false, problem: null, checkedAtMs: NOW };
  };
  const publish = () => {
    const frame = JSON.stringify({ type: "message", channel: "stocks", payload: snapshot() });
    for (const socket of sockets) socket.send(frame);
  };
  await page.clock.setFixedTime(NOW);
  await page.addInitScript((api) => {
    localStorage.setItem("api_base", JSON.stringify(api));
    localStorage.setItem("api_auth_token", JSON.stringify("isolated-fixture-token"));
  }, API);
  page.on("pageerror", (error) => errors.push(error.message));
  await page.routeWebSocket(/.*/, (socket) => {
    if (!socket.url().startsWith(API.replace("http:", "ws:"))) return socket.close();
    socket.onMessage((raw) => {
      const message = JSON.parse(raw.toString());
      if (message.type === "subscribe") {
        socket.send(JSON.stringify({ type: "ack", subscribed: message.channels }));
        if (message.channels.includes("stocks")) { sockets.add(socket); publish(); }
      } else if (message.type === "ping") socket.send(JSON.stringify({ type: "pong" }));
    });
    socket.onClose(() => sockets.delete(socket));
  });
  await page.route("**/*", async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    if (!request.url().startsWith(API)) {
      if (url.hostname === "127.0.0.1" && !url.pathname.startsWith("/api/")) return route.continue();
      return route.abort();
    }
    const path = url.pathname;
    const json = (body: any, status = 200) => route.fulfill({ status, json: body });
    if (request.method() === "OPTIONS") return route.fulfill({ status: 204 });
    if (path === "/api/auth/ws-ticket") return json({ ticket: "fixture", expiresAtMs: NOW + 60_000 });
    if (path === "/api/stocks/catalog") return json({ rows: securities, observedAtMs: NOW });
    if (["/api/stocks", "/api/stocks/peer/plans"].includes(path) && request.method() === "GET") return json(snapshot());
    if (path === "/api/trading/action-runs") return json({ status: "ready", data: actions, problems: [], source: "fixture", observedAtMs: NOW });
    if (path.startsWith("/api/trading/action-runs/")) return json(actions.find(action => action.id === decodeURIComponent(path.split("/").at(-1)!)));
    if (request.method() === "POST" && path.startsWith("/api/stocks/")) {
      const payload = request.postDataJSON();
      const body = path === "/api/stocks/batch" ? payload.request : payload;
      writes.push({ path, body, expectedRevision: payload.expectedRevision });
      if (path === "/api/stocks/batch") {
        const action: any = { id: `batch-${actions.length + 1}`, kind: "stock_batch_update", status: "accepted",
          target: "stocks-batch", actor: "fixture", message: "saved", startedAtMs: NOW, updatedAtMs: NOW,
          requestId: request.headers()["x-request-id"], idempotencyKey: request.headers()["idempotency-key"] };
        actions.push(action);
        if (batchReply) { await batchReply; batchReply = undefined; }
        if (payload.expectedRevision !== batch.revision) {
          action.status = "failed";
          return json({ error: { code: "STOCK_BATCH_CHANGED", message: "后台批量参数已变化，本次未修改", status: 409 } }, 409);
        }
        if (rejectBatch) {
          rejectBatch = false;
          action.status = "failed";
          return json({ error: { code: "STOCK_BATCH_REJECTED", message: "批量参数未通过验证", status: 400 } }, 400);
        }
        batch = { ...batch, revision: `fixture-saved-${actions.length}`, request: body, completedRounds: 1, metadataAtMs: NOW,
          rows: securities.filter((s) => body.assets.includes(s.asset)).map(row) };
        const receipt = snapshot();
        Object.assign(action, { status: "succeeded", result: receipt });
        if (loseBatchReply) { loseBatchReply = false; return route.abort(); }
        return json(receipt);
      }
      if (path === "/api/stocks/watch") {
        const outcome = watchOutcome;
        watchOutcome = "ok";
        if (outcome === "rejected") return json({ error: { code: "STOCK_WATCH_FAILED", message: "股票目录暂不可用", status: 503 } }, 503);
        if (outcome === "mismatch") return json({ ...snapshot(), security: securities[2] });
        security = securities.find((s) => s.asset === body.asset) ?? null;
        const receipt = snapshot();
        const waiting = watchReply;
        watchReply = undefined;
        if (waiting) await waiting;
        return json(receipt);
      }
      throw new Error(`Unexpected mutation: ${path}`);
    }
    return json({ error: { code: "FIXTURE_NOT_CONFIGURED", message: "isolated fixture", status: 404 } }, 404);
  });
  const updateBatch = (patch: any) => {
    batch = { ...batch, ...(Object.hasOwn(patch, "request") ? { revision: `fixture-other-${++revision}` } : {}), ...patch };
    publish();
  };
  const holdBatch = (reject = false) => {
    rejectBatch = reject;
    let release!: () => void;
    batchReply = new Promise<void>((resolve) => { release = resolve; });
    return release;
  };
  return { writes, errors, publish, updateBatch, holdBatch, actions,
    batchRows: (): any[] => structuredClone(batch.rows),
    holdWatch: () => {
      let release!: () => void;
      watchReply = new Promise<void>(resolve => { release = resolve; });
      return release;
    },
    watchOutcome: (outcome: "rejected" | "mismatch") => { watchOutcome = outcome; },
    selectStock: (asset: string) => { security = securities.find(s => s.asset === asset) ?? null; publish(); },
    loseReply: () => { loseBatchReply = true; } };
}

test("BP workbench aligns full-width monitoring, stock selection and detail at desktop and mobile", async ({ page }) => {
  const f = await setup(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/#stocks");
  await expect(page.getByRole("navigation", { name: "股票详情视图" }).getByRole("button", { name: "市场监控", exact: true })).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator(".stock-detail-workspace")).toBeHidden();
  const picker = page.getByRole("button", { name: "选择股票", exact: true });
  const catalog = page.getByRole("complementary", { name: "Backpack 股票目录" });
  const batch = page.getByRole("region", { name: "批量链上监控" });
  await expect(picker).toHaveAttribute("aria-expanded", "false");
  await expect(catalog).toBeHidden();
  await picker.click();
  await catalog.getByRole("searchbox", { name: "搜索股票", exact: true }).fill("AAPL");
  await catalog.getByRole("checkbox", { name: "监控 AAPL", exact: true }).check();
  await catalog.getByRole("searchbox", { name: "搜索股票", exact: true }).fill("");
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await picker.scrollIntoViewIfNeeded();
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
    await page.screenshot({ path: test.info().outputPath(`bp-picker-${width}.png`) });
  }
  await page.getByRole("button", { name: "完成选择", exact: true }).click();
  await expect(picker).toBeFocused();
  await expect(catalog).toBeHidden();
  await picker.click();
  await catalog.getByRole("searchbox", { name: "搜索股票", exact: true }).focus();
  await page.keyboard.press("Escape");
  await expect(picker).toBeFocused();
  await expect(catalog).toBeHidden();
  expect(f.writes).toHaveLength(0);
  await batch.getByRole("button", { name: "开始批量轮询", exact: true }).click();
  await expect(batch.locator(".stock-batch-state")).toHaveText("监控中 · 1 轮");
  const mu = batch.getByRole("row").filter({ hasText: "Micron Technology" });
  await expect(mu.locator("td").nth(1)).toHaveText("50");
  await expect(mu.locator("td").nth(2)).toHaveText("48");
  for (const width of [1440, 1024, 390, 320]) {
    await page.setViewportSize({ width, height: 900 });
    await page.evaluate(() => window.scrollTo({ top: 0, behavior: "instant" }));
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
    const main = await page.locator(".stock-main").boundingBox();
    expect(main!.width).toBeGreaterThan(width * 0.9);
    const overview = batch.getByLabel("股票监控摘要");
    await expect(overview).toContainText("运行范围3只股票");
    expect(await overview.locator("dl").evaluateAll(items => items.every(item =>
      item.scrollWidth <= item.clientWidth + 1))).toBe(true);
    const controls = batch.locator(".stock-batch-toolbar");
    expect(await controls.locator("input, select, button").evaluateAll(items => items.every(item => {
      const rect = item.getBoundingClientRect();
      return rect.width >= (item.tagName === "BUTTON" ? 44 : 60)
        && rect.left >= 0 && rect.right <= innerWidth;
    }))).toBe(true);
    expect(await batch.locator(".stock-batch-table-wrap").evaluate(el => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
    const filter = batch.getByRole("searchbox", { name: "搜索监控股票", exact: true });
    await expect(filter).toBeInViewport();
    expect(await filter.evaluate(el => {
      const r = el.getBoundingClientRect();
      return el === document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
    })).toBe(true);
    if (width > 800) await expect(batch.locator("thead")).toBeVisible();
    const action = mu.getByRole("button", { name: "查看", exact: true });
    await expect(action).toBeInViewport();
    expect(await action.evaluate(el => {
      const r = el.getBoundingClientRect();
      return el.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2));
    })).toBe(true);
    await page.screenshot({ path: test.info().outputPath(`bp-monitor-${width}.png`) });
  }
  await mu.getByRole("button", { name: "查看", exact: true }).click();
  await expect(page.locator("#stock-batch-content")).toBeHidden();
  const tabs = page.getByRole("navigation", { name: "股票详情视图" });
  const amount = page.getByLabel("链买预算 USDC", { exact: true });
  await amount.fill("12.75");
  const writesBeforeViewChange = f.writes.length;
  await page.getByRole("button", { name: "返回监控", exact: true }).click();
  f.publish();
  await expect(batch).toBeVisible();
  await expect(page.locator(".stock-detail-workspace")).toBeHidden();
  await tabs.getByRole("button", { name: "行情与提醒", exact: true }).click();
  await expect(amount).toHaveValue("12.75");
  expect(f.writes).toHaveLength(writesBeforeViewChange);
  for (const width of [1440, 390, 320]) {
    await page.setViewportSize({ width, height: 900 });
    await tabs.scrollIntoViewIfNeeded();
    await expect(page.getByRole("button", { name: "关闭详情", exact: true })).toBeInViewport();
    await expect(page.getByLabel("链买预算 USDC", { exact: true })).toBeInViewport();
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
    expect(await tabs.getByRole("button").evaluateAll(buttons => buttons.every(button => {
      const r = button.getBoundingClientRect();
      return r.left >= 0 && r.right <= innerWidth;
    }))).toBe(true);
    await expect(tabs.locator("button.active")).toHaveCount(1);
    expect(await tabs.locator("button.active").evaluate(el => getComputedStyle(el).minHeight)).toBe("38px");
    await page.screenshot({ path: test.info().outputPath(`bp-workspace-${width}.png`) });
  }
  await tabs.getByRole("button", { name: "合约资料", exact: true }).click();
  await expect(page.locator(".stock-contract-meta")).toContainText("595112103");
  await tabs.getByRole("button", { name: "跨所对比", exact: true }).click();
  await expect(page.getByRole("region", { name: "其他交易所股票对比" })).toBeVisible();
  await expect(page.locator(".stock-comparison")).toBeHidden();
  await tabs.getByRole("button", { name: "库存与成本", exact: true }).click();
  await expect(page.locator("#stock-inventory")).toBeVisible();
  for (const width of [1440, 390, 320]) {
    await page.setViewportSize({ width, height: 900 });
    await page.evaluate(() => window.scrollTo({ top: 0, behavior: "instant" }));
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
    if (width === 1440) {
      expect((await page.getByLabel("股票套利 Solana 钱包地址").boundingBox())!.width).toBeLessThanOrEqual(640);
    }
    await expect(page.locator(".stock-trading-route")).toBeHidden();
    await page.screenshot({ path: test.info().outputPath(`bp-inventory-${width}.png`) });
  }
  await tabs.getByRole("button", { name: "询价与执行", exact: true }).click();
  await expect(page.getByRole("region", { name: "Backpack 股票 询价" })).toBeVisible();
  for (const width of [1440, 390, 320]) {
    await page.setViewportSize({ width, height: 900 });
    await page.evaluate(() => window.scrollTo({ top: 0, behavior: "instant" }));
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
    if (width === 1440) {
      const direction = page.getByRole("region", { name: "Backpack 股票 询价" }).getByRole("combobox");
      expect((await direction.boundingBox())!.width).toBeLessThanOrEqual(280);
    }
    await page.screenshot({ path: test.info().outputPath(`bp-rfq-${width}.png`) });
  }
  await tabs.getByRole("button", { name: "执行记录", exact: true }).click();
  await expect(page.getByRole("region", { name: "股票执行计划" })).toBeVisible();
  await tabs.getByRole("button", { name: "行情与提醒", exact: true }).click();
  await page.getByRole("button", { name: "关闭详情", exact: true }).click();
  await expect(page.locator("#stock-batch-content")).toBeHidden();
  await tabs.getByRole("button", { name: "市场监控", exact: true }).click();
  await expect(page.locator("#stock-batch-content")).toBeVisible();
  expect(f.writes.map(w => w.path)).toEqual(["/api/stocks/batch", "/api/stocks/watch", "/api/stocks/watch"]);
  expect(f.errors).toEqual([]);
});

test("BP monitor separates bid and ask without changing units or stale quote guards", async ({ page }) => {
  const f = await setup(page);
  await page.goto("/#stocks");
  const panel = page.getByRole("region", { name: "批量链上监控" });
  await panel.getByRole("button", { name: "开始批量轮询", exact: true }).click();
  await expect(panel.locator(".stock-batch-state")).toHaveText("监控中 · 1 轮");
  const rows = f.batchRows().map((row, index) => ({ ...row, connected: true,
    security: { ...row.security, orderBooks: [{ symbol: `${row.security.asset}_USDC`, quote: "USDC",
      state: "Open", tickSize: "0.00001", minQuantity: "1", stepSize: "1" }] },
    books: [{ symbol: `${row.security.asset}_USDC`, bid: index ? "8.0001" : "1234.56789",
      ask: index ? "8.0002" : "1235.67891", bidQuantity: "1", askQuantity: "1", sourceAtMs: NOW,
      receivedAtMs: NOW, updateId: 1 }],
  }));
  f.updateBatch({ rows });
  const first = panel.locator("tbody tr").first();
  await expect(first.locator(".stock-book-bid")).toHaveText("1234.56789");
  await expect(first.locator(".stock-book-ask")).toHaveText("1235.67891");
  for (const width of [1440, 1024, 390, 320]) {
    await page.setViewportSize({ width, height: 900 });
    await first.scrollIntoViewIfNeeded();
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
    expect(await panel.locator(".stock-batch-table-wrap").evaluate(el => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
    const pair = first.locator(".stock-book-prices");
    const bid = await pair.locator("span").first().boundingBox();
    const ask = await pair.locator("span").last().boundingBox();
    expect(bid!.x + bid!.width).toBeLessThanOrEqual(ask!.x);
    expect(await pair.locator("span").evaluateAll(nodes => nodes.every(el => el.scrollWidth <= el.clientWidth + 1))).toBe(true);
    if (width > 800) {
      const labels = panel.locator("thead .stock-book-prices span");
      for (const index of [0, 1]) {
        const header = await labels.nth(index).boundingBox();
        const value = await pair.locator("span").nth(index).boundingBox();
        expect(Math.abs(header!.x + header!.width - value!.x - value!.width)).toBeLessThan(1);
      }
    }
    await page.screenshot({ path: test.info().outputPath(`bp-book-columns-${width}.png`) });
  }
  // Missing asks stay unknown, and the existing three-second book guard still applies.
  rows[0].books[0].ask = null;
  f.updateBatch({ rows });
  await expect(first.locator(".stock-book-ask")).toHaveText("—");
  await page.clock.setFixedTime(NOW + 3001);
  f.publish();
  await expect(first.locator(".stock-book-status")).toHaveText("暂无新鲜盘口");
  await expect(first.locator(".stock-book-prices")).toHaveCount(0);
  expect(f.writes.map(write => write.path)).toEqual(["/api/stocks/batch"]);
  expect(f.errors).toEqual([]);
});

test("BP batch search keeps monitoring scope and shows actual round cadence", async ({ page }) => {
  const f = await setup(page);
  await page.goto("/#stocks");
  const panel = page.getByRole("region", { name: "批量链上监控" });
  const timing = panel.getByLabel("批量轮询时效");
  const search = panel.getByRole("searchbox", { name: "搜索监控股票", exact: true });
  await panel.getByRole("button", { name: "开始批量轮询", exact: true }).click();
  // An older server without timing fields remains readable, but must not imply a zero duration.
  await expect(timing.locator("dd").first()).toHaveText("—");
  f.updateBatch({ running: true, roundStartedAtMs: NOW - 65_000 });
  await expect(timing).toContainText("本轮已耗时1m 05s");
  await expect(timing).toContainText("下轮更新本轮结束后");
  await search.fill("sandisk");
  await expect(panel.getByLabel("股票监控摘要")).toContainText("运行范围2只股票");
  await expect(panel.locator("tbody tr")).toHaveCount(1);
  await expect(panel.locator("tbody tr")).toContainText("SNDK");
  await expect(panel.locator(".stock-batch-result-count")).toHaveText("1 / 2 只");
  await expect(panel.getByLabel("双向新鲜报价")).toHaveText("双向新鲜 2/2");
  await expect(panel).toContainText("已选 2 / 32");
  f.updateBatch({ running: false, roundStartedAtMs: null, lastRoundElapsedMs: 147_800, nextAtMs: NOW + 15_000 });
  await expect(timing).toContainText("上轮耗时2m 27s");
  await expect(timing).toContainText("下轮更新15s 后");
  await expect(search).toBeFocused();
  await expect(search).toHaveValue("sandisk");
  for (const width of [1440, 390, 320]) {
    await page.setViewportSize({ width, height: 900 });
    await search.scrollIntoViewIfNeeded();
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
    expect(await timing.evaluate(el => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
    await page.screenshot({ path: test.info().outputPath(`bp-monitor-cadence-${width}.png`) });
  }
  await search.fill("missing");
  await expect(panel).toContainText("没有匹配的监控股票");
  await expect(panel.getByLabel("双向新鲜报价")).toHaveText("双向新鲜 2/2");
  await search.fill("");
  await expect(panel.locator("tbody tr")).toHaveCount(2);
  await page.clock.setFixedTime(NOW + 16_000);
  f.publish();
  await expect(timing).toContainText("下轮更新等待调度");
  await expect(panel.getByLabel("双向新鲜报价")).toHaveText("双向新鲜 0/2");
  await expect(panel.locator("tbody tr").first().locator("td").nth(1)).toHaveText("—");
  expect(f.writes).toHaveLength(1);
  await panel.getByRole("button", { name: "暂停", exact: true }).click();
  await expect(timing).toContainText("下轮更新未运行");
  expect(f.writes.at(-1).body.assets).toEqual(["MU.US", "SNDK.US"]);
  expect(f.errors).toEqual([]);
});

test("BP batch detail inherits saved quote parameters without applying unsaved drafts or auto quoting", async ({ page }) => {
  const f = await setup(page);
  await page.goto("/#stocks");
  const batch = page.getByRole("region", { name: "批量链上监控" });
  await batch.getByLabel("批量询价金额").fill("25.5");
  await batch.getByLabel("批量报价源").selectOption("keyed");
  await batch.getByRole("button", { name: "开始批量轮询", exact: true }).click();
  await expect(batch.locator(".stock-batch-state")).toHaveText("监控中 · 1 轮");
  await batch.getByLabel("批量询价金额").fill("not-saved");
  await batch.getByLabel("批量报价源").selectOption("public");
  const mu = batch.getByRole("row").filter({ hasText: "Micron Technology" });
  await mu.getByRole("button", { name: "查看", exact: true }).click();
  const controls = page.getByRole("region", { name: "当前股票询价参数" });
  const amount = controls.getByLabel("链买预算 USDC", { exact: true });
  const source = controls.getByLabel("Jupiter 接入", { exact: true });
  await expect(amount).toHaveValue("25.5");
  await expect(source).toHaveValue("keyed");
  await expect(controls).toContainText("尚无链上报价");
  await amount.fill("12.75");
  await source.selectOption("public");
  await page.getByRole("navigation", { name: "股票详情视图" }).getByRole("button", { name: "市场监控", exact: true }).click();
  await expect(batch.getByLabel("批量询价金额")).toHaveValue("not-saved");
  await mu.getByRole("button", { name: "查看", exact: true }).click();
  await expect(amount).toHaveValue("12.75");
  await expect(source).toHaveValue("public");
  f.watchOutcome("rejected");
  await page.getByRole("navigation", { name: "股票详情视图" }).getByRole("button", { name: "市场监控", exact: true }).click();
  const sndk = batch.getByRole("row").filter({ hasText: "Sandisk" });
  await sndk.getByRole("button", { name: "查看", exact: true }).click();
  await expect(page.locator(".stock-problem").filter({ hasText: "股票目录暂不可用" })).toBeVisible();
  await expect(amount).toHaveValue("12.75");
  await expect(source).toHaveValue("public");
  const release = f.holdWatch();
  await page.getByRole("navigation", { name: "股票详情视图" }).getByRole("button", { name: "市场监控", exact: true }).click();
  await sndk.getByRole("button", { name: "查看", exact: true }).click();
  await expect(amount).toBeDisabled();
  await expect(source).toBeDisabled();
  await expect.poll(() => f.writes.filter(w => w.path === "/api/stocks/watch").length).toBe(4);
  f.publish();
  await expect(page.locator(".stock-summary").first()).toContainText("SNDK");
  release();
  await expect(amount).toBeEnabled();
  await expect(amount).toHaveValue("25.5");
  await expect(source).toHaveValue("keyed");
  await expect(page.locator(".stock-problem").filter({ hasText: "保留当前详情与询价参数" })).toHaveCount(0);
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await controls.scrollIntoViewIfNeeded();
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
    await page.screenshot({ path: test.info().outputPath(`bp-detail-parameters-${width}.png`) });
  }
  expect(f.writes.filter(w => w.path === "/api/stocks/batch")).toHaveLength(1);
  expect(f.writes.filter(w => w.path === "/api/stocks/watch")).toHaveLength(4);
  expect(f.writes.every(w => ["/api/stocks/batch", "/api/stocks/watch"].includes(w.path))).toBe(true);
  expect(f.errors).toEqual([]);
});

test("BP stock selection rejects mismatched and late replies without replacing the current draft", async ({ page }) => {
  const f = await setup(page);
  await page.goto("/#stocks");
  const batch = page.getByRole("region", { name: "批量链上监控" });
  await batch.getByLabel("批量询价金额").fill("80");
  await batch.getByLabel("批量报价源").selectOption("keyed");
  await batch.getByRole("button", { name: "开始批量轮询", exact: true }).click();
  await expect(batch.locator(".stock-batch-state")).toHaveText("监控中 · 1 轮");
  const release = f.holdWatch();
  const mu = batch.getByRole("row").filter({ hasText: "Micron Technology" });
  await mu.getByRole("button", { name: "查看", exact: true }).click();
  await expect.poll(() => f.writes.filter(w => w.path === "/api/stocks/watch").length).toBe(1);
  f.selectStock("SNDK.US");
  const amount = page.getByLabel("链买预算 USDC", { exact: true });
  await expect(amount).toBeDisabled();
  release();
  await expect(page.locator(".stock-problem").filter({ hasText: "保留当前详情与询价参数" })).toBeVisible();
  await expect(amount).toHaveValue("10");
  await expect(page.getByLabel("Jupiter 接入", { exact: true })).toHaveValue("public");
  await expect(page.locator(".stock-summary").first()).toContainText("SNDK");
  await amount.fill("14.25");
  await page.getByRole("navigation", { name: "股票详情视图" }).getByRole("button", { name: "市场监控", exact: true }).click();
  f.watchOutcome("mismatch");
  await mu.getByRole("button", { name: "查看", exact: true }).click();
  await expect(page.locator(".stock-problem").filter({ hasText: "保留当前详情与询价参数" })).toBeVisible();
  await expect(amount).toHaveValue("14.25");
  await expect(page.locator(".stock-summary").first()).toContainText("SNDK");
  const releaseLatest = f.holdWatch();
  await page.getByRole("navigation", { name: "股票详情视图" }).getByRole("button", { name: "市场监控", exact: true }).click();
  await mu.getByRole("button", { name: "查看", exact: true }).click();
  await expect.poll(() => f.writes.filter(w => w.path === "/api/stocks/watch").length).toBe(3);
  f.updateBatch({ request: { ...f.writes[0].body, budgetUsdc: "90", keyed: false } });
  await expect(page.locator(".stock-summary").first()).toContainText("MU");
  releaseLatest();
  await expect(amount).toHaveValue("90");
  await expect(page.getByLabel("Jupiter 接入", { exact: true })).toHaveValue("public");
  await expect(page.locator(".stock-summary").first()).toContainText("MU");
  expect(f.writes.every(w => ["/api/stocks/batch", "/api/stocks/watch"].includes(w.path))).toBe(true);
  expect(f.errors).toEqual([]);
});

test("BP batch reload retains drafts and verifies lost responses without duplicate saves", async ({ page }) => {
  const f = await setup(page);
  await page.goto("/#stocks");
  const panel = page.getByRole("region", { name: "批量链上监控" });
  const budget = panel.getByLabel("批量询价金额");
  const recovery = panel.getByRole("alert", { name: "设置操作待核对" });
  await expect(panel).toContainText("已选 2 / 32");
  await budget.fill("25.5");
  await panel.getByLabel("批量更新间隔").selectOption("30");
  await panel.getByLabel("批量报价源").selectOption("keyed");
  await page.getByRole("button", { name: "选择股票", exact: true }).click();
  await page.getByRole("checkbox", { name: "监控 AAPL", exact: true }).check();
  await page.reload();
  await expect(budget).toHaveValue("25.5");
  await expect(panel.getByLabel("批量更新间隔")).toHaveValue("30");
  await expect(panel.getByLabel("批量报价源")).toHaveValue("keyed");
  await page.getByRole("button", { name: "选择股票", exact: true }).click();
  await expect(page.getByRole("checkbox", { name: "监控 AAPL", exact: true })).toBeChecked();
  expect(f.writes).toEqual([]);
  f.loseReply();
  await panel.getByRole("button", { name: "开始批量轮询", exact: true }).click();
  await expect(recovery).toContainText("保存股票批量监控结果待核对");
  await page.reload();
  await expect(recovery).toBeVisible();
  await expect(budget).toBeDisabled();
  f.actions[0].target = "wrong-target";
  await recovery.getByRole("button", { name: "核对上次操作", exact: true }).click();
  await expect(recovery).toContainText("账本未找到原操作");
  await expect(budget).toBeDisabled();
  f.actions[0].target = "stocks-batch";
  f.updateBatch({ request: { ...f.writes[0].body, enabled: false, budgetUsdc: "88.75", intervalSecs: 60 } });
  await recovery.getByRole("button", { name: "核对上次操作", exact: true }).click();
  await expect(recovery).toBeHidden();
  await expect(panel.locator(".stock-batch-state")).toHaveText("已暂停");
  await expect(budget).toHaveValue("25.5");
  await expect(budget).toBeEnabled();
  const conflict = panel.getByRole("alert", { name: "批量参数冲突" });
  await expect(conflict).toContainText("88.75 USDC");
  await expect(panel.getByRole("button", { name: "开始批量轮询", exact: true })).toBeDisabled();
  expect(f.writes).toHaveLength(1);
  const records = await page.evaluate(() => Object.entries(sessionStorage)
    .filter(([key]) => key.startsWith("crossline.settings.pending.v1:stocks-batch:")));
  expect(records).toHaveLength(1);
  expect(records[0][0]).toMatch(/\.draft$/);
  expect(records[0][1]).not.toContain("isolated-fixture-token");
  await page.reload();
  await expect(budget).toHaveValue("25.5");
  await expect(conflict).toBeVisible();
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await conflict.scrollIntoViewIfNeeded();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    await page.screenshot({ path: test.info().outputPath(`bp-conflict-${width}.png`) });
  }
  await conflict.getByRole("button", { name: "保留草稿待应用", exact: true }).click();
  await expect(conflict).toBeHidden();
  await expect(panel).toContainText("有未应用的更改 · 后台监控仍已暂停");
  expect(f.writes).toHaveLength(1);
  const release = f.holdBatch(true);
  await panel.getByRole("button", { name: "开始批量轮询", exact: true }).click(); release();
  await expect(panel.locator(".stock-batch-save-problem")).toContainText("批量参数未通过验证");
  await expect(recovery).toBeHidden();
  await expect(budget).toHaveValue("25.5");
  const key = records[0][0];
  await page.evaluate(key => sessionStorage.setItem(key, "bad-draft"), key);
  await page.reload();
  await expect(panel).toContainText("批量草稿损坏");
  await expect(budget).toBeDisabled();
  expect(await page.evaluate(key => sessionStorage.getItem(key), key)).toBe("bad-draft");
  await panel.getByRole("button", { name: "清除本地草稿", exact: true }).click();
  await expect(budget).toHaveValue("88.75");
  await expect(budget).toBeEnabled();
  // A new server instance publishes a fresh empty snapshot: no automatic resume.
  f.updateBatch({ request: null, rows: [], completedRounds: 0 });
  await expect(panel.locator(".stock-batch-state")).toHaveText("未开始");
  await expect(budget).toHaveValue("88.75");
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await panel.scrollIntoViewIfNeeded();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    await page.screenshot({ path: test.info().outputPath(`bp-reload-${width}.png`), fullPage: true });
  }
  expect(f.writes).toHaveLength(2); expect(f.errors).toEqual([]);
});

test("batch stock draft, pause, expiry and detail tabs reflect saved backend state", async ({ page }) => {
  const f = await setup(page);
  await page.goto("/#stocks");
  const panel = page.getByRole("region", { name: "批量链上监控" });
  await expect(panel).toBeVisible();
  await page.getByRole("button", { name: "选择股票", exact: true }).click();
  const catalog = page.getByRole("complementary", { name: "Backpack 股票目录" });
  await expect(catalog).toContainText("发行资料已收录 2 / 3");
  await catalog.getByRole("checkbox", { name: "只看已收录发行资料", exact: true }).check();
  await expect(catalog.getByRole("checkbox", { name: "监控 AAPL", exact: true })).toHaveCount(0);
  await catalog.getByRole("checkbox", { name: "只看已收录发行资料", exact: true }).uncheck();
  await page.getByRole("checkbox", { name: "监控 AAPL", exact: true }).check();
  const release = f.holdBatch();
  await panel.getByRole("button", { name: "开始批量轮询", exact: true }).click();
  await expect(panel.getByLabel("批量询价金额")).toBeDisabled();
  await expect(panel.getByLabel("批量报价源")).toBeDisabled();
  await expect(panel.getByLabel("批量更新间隔")).toBeDisabled();
  release();
  await expect(panel.locator(".stock-batch-state")).toHaveText("监控中 · 1 轮");
  await expect(panel.getByLabel("批量询价金额")).toBeEnabled();
  expect([...f.writes[0].body.assets].sort()).toEqual(["AAPL.US", "MU.US", "SNDK.US"]);
  const apple = panel.getByRole("row").filter({ hasText: "Apple Inc." });
  await expect(apple.locator("td").nth(1)).toHaveText("50");
  await expect(apple.locator("td").nth(2)).toHaveText("48");
  await expect(apple).toContainText("关闭 / 未知");
  await expect(apple).toContainText("仅官方合约映射");
  await expect(panel.getByRole("row").filter({ hasText: "Micron Technology" })).toContainText("发行映射已匹配");
  f.updateBatch({ running: true });
  await expect(panel.locator(".stock-batch-state")).toHaveText("本轮更新 3/3");
  f.updateBatch({ running: false, problem: "报价服务 暂时不可用", nextAtMs: NOW + 20_000 });
  await expect(panel.locator(".stock-batch-state")).toHaveText("等待重试 · 20s");
  f.updateBatch({ problem: null });
  await panel.getByLabel("批量询价金额").fill("25.5");
  f.publish();
  await expect(panel.getByLabel("批量询价金额")).toHaveValue("25.5");
  await expect(panel).toContainText("有未应用的更改");
  await page.getByRole("checkbox", { name: "监控 AAPL", exact: true }).uncheck();
  await expect(apple).toBeVisible();
  await panel.getByRole("button", { name: "应用参数", exact: true }).click();
  await expect(apple).toHaveCount(0);
  expect(f.writes.at(-1).body.budgetUsdc).toBe("25.5");
  await panel.getByLabel("批量询价金额").fill("invalid");
  await panel.getByRole("button", { name: "暂停", exact: true }).click();
  await expect(panel.locator(".stock-batch-state")).toHaveText("已暂停");
  expect(f.writes.at(-1).body).toMatchObject({ enabled: false, budgetUsdc: "25.5" });
  await expect(panel.getByLabel("批量询价金额")).toHaveValue("invalid");
  await panel.getByLabel("批量询价金额").fill("25.5");
  await panel.getByRole("button", { name: "开始批量轮询", exact: true }).click();
  await page.clock.setFixedTime(NOW + 11_000);
  f.publish();
  await expect(panel.getByRole("row").filter({ hasText: "Micron Technology" })).toContainText("报价已过期");
  await expect(panel.getByRole("row").filter({ hasText: "Micron Technology" }).locator("td").nth(1)).toHaveText("—");
  await panel.getByRole("button", { name: "查看", exact: true }).first().click();
  // Stored issuer documents never substitute for the selected market's live mapping.
  await expect(page.locator(".stock-identity-problem")).toContainText("官方目录缺少该股票的 Solana 映射");
  await expect(page.getByRole("button", { name: "更新询价", exact: true })).toBeDisabled();
  await expect(page.locator("#stock-batch-content")).toBeHidden();
  await expect(panel).toBeHidden();
  const tabs = page.getByRole("navigation", { name: "股票详情视图" });
  await page.getByLabel("链买预算 USDC", { exact: true }).fill("12.5");
  await tabs.getByRole("button", { name: "询价与执行", exact: true }).click();
  await expect(page.getByLabel("链买预算 USDC", { exact: true })).toBeHidden();
  await expect(page.getByRole("region", { name: "Backpack 股票 询价" })).toBeVisible();
  await expect(page.locator("#stock-inventory")).toBeHidden();
  await tabs.getByRole("button", { name: "库存与成本", exact: true }).click();
  await expect(page.locator("#stock-inventory")).toBeVisible();
  await expect(page.getByLabel("链买预算 USDC", { exact: true })).toBeVisible();
  await expect(page.getByLabel("链买预算 USDC", { exact: true })).toHaveValue("12.5");
  const controls = page.getByRole("region", { name: "当前股票询价参数" });
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await controls.scrollIntoViewIfNeeded();
    expect(await controls.evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
    await page.screenshot({ path: test.info().outputPath(`stock-inventory-${width}.png`) });
  }
  await page.setViewportSize({ width: 1440, height: 900 });
  await expect(page.getByRole("region", { name: "Backpack 股票 询价" })).toBeHidden();
  await tabs.getByRole("button", { name: "执行记录", exact: true }).click();
  await expect(page.getByRole("region", { name: "股票执行计划" })).toBeVisible();
  await tabs.getByRole("button", { name: "行情与提醒", exact: true }).click();
  await tabs.getByRole("button", { name: "市场监控", exact: true }).click();
  // Restore the fixture's fresh window for the narrow-screen price visibility check.
  await page.clock.setFixedTime(NOW + 1000);
  f.publish();
  const firstRow = panel.getByRole("row").filter({ hasText: "Micron Technology" });
  await expect(firstRow.locator("td").nth(1)).toHaveText("50");
  await expect(firstRow.locator("td").nth(2)).toHaveText("48");
  for (const width of [1440, 1024, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await panel.scrollIntoViewIfNeeded();
    expect(await panel.evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
    const action = panel.getByRole("button", { name: "查看", exact: true }).first();
    const bounds = await action.boundingBox();
    expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(width);
    const tableFits = await panel.locator(".stock-batch-table-wrap").evaluate(el => el.scrollWidth <= el.clientWidth + 1);
    expect(tableFits).toBe(true);
    for (const cell of await firstRow.locator("td[data-label]").all()) {
      const rect = await cell.boundingBox();
      expect(rect!.x).toBeGreaterThanOrEqual(0);
      expect(rect!.x + rect!.width).toBeLessThanOrEqual(width);
    }
    await page.screenshot({ path: test.info().outputPath(`stock-batch-${width}.png`) });
  }
  expect(f.errors).toEqual([]);
  await tabs.getByRole("button", { name: "行情与提醒", exact: true }).click();
  await page.getByRole("button", { name: "关闭详情", exact: true }).click();
  await tabs.getByRole("button", { name: "市场监控", exact: true }).click();
  await expect(page.locator("#stock-batch-content")).toBeVisible();
  expect(f.writes.every((w) => ["/api/stocks/batch", "/api/stocks/watch"].includes(w.path))).toBe(true);
});

test("batch stock drafts and in-flight save survive module navigation without duplicate writes", async ({ page }) => {
  const f = await setup(page);
  await page.goto("/#stocks");
  const nav = page.getByRole("navigation", { name: "功能模块", exact: true });
  const panel = page.getByRole("region", { name: "批量链上监控" });
  const go = async (module: string) => {
    await nav.locator(`button[data-module="${module}"]`).click();
    await expect(page).toHaveURL(new RegExp(`#${module}$`));
  };
  const amount = panel.getByLabel("批量询价金额");
  await amount.fill("25.5");
  await panel.getByLabel("批量更新间隔").selectOption("30");
  await panel.getByLabel("批量报价源").selectOption("keyed");
  await page.getByRole("button", { name: "选择股票", exact: true }).click();
  await page.getByRole("checkbox", { name: "监控 AAPL", exact: true }).check();
  await go("settings"); await go("stocks");
  await expect(amount).toHaveValue("25.5");
  await expect(panel.getByLabel("批量更新间隔")).toHaveValue("30");
  await expect(panel.getByLabel("批量报价源")).toHaveValue("keyed");
  await page.getByRole("button", { name: "选择股票", exact: true }).click();
  await expect(page.getByRole("checkbox", { name: "监控 AAPL", exact: true })).toBeChecked();

  const release = f.holdBatch();
  await panel.getByRole("button", { name: "开始批量轮询", exact: true }).click();
  await expect.poll(() => f.writes.length).toBe(1);
  await go("settings"); await go("stocks");
  await expect(amount).toBeDisabled();
  await expect(panel.getByRole("button", { name: "保存中…", exact: true })).toBeDisabled();
  await page.getByRole("button", { name: "选择股票", exact: true }).click();
  await expect(page.getByRole("checkbox", { name: "监控 AAPL", exact: true })).toBeDisabled();
  await go("settings");
  const saved = page.waitForResponse(r => r.url().endsWith("/api/stocks/batch") && r.status() === 200);
  release();
  await saved;
  await go("stocks");
  await expect(panel.locator(".stock-batch-state")).toHaveText("监控中 · 1 轮");
  await expect(amount).toHaveValue("25.5");
  await expect(amount).toBeEnabled();
  expect(f.writes).toHaveLength(1);

  await amount.fill("42.25");
  const reject = f.holdBatch(true);
  await panel.getByRole("button", { name: "应用参数", exact: true }).click();
  await expect.poll(() => f.writes.length).toBe(2);
  await go("settings");
  const rejected = page.waitForResponse(r => r.url().endsWith("/api/stocks/batch") && r.status() === 400);
  reject(); await rejected;
  await go("stocks");
  await expect(amount).toHaveValue("42.25");
  await expect(amount).toBeEnabled();
  await expect(panel.locator(".stock-batch-save-problem")).toHaveText("批量参数未通过验证");
  await expect(panel).toContainText("有未应用的更改");
  await expect(panel.locator(".stock-batch-state")).toHaveText("监控中 · 1 轮");
  f.publish();
  await expect(amount).toHaveValue("42.25");
  await go("settings"); await go("stocks");
  await expect(amount).toHaveValue("42.25");
  await expect(panel.locator(".stock-batch-save-problem")).toHaveText("批量参数未通过验证");
  expect(f.writes).toHaveLength(2);
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await panel.scrollIntoViewIfNeeded();
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
    await page.screenshot({ path: test.info().outputPath(`bp-save-recovery-${width}.png`) });
  }
  await panel.getByRole("button", { name: "应用参数", exact: true }).click();
  await expect(panel.locator(".stock-batch-save-problem")).toHaveCount(0);
  await expect(panel.getByRole("button", { name: "应用参数", exact: true })).toBeDisabled();
  expect(f.writes).toHaveLength(3);
  expect(f.writes.at(-1).body).toMatchObject({ budgetUsdc: "42.25", keyed: true, intervalSecs: 30 });
  expect(f.writes.every(w => w.path === "/api/stocks/batch")).toBe(true);
  expect(f.errors).toEqual([]);
});
