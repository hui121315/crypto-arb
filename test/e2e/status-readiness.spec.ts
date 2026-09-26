import { expect, test, type Page } from "@playwright/test";
import { API, NOW, setup } from "./fixtures/opportunity-workbench";

function row(operation: string, status = "ok", configured: boolean | null = true, venue = "binance") {
  return { venue, operation, status, configured, supported: true, source: "fixture",
    message: `${venue} ${operation} ${status}`, observedAtMs: NOW, rows: status === "ok" ? 1 : 0 };
}
function connectedRows() {
  return [row("ws_ticker_snapshot"), row("private_read"), row("private_ws_order_stream")];
}
async function fixture(page: Page) {
  const base = await setup(page);
  await page.clock.install({ time: NOW });
  const system = await (await page.request.get(API + "/api/system/health")).json();
  const trading = await (await page.request.get(API + "/api/trading/status")).json();
  let rows: any[] = [], version = 1, frozenVersion: number | undefined, fail = false;
  system.data.risk = "ok";
  await page.route(API + "/api/system/venue-operation-health", route => route.fulfill(fail
    ? { status: 503, json: { error: { code: "HEALTH_READ_UNAVAILABLE", message: "fixture health read failed" } } }
    : { json: { rows, rowCount: rows.length, attentionCount: rows.filter(r => r.status !== "ok").length, generatedAtMs: NOW + (frozenVersion ?? version) } }));
  await page.route(API + "/api/system/health", route => route.fulfill({ json: {
    ...system, data: { ...system.data, updatedAtMs: NOW + version },
  } }));
  await page.route(API + "/api/trading/status", route => route.fulfill({ json: trading }));
  const emit = () => {
    for (const socket of base.channelSockets.get("system") ?? []) socket.send(JSON.stringify({ type: "message", channel: "system",
      payload: { ...system.data, updatedAtMs: NOW + version } }));
  };
  return { ...base,
    rows: (next: any[]) => { rows = next; },
    environment: (value: string) => { trading.environment = value; },
    risk: (value: string) => { system.data.risk = value; },
    fail: (value: boolean) => { fail = value; },
    freezeHealth: (value = true) => { frozenVersion = value ? version : undefined; },
    refresh: async () => {
      version++; emit();
      await page.clock.runFor(5_100);
      base.tick(5_100);
      await expect.poll(async () => (await page.getByTestId("status-api-runtime").count())).toBe(1);
    },
  };
}

