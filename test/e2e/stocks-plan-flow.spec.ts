import { expect, test, type Page, type WebSocketRoute } from "@playwright/test";
import { readFileSync } from "node:fs";

const fixture = JSON.parse(readFileSync(new URL("../../shared-types/fixtures/stocks_plan_build.json", import.meta.url), "utf8"));

// Captured from stock_plan_build_one_pass with mock inventory, cost RPC and a real local journal.
// All application HTTP/WS are intercepted. No account, signer or webhook can be reached.
const API = "http://127.0.0.1:18997";
const NOW = fixture.observedAtMs;
const WALLET = fixture.plans[0].request.walletAddress;

async function setup(page: Page) {
  const market = structuredClone(fixture);
  market.plans = [];
  let revision = NOW;
  const writes: { path: string; body: any }[] = [];
  const errors: string[] = [];
  const sockets = new Set<WebSocketRoute>();
  const snapshot = () => ({ ...market, observedAtMs: ++revision });
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
      const msg = JSON.parse(raw.toString());
      if (msg.type === "subscribe") {
        socket.send(JSON.stringify({ type: "ack", subscribed: msg.channels }));
        if (msg.channels.includes("stocks")) { sockets.add(socket); publish(); }
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
    if (path === "/api/auth/ws-ticket") return json({ ticket: "fixture", expiresAtMs: NOW + 60_000 });
    if (path === "/api/stocks/catalog") return json({ rows: [market.security], observedAtMs: NOW });
    if (path === "/api/stocks/peer-markets") return json({ rows: [], observedAtMs: NOW });
    if (path === "/api/stocks" && req.method() === "GET") return json(snapshot());
    if (req.method() === "POST" && path.startsWith("/api/stocks/")) {
      const body = req.postDataJSON();
      writes.push({ path, body });
      if (path === "/api/stocks/plans/build") {
        expect(body).toMatchObject({ asset: "MU.US", direction: "buy", walletAddress: WALLET, inputRaw: "10000000", keyed: false });
        const plan = structuredClone(fixture.plans[0]);
        plan.request.requestId = body.requestId;
        plan.request.build = body;
        market.plans = [plan];
        return json(snapshot());
      }
      if (path === "/api/stocks/plans/cancel") {
        expect(body.planId).toBe(market.plans[0].planId);
        market.plans[0].phase = "cancelled";
        market.plans[0].revision += 1;
        return json(snapshot());
      }
      throw new Error(`Unexpected stock mutation: ${path}`);
    }
    return json({ error: { code: "FIXTURE_NOT_CONFIGURED", message: "isolated fixture", status: 404 } }, 404);
  });
  return { writes, errors, publish, damage: () => { market.planProblem = "fixture journal damaged"; publish(); } };
}

test("BP build and cancel update the actual WASM controls and survive reload", async ({ page }) => {
  const f = await setup(page);
  await page.goto("/#stocks");
  const inventory = page.getByRole("region", { name: "股票库存与成本预检" });
  const build = inventory.getByRole("button", { name: "构建并预留", exact: true }).first();
  const summary = page.locator(".stock-main > .stock-summary").first();
  await expect(build).toBeDisabled();
  await expect(inventory.locator(".stock-build-status").first()).toContainText("先填写 Solana 钱包地址");
  await inventory.getByLabel("股票套利 Solana 钱包地址").fill(WALLET);
  await page.getByLabel("链买预算 USDC", { exact: true }).fill("10.00");
  await expect(build).toBeEnabled();
  await expect(summary).toContainText("已预检 · 未预留");
  await expect(inventory.locator(".stock-chain-cost").first()).toHaveAttribute("data-current", "true");
  await page.getByLabel("链买预算 USDC", { exact: true }).fill("11");
  await expect(build).toBeDisabled();
  await expect(summary).toContainText("预检需更新");
  await expect(inventory.locator(".stock-chain-cost").first()).toHaveAttribute("data-current", "false");
  await expect(inventory.locator(".stock-build-status").first()).toContainText("先更新询价");
  await page.getByLabel("链买预算 USDC", { exact: true }).fill("10.00");
  await page.getByLabel("Jupiter 接入", { exact: true }).selectOption("keyed");
  await expect(build).toBeDisabled();
  await expect(summary).toContainText("预检需更新");
  await page.getByLabel("Jupiter 接入", { exact: true }).selectOption("public");
  await build.click();
  const history = page.getByRole("region", { name: "股票执行计划", exact: true });
  await expect(history.locator(".stock-plan-record")).toHaveCount(1);
  await expect(summary).toContainText("已预留 · 未下单");
  await expect(build).toBeDisabled();
  await expect(inventory.locator(".stock-build-status").first()).toContainText("已有股票计划占用资金");
  await page.reload();
  await expect(history.locator(".stock-plan-record")).toHaveCount(1);
  await expect(summary).toContainText("已预留 · 未下单");
  await history.getByRole("button", { name: "取消预留", exact: true }).click();
  await expect(history.locator(".stock-plan-phase")).toHaveText("已取消");
  await inventory.getByLabel("股票套利 Solana 钱包地址").fill(WALLET);
  await expect(build).toBeEnabled();
  for (const width of [1440, 390, 320]) {
    await page.setViewportSize({ width, height: 900 });
    await inventory.evaluate((el) => el.scrollIntoView({ block: "start" }));
    const wallet = inventory.getByLabel("股票套利 Solana 钱包地址");
    expect(await wallet.evaluate((el) => {
      const r = el.getBoundingClientRect();
      const header = document.querySelector(".mod-topbar")!.getBoundingClientRect();
      return r.top >= header.bottom && document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2) === el;
    })).toBe(true);
    expect(await inventory.evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
    expect(await page.locator(".module-tabs").evaluate((el) => {
      const nav = el.getBoundingClientRect();
      const status = document.querySelector(".topbar-status-frame")!.getBoundingClientRect();
      return Array.from(el.querySelectorAll("button")).every((button) => {
        const r = button.getBoundingClientRect();
        const hit = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
        return r.bottom <= status.top + 1 && r.left >= nav.left && r.right <= nav.right + 1 && !!hit && button.contains(hit);
      });
    })).toBe(true);
    await page.screenshot({ path: test.info().outputPath(`bp-build-preflight-${width}.png`) });
  }
  await page.reload();
  await expect(history.locator(".stock-plan-phase")).toHaveText("已取消");
  expect(f.writes.map((w) => w.path)).toEqual(["/api/stocks/plans/build", "/api/stocks/plans/cancel"]);
  expect(f.errors).toEqual([]);
});

test("BP expired evidence can be refreshed by build, while damaged journals block new reservations", async ({ page }) => {
  const f = await setup(page);
  await page.clock.setFixedTime(NOW + 11_000);
  await page.goto("/#stocks");
  const inventory = page.getByRole("region", { name: "股票库存与成本预检" });
  await inventory.getByLabel("股票套利 Solana 钱包地址").fill(WALLET);
  const build = inventory.getByRole("button", { name: "构建并预留", exact: true }).first();
  await expect(page.locator(".stock-readiness > strong")).toHaveText("预检需更新");
  await expect(build).toBeEnabled();
  f.damage();
  await expect(build).toBeDisabled();
  await expect(inventory.locator(".stock-build-status").first()).toContainText("资金记录存在问题");
  expect(f.writes).toEqual([]);
  expect(f.errors).toEqual([]);
});
