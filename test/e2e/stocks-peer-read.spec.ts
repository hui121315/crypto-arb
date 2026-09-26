import { expect, test, type Page, type WebSocketRoute } from "@playwright/test";
import { readFileSync } from "node:fs";

const base = JSON.parse(readFileSync(new URL("../../shared-types/fixtures/stocks_plan_build.json", import.meta.url), "utf8"));
const settlement = JSON.parse(readFileSync(new URL("../../shared-types/fixtures/stocks_peer_settlement.json", import.meta.url), "utf8"));
const basis = settlement.peerPlans[0].terms.basis;
const NOW = base.observedAtMs;
const API = "http://127.0.0.1:18997";
const WALLET = basis.wallet.owner;

async function openPeers(page: Page) {
  await page.getByRole("navigation", { name: "股票详情视图" }).getByRole("button", { name: "跨所对比", exact: true }).click();
}

async function flushRender(page: Page) {
  await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
}

async function switchAccount(page: Page, token: string) {
  await page.getByRole("button", { name: "切换到设置", exact: true }).click();
  await page.getByRole("tab", { name: "诊断", exact: true }).click();
  await page.getByRole("tab", { name: "连接", exact: true }).click();
  await page.locator(".settings-api-token-task input").fill(token);
  await page.getByRole("button", { name: "保存 Token", exact: true }).click();
  await page.getByRole("button", { name: "切换到股票套利", exact: true }).click();
  await openPeers(page);
}