test("summary and details distinguish missing samples, optional credentials, disabled markets and faults", async ({ page }, info) => {
  const f = await fixture(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/#futures");
  const summary = page.locator(".status-summary");
  const evidence = page.getByRole("group", { name: "接口与后台状态", exact: true });
  const market = page.getByTestId("status-market-data");
  const api = page.getByTestId("status-api-runtime");
  const ws = page.getByTestId("status-private-ws");
  await expect(summary).toContainText("运行状态待确认");
  await summary.click();
  for (const slot of [market, api, ws]) {
    await expect(slot).toContainText("无数据依据");
    await expect(slot).toHaveAttribute("data-state", "unknown");
    await expect(slot.locator(".slot-dot")).toHaveClass(/neutral/);
  }
  await expect(evidence).toContainText("后台快照未提供这类接口的运行样本");

  const unconfigured = [row("ws_ticker_snapshot"), row("private_read", "blocked", false), row("private_ws_order_stream", "unknown", false)];
  f.rows(unconfigured); f.environment("paper"); await f.refresh();
  await expect(summary).toHaveAttribute("data-state", "healthy");
  await expect(api).toContainText("模拟无需"); await expect(ws).toContainText("模拟无需");
  await expect(api.locator(".slot-dot")).toHaveClass(/neutral/);
  await expect(evidence).toHaveCount(0);
  f.environment("live"); await f.refresh();
  await expect(summary).toHaveAttribute("data-state", "unknown");
  await expect(api).toContainText("需配置凭证"); await expect(evidence).toContainText("设置的 API 凭证");

  f.rows([row("ws_ticker_snapshot"), row("private_read", "ok", null), row("private_ws_order_stream", "ok", null)]);
  await f.refresh();
  await expect(api).toContainText("配置待确认"); await expect(ws).toContainText("配置待确认");
  await expect(evidence).toContainText("配置状态尚未确认");
  f.rows([row("ws_ticker_snapshot"), row("private_read", "unknown"), row("private_ws_order_stream", "unknown")]);
  await f.refresh();
  await expect(api).toContainText("0可用/1配置");
  await expect(api).toHaveAttribute("data-state", "unknown");
  await expect(evidence).toContainText("private_ws_order_stream unknown");

  f.rows([...connectedRows(), row("ws_ticker_snapshot", "warn", true, "bitget")]);
  await f.refresh();
  await expect(summary).toHaveAttribute("data-state", "warning");
  await expect(market).toContainText("1/2");
  await expect(market.locator(".slot-dot")).toHaveClass(/warning/);
  await expect(evidence).toContainText("bitget · ws_ticker_snapshot");
  f.rows([...connectedRows(), row("ws_ticker_snapshot", "blocked", false, "bitget")]);
  await f.refresh();
  await expect(summary).toHaveAttribute("data-state", "healthy");
  await expect(market).toContainText("1/1");
  f.rows([row("ws_ticker_snapshot", "blocked", false), row("private_read"), row("private_ws_order_stream")]);
  await f.refresh();
  await expect(summary).toHaveAttribute("data-state", "unknown");
  await expect(market).toContainText("未启用");
  await expect(evidence).toContainText("不代表交易所连接失败");

  f.rows([row("ws_ticker_snapshot"), row("private_read", "blocked"), row("private_ws_order_stream")]);
  await f.refresh();
  await expect(summary).toContainText("运行状态异常");
  await expect(api).toHaveAttribute("data-state", "degraded");
  await expect(evidence).toContainText("交易接口 · 受限");
  await page.screenshot({ path: info.outputPath("readiness-desktop.png") });
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(evidence).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
  const rect = await evidence.boundingBox();
  expect(rect!.x).toBeGreaterThanOrEqual(0); expect(rect!.x + rect!.width).toBeLessThanOrEqual(391);
  await page.screenshot({ path: info.outputPath("readiness-mobile.png") });
  f.rows(connectedRows()); await f.refresh();
  await expect(summary).toHaveAttribute("data-state", "healthy");
  await expect(evidence).toHaveCount(0);
  expect(f.writes).toEqual([]); expect(f.errors).toEqual([]);
});

test("task and authenticated transport faults, WS reconnect and risk priority share the same summary", async ({ page }) => {
  const f = await fixture(page);
  f.rows(connectedRows()); f.environment("live");
  await page.goto("/#futures");
  const summary = page.locator(".status-summary");
  const evidence = page.getByRole("group", { name: "接口与后台状态", exact: true });
  await expect(summary).toHaveAttribute("data-state", "healthy");
  await summary.click();
  const transport = { ...row("http_rest:GET /fixture", "warn"), evidence: { method: "GET", path: "/fixture",
    authKind: "private_read", docUrls: [], useCases: [], dataKinds: [], rateScopes: [], weight: 1 } };
  f.rows([...connectedRows(), transport]); await f.refresh();
  await expect(summary).toHaveAttribute("data-state", "warning");
  await expect(page.getByTestId("status-api-runtime")).toHaveText("TradingAPI1可用/1配置");
  await expect(evidence).toContainText("http_rest:GET /fixture");
  f.rows([...connectedRows(), { ...transport, configured: false }, { ...transport, evidence: { ...transport.evidence, authKind: "public" } }]);
  await f.refresh(); await expect(summary).toHaveAttribute("data-state", "healthy");

  const task = { ...row("background_task:fixture", "blocked"), source: "task_registry", message: "fixture task stopped" };
  f.rows([...connectedRows(), task]); await f.refresh();
  await expect(summary).toContainText("运行状态异常");
  await expect(evidence).toContainText("后台任务 · 受限");
  f.risk("block"); await f.refresh();
  await expect(summary).toContainText("风险已阻断");
  f.risk("warn"); f.rows([]); await f.refresh();
  await expect(summary).toContainText("风险警告");
  await expect(evidence).toContainText("等待确认");

  f.risk("ok"); f.rows(connectedRows()); await f.refresh();
  await expect(summary).toHaveAttribute("data-state", "healthy");
  const connectionCount = f.connections();
  for (const socket of f.channelSockets.get("system")!) await socket.close();
  await expect(page.getByTestId("status-app-ws")).toContainText("已断开");
  await expect(summary).toHaveAttribute("data-state", "warning");
  await expect(evidence).toContainText("应用连接");
  await f.refresh();
  await expect.poll(f.connections).toBeGreaterThan(connectionCount);
  await expect(page.getByTestId("status-app-ws")).toContainText("已订阅");
  await expect(summary).toHaveAttribute("data-state", "healthy");

  f.rows([...connectedRows(), { ...row("app_ws_broadcast:system", "warn"), rows: 5, requested: 1 }]);
  await f.refresh();
  await expect(page.getByTestId("status-app-ws")).toContainText("丢帧 5");
  await expect(summary).toHaveAttribute("data-state", "warning");
  f.rows([...connectedRows(), { ...row("app_ws_broadcast:system"), rows: 500, requested: 100 }]);
  await f.refresh();
  await expect(summary).toHaveAttribute("data-state", "healthy");
  f.fail(true); await f.refresh();
  await expect(summary).toHaveAttribute("data-state", "degraded");
  await expect(evidence).toContainText("HEALTH_READ_UNAVAILABLE");
  f.fail(false); await f.refresh();
  await expect(summary).toHaveAttribute("data-state", "healthy");
  f.freezeHealth();
  for (let i = 0; i < 4; i++) await f.refresh();
  await expect(summary).toContainText("运行状态待确认");
  await expect(evidence).toContainText("运行状态 · 已过期");
  for (const id of ["status-market-data", "status-api-runtime", "status-private-ws"]) {
    await expect(page.getByTestId(id)).toContainText("待确认");
    await expect(page.getByTestId(id)).toHaveAttribute("data-state", "unknown");
  }
  f.freezeHealth(false); await f.refresh();
  await expect(summary).toHaveAttribute("data-state", "healthy");
  expect(f.writes).toEqual([]); expect(f.errors).toEqual([]);
});