async function setup(page: Page) {
  const market = structuredClone(base);
  market.plans = []; market.peerPlans = []; market.peerOrderChecks = [];
  market.peer = structuredClone(basis.peer);
  market.peer.instrument.checkedAtMs = NOW;
  for (const q of [market.peer.quote, market.peer.quoteConversion]) q.sourceAtMs = q.receivedAtMs = NOW;
  market.peerPreflight = null; market.peerFunding = null;
  let revision = NOW;
  let edit: { path: string; apply: (body: any) => void } | undefined;
  let hold: { path: string; promise: Promise<void> } | undefined;
  const reads: { path: string; query: string; token: string | undefined }[] = [];
  const writes: { path: string; body: any; token: string | undefined }[] = [];
  const errors: string[] = [];
  const sockets = new Set<WebSocketRoute>();
  const snapshot = () => structuredClone({ ...market, observedAtMs: ++revision });
  const publish = () => {
    const payload = snapshot();
    for (const socket of sockets) socket.send(JSON.stringify({ type: "message", channel: "stocks", payload }));
  };
  await page.clock.setFixedTime(NOW);
  await page.addInitScript(api => {
    localStorage.setItem("api_base", JSON.stringify(api));
    localStorage.setItem("api_auth_token", JSON.stringify("peer-source-a"));
  }, API);
  page.on("pageerror", e => errors.push(e.message));
  page.on("console", m => { if (m.type() === "error" && /panicked at|already borrowed|disposed/i.test(m.text())) errors.push(m.text()); });
  await page.routeWebSocket(/.*/, socket => {
    if (!socket.url().startsWith(API.replace("http:", "ws:"))) return socket.close();
    socket.onMessage(raw => {
      const msg = JSON.parse(raw.toString());
      if (msg.type === "subscribe") {
        socket.send(JSON.stringify({ type: "ack", subscribed: msg.channels }));
        if (msg.channels.includes("stocks")) { sockets.add(socket); publish(); }
      } else if (msg.type === "ping") socket.send(JSON.stringify({ type: "pong" }));
    });
    socket.onClose(() => sockets.delete(socket));
  });
  // Every account, venue and chain response is synthetic. Unexpected business calls fail closed.
  await page.route("**/*", async route => {
    const req = route.request(); const url = new URL(req.url()); const path = url.pathname;
    if (!req.url().startsWith(API)) {
      if (url.hostname === "127.0.0.1" && !path.startsWith("/api/")) return route.continue();
      return route.abort();
    }
    const json = async (value: any, status = 200) => {
      const body = structuredClone(value);
      if (edit?.path === path) { const fn = edit.apply; edit = undefined; fn(body); }
      if (hold?.path === path) {
        const waiting = hold.promise; hold = undefined; await waiting;
        if (body.observedAtMs) body.observedAtMs = ++revision;
      }
      await route.fulfill({ status, json: body });
    };
    if (req.method() === "OPTIONS") return route.fulfill({ status: 204 });
    if (path === "/api/auth/ws-ticket") return json({ ticket: "fixture", expiresAtMs: NOW + 60_000 });
    if (req.method() === "GET") {
      reads.push({ path, query: url.search, token: req.headers().authorization });
      if (path === "/api/stocks/catalog") return json({ rows: [market.security], observedAtMs: NOW });
      if (path === "/api/stocks/peer-markets") {
        const request = Object.fromEntries(url.searchParams);
        const rows = ["MUx/USD", "MUx/USDC"].map(nativeSymbol => ({ ...basis.peer.instrument,
          venue: request.venue, productType: request.product, nativeSymbol,
          quoteAsset: nativeSymbol.split("/")[1], checkedAtMs: NOW,
        })).filter(r => r.nativeSymbol.toLowerCase().includes(request.search.toLowerCase()));
        return json({ request, rows, matched: rows.length, registryCount: 2 });
      }
      if (["/api/stocks", "/api/stocks/peer/plans", "/api/stocks/funding/stablecoin-plans"].includes(path)) return json(snapshot());
    }
    if (req.method() === "POST" && path.startsWith("/api/stocks/")) {
      const body = req.postDataJSON(); writes.push({ path, body, token: req.headers().authorization });
      if (path === "/api/stocks/peer") {
        market.peer = body.selection ? { ...structuredClone(basis.peer), selection: body.selection } : null;
        market.peerPreflight = null; market.peerFunding = null; market.peerOrderChecks = [];
        return json(snapshot());
      }
      if (path === "/api/stocks/peer/preflight") return json({ ...snapshot(), peerPreflight: {
        asset: body.asset, selection: body.selection, checkedAtMs: NOW,
        account: { ...basis.account, nativeSymbol: body.selection.nativeSymbol, observedAtMs: NOW },
        wallet: body.walletAddress ? { ...basis.wallet, owner: body.walletAddress, checkedAtMs: NOW } : null,
        problems: [],
      } });
      if (path === "/api/stocks/peer/funding") return json({ ...snapshot(), peerFunding: {
        asset: body.asset, selection: body.selection, checkedAtMs: NOW,
        routes: ["MUx", "USDC"].flatMap(asset => ["deposit", "withdraw"].map(direction => ({
          asset, assetClass: asset === "USDC" ? "currency" : "tokenized_asset", direction, amountUnit: "base", methods: [], checkedAtMs: NOW,
          sourceUrl: "https://docs.kraken.com/", problem: null,
        }))),
      } });
      if (path === "/api/stocks/peer/order-check") return json({ ...snapshot(), peerOrderChecks: [{
        draft: { request: body, quantity: "0.02", limitPrice: "600", quoteAsset: "USD",
          preparedAtMs: NOW, sourceAtMs: NOW, metadataAtMs: NOW },
        completedAtMs: NOW, status: "passed", message: "isolated validation, no order sent",
      }] });
      throw new Error(`Unexpected business request ${path}`);
    }
    return json({ error: { code: "FIXTURE_NOT_CONFIGURED", message: "isolated fixture", status: 404 } }, 404);
  });
  return { reads, writes, errors,
    editNext: (path: string, apply: (body: any) => void) => { edit = { path, apply }; },
    holdNext: (path: string) => {
      let release!: () => void; hold = { path, promise: new Promise<void>(r => release = r) }; return release;
    },
    peer: (nativeSymbol: string) => {
      market.peer.selection.nativeSymbol = nativeSymbol;
      market.peerPreflight = null; market.peerFunding = null; market.peerOrderChecks = [];
      publish();
    },
    clearReports: () => { market.peerPreflight = null; market.peerFunding = null; market.peerOrderChecks = []; publish(); },
  };
}

test("BP peer catalog keeps search drafts separate and rejects obsolete market replies", async ({ page }) => {
  const f = await setup(page); await page.goto("/#stocks"); await openPeers(page);
  const panel = page.getByRole("region", { name: "其他交易所股票对比" });
  const search = panel.getByLabel("股票对比市场搜索", { exact: true });
  const choices = panel.getByLabel("选择对比市场", { exact: true });
  const selected = panel.locator(".stock-peer-current > header");
  await expect(choices).toBeEnabled();
  await expect(choices.locator("option[value='MUx/USDC']")).toHaveCount(1);
  await search.fill("OTHER");
  await expect(choices).toBeDisabled();
  await expect(choices.locator("option[value='MUx/USDC']")).toHaveCount(0);
  await expect(selected).toContainText("MUx/USD");
  await search.fill("MUx");
  const release = f.holdNext("/api/stocks/peer-markets");
  const sent = page.waitForRequest(r => r.url().includes("/peer-markets?") && r.url().includes("search=MUx"));
  await panel.getByRole("button", { name: "搜索", exact: true }).click(); await sent;
  await search.fill("MUx/USDC"); await search.press("Enter");
  await expect(choices).toBeEnabled();
  await expect(panel).toContainText("匹配 1 个市场");
  release();
  await choices.selectOption("MUx/USDC");
  await expect(selected).toContainText("MUx/USDC");
  await expect(choices).toHaveValue("MUx/USDC");
  f.editNext("/api/stocks/peer-markets", c => { c.rows[0].venue = "wrong-venue"; });
  await panel.getByRole("button", { name: "搜索", exact: true }).click();
  await expect(panel.getByRole("alert")).toContainText("市场目录回复与搜索条件不一致");
  await expect(choices).toBeDisabled();
  await expect(selected).toContainText("MUx/USDC");
  await search.fill("MU"); await search.press("Enter");
  await expect(choices).toBeEnabled();
  f.editNext("/api/stocks/peer", s => { s.peer.selection.nativeSymbol = "WRONG/USD"; });
  await choices.selectOption("MUx/USD");
  await expect(panel.getByRole("alert")).toContainText("市场选择回复与当前选择不一致");
  await expect(selected).toContainText("MUx/USDC");
  await expect(choices).toHaveValue("MUx/USDC");
  await panel.getByRole("button", { name: "移除对比", exact: true }).click();
  await expect(selected).toHaveCount(0);
  expect(f.writes.map(w => w.path)).toEqual(["/api/stocks/peer", "/api/stocks/peer", "/api/stocks/peer"]);
  expect(f.errors).toEqual([]);
});

test("BP peer preflight funding and validation stay with the requested account wallet and market", async ({ page }) => {
  const f = await setup(page); await page.goto("/#stocks"); await openPeers(page);
  const panel = page.getByRole("region", { name: "其他交易所股票对比" });
  const wallet = page.getByLabel("股票对比交易检查钱包", { exact: true });
  const inventory = panel.locator(".stock-peer-preflight");
  const preflight = "/api/stocks/peer/preflight";
  await wallet.fill(WALLET);
  await panel.getByRole("button", { name: "检查账户与费用", exact: true }).click();
  await expect(inventory).toContainText("费用与库存交易检查");
  await expect(inventory).not.toContainText("执行尚未接通");
  await expect(inventory.locator(".stock-peer-inventory").first()).toContainText("可用 10");
  f.editNext(preflight, s => { s.peerPreflight.wallet.owner = "wrong-wallet"; });
  await panel.getByRole("button", { name: "检查账户与费用", exact: true }).click();
  await expect(panel.getByRole("alert")).toContainText("账户交易检查回复与当前股票、交易对或钱包不一致");
  f.clearReports();
  const releaseWallet = f.holdNext(preflight);
  const sent = page.waitForRequest(`${API}${preflight}`);
  await panel.getByRole("button", { name: "检查账户与费用", exact: true }).click(); await sent;
  await wallet.fill("");
  const reply = page.waitForResponse(`${API}${preflight}`); releaseWallet(); await (await reply).finished();
  await flushRender(page);
  await expect(panel.getByRole("button", { name: "检查账户与费用", exact: true })).toBeEnabled();
  await expect(inventory).toContainText("尚未读取所选交易所账户");
  await expect(inventory.locator(".stock-peer-inventory")).toHaveCount(0);

  const funding = "/api/stocks/peer/funding";
  await panel.getByRole("button", { name: "检查充提", exact: true }).click();
  await expect(panel.locator(".stock-peer-funding-route")).toHaveCount(4);
  await expect(panel.locator(".stock-peer-funding")).toContainText("未返回可用方法");
  await expect(panel.locator(".stock-peer-funding")).toContainText("当前方法快照");
  await expect(panel.locator(".stock-peer-funding")).not.toContainText("合约匹配");
  f.editNext(funding, s => { s.peerFunding.selection.nativeSymbol = "WRONG/USD"; });
  await panel.getByRole("button", { name: "检查充提", exact: true }).click();
  await expect(panel.getByRole("alert")).toContainText("充提回复与当前股票或交易对不一致");
  f.clearReports();

  const order = "/api/stocks/peer/order-check";
  f.editNext(order, s => { s.peerOrderChecks[0].draft.request.direction = "sell"; });
  await panel.getByRole("button", { name: "验证股票卖单", exact: true }).click();
  await expect(panel.getByRole("alert")).toContainText("订单验证回复与当前方向或市场不一致");
  await expect(panel.locator(".stock-peer-order-result")).toHaveCount(0);
  await panel.getByRole("button", { name: "验证股票卖单", exact: true }).click();
  await expect(panel.locator(".stock-peer-order-result")).toContainText("参数验证通过");
  f.clearReports();

  const releaseMarket = f.holdNext(preflight);
  const marketSent = page.waitForRequest(`${API}${preflight}`);
  await panel.getByRole("button", { name: "检查账户与费用", exact: true }).click(); await marketSent;
  f.peer("MUx/USDC"); await expect(panel.locator(".stock-peer-current > header")).toContainText("MUx/USDC");
  f.peer("MUx/USD"); await expect(panel.locator(".stock-peer-current > header")).toContainText("MUx/USD");
  const marketReply = page.waitForResponse(`${API}${preflight}`); releaseMarket(); await (await marketReply).finished();
  await flushRender(page);
  await expect(inventory).toContainText("尚未读取所选交易所账户");

  const releaseSource = f.holdNext(preflight);
  const sourceSent = page.waitForRequest(`${API}${preflight}`);
  await panel.getByRole("button", { name: "检查账户与费用", exact: true }).click();
  expect((await sourceSent).headers().authorization).toBe("Bearer peer-source-a");
  await switchAccount(page, "peer-source-b");
  await expect(panel.getByRole("button", { name: "检查账户与费用", exact: true })).toBeEnabled();
  await expect.poll(() => f.reads.some(r => r.path === "/api/stocks/peer-markets" && r.token === "Bearer peer-source-b")).toBe(true);
  const sourceReply = page.waitForResponse(`${API}${preflight}`); releaseSource(); await (await sourceReply).finished();
  await flushRender(page);
  await expect(inventory).toContainText("尚未读取所选交易所账户");
  await panel.getByRole("button", { name: "检查账户与费用", exact: true }).click();
  await expect(inventory).toContainText("费用与库存交易检查");
  expect(f.writes.at(-1)?.token).toBe("Bearer peer-source-b");
  expect(f.writes.every(w => [preflight, funding, order].includes(w.path))).toBe(true);
  expect(f.errors).toEqual([]);
});
