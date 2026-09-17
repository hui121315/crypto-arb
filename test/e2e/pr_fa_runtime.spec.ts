import { expect, test, type Page } from "@playwright/test";
import { createServer } from "node:http";
import { gzipSync } from "node:zlib";
const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";
const WEB_BASE = process.env.CROSSLINE_E2E_WEB_BASE ?? "http://127.0.0.1:18080";
const SCENARIO_BASE = `${API_BASE}/e2e-portfolio-snapshot-502`;
const SNAPSHOT_PATH = "/api/trading/portfolio/snapshot";
const OPPORTUNITY_PATH = "/api/v3/arbitrage/opportunities/list";
const NOW = 1_770_000_000_000;
const FINGERPRINT = "hmac-sha256:0123456789abcdef01234567";
function typedProblem(code: string, message: string, requestId: string) {
  return {
    code,
    message,
    status: 503,
    source: "e2e-pr-fa",
    requestId,
    retryAfterMs: 2_000,
    details: { lane: "pr-fa-browser", trustedEmpty: false },
  };
}
function operation(venue: string, kind: string, status: string, rows: number) {
  return {
    venue,
    operation: kind,
    status,
    source: "account_cache",
    message: status === "ok" ? "fresh account sample" : "venue read degraded",
    supported: true,
    configured: true,
    rows,
    freshnessMs: status === "ok" ? 120 : 4_000,
    observedAtMs: NOW,
  };
}
function accountBindings() {
  return [
    {
      venue: "okx",
      accountScope: "unified_margin",
      status: "verified",
      source: "account_mode_read",
      checkedAtMs: NOW - 120,
      freshnessMs: 120,
      credentialFingerprint: FINGERPRINT,
      problem: null,
    },
    {
      venue: "gate",
      accountScope: null,
      status: "unverified",
      source: "account_binding_runtime",
      checkedAtMs: NOW - 4_000,
      freshnessMs: 4_000,
      credentialFingerprint: null,
      problem: typedProblem(
        "ACCOUNT_SCOPE_UNVERIFIED",
        "Gate account scope has not been verified",
        "req-account-scope-gate",
      ),
    },
  ];
}
function balanceRows() {
  return [
    { venue: "okx", currency: "USDT", total: 8_000, available: 7_700, frozen: 300, unrealizedPnl: 12 },
    { venue: "gate", currency: "USDT", total: 2_000, available: 1_900, frozen: 100, unrealizedPnl: -3 },
  ];
}
function risk() {
  return {
    var991dUsd: 25,
    varPctOfNav: 0.0025,
    fundingClustering: [],
    deltaConcentration: [],
    marginUtilization: [],
    hardLimits: {
      openOrdersUsed: 0,
      openOrdersMax: 10,
      maxSymbolNotionalUsd: 10_000,
      maxOrderNotionalUsd: 5_000,
      killSwitchActive: false,
    },
    updatedAtMs: NOW,
  };
}
function accountEnvelope(mode: "partial" | "warming" = "partial") {
  const warming = typedProblem(
    "ACCOUNT_STATE_SNAPSHOT_WARMING",
    "Account snapshot is warming; no trusted empty state is available",
    "req-account-warming",
  );
  const positionsProblem = typedProblem(
    "POSITION_READ_DEGRADED",
    "Gate positions read failed; balances remain fresh",
    "req-gate-positions",
  );
  const balances = mode === "warming" ? [] : balanceRows();
  const activeProblem = mode === "warming" ? warming : positionsProblem;
  const bindings = mode === "warming" ? [] : accountBindings();
  const balanceStatus = mode === "warming" ? "degraded" : "fresh";
  const balanceProblems = mode === "warming" ? [warming] : [];
  const balanceHealth = mode === "warming" ? [] : [operation("okx", "balance", "ok", 1), operation("gate", "balance", "ok", 1)];
  const positionHealth = [operation("gate", "positions", "warn", 0)];
  const accountState = {
    balances: {
      rows: balances,
      rowCount: balances.length,
      status: balanceStatus,
      source: mode === "warming" ? "account_state_warming" : "account_balance_runtime",
      observedAtMs: NOW,
      problems: balanceProblems,
      operationHealth: balanceHealth,
      fieldQuality: [],
      rowHealth: [],
      accountBindings: bindings,
    },
    positions: {
      rows: [],
      rowCount: 0,
      status: "degraded",
      source: mode === "warming" ? "account_state_warming" : "account_position_runtime",
      observedAtMs: NOW,
      problems: [activeProblem],
      operationHealth: positionHealth,
      fieldQuality: [],
      rowHealth: [],
      accountBindings: bindings,
    },
    status: "degraded",
    source: mode === "warming" ? "account_state_warming" : "account_state_runtime",
    observedAtMs: NOW,
    problems: [activeProblem],
    operationHealth: [...balanceHealth, ...positionHealth],
    fieldQuality: [],
  };
  const snapshot = {
    summary: {
      totalNavUsd: mode === "warming" ? 0 : 10_000,
      navChange24hPct: 0,
      netDeltaUsd: 0,
      netDeltaPctOfNav: 0,
      nakedExposureUsd: 0,
      nakedPositionCount: 0,
      realizedPnlTodayUsd: 0,
      pnlBreakdown: { fundingUsd: 0, priceUsd: 0, feeRebateUsd: 0 },
      updatedAtMs: NOW,
    },
    positions: [],
    balances,
    risk: risk(),
    serverNowMs: NOW,
    degraded: true,
    problems: [{
      scope: "portfolio",
      operation: mode === "warming" ? "account_state" : "positions",
      code: activeProblem.code,
      message: activeProblem.message,
      venue: mode === "warming" ? null : "gate",
      retryAfterMs: activeProblem.retryAfterMs,
      observedAtMs: NOW,
    }],
    operationHealth: [...balanceHealth, ...positionHealth],
    accountState,
    recentCloseRuns: [],
  };
  return {
    status: "degraded",
    source: "e2e-pr-fa",
    observedAtMs: NOW,
    snapshot,
    problem: activeProblem,
    problems: [activeProblem],
    operationHealth: snapshot.operationHealth,
    retryAfterMs: activeProblem.retryAfterMs,
  };
}
async function useApi(page: Page, apiBase: string) {
  await page.addInitScript((base) => {
    localStorage.setItem("api_base", JSON.stringify(base));
    localStorage.setItem("api_auth_token", JSON.stringify("e2e-token"));
  }, apiBase);
}
test("account evidence isolates section freshness and rejects trusted warming empties", async ({ page }) => {
  let mode: "partial" | "warming" = "partial";
  await useApi(page, SCENARIO_BASE);
  await page.route(`**/e2e-portfolio-snapshot-502${SNAPSHOT_PATH}`, async (route) => {
    if (route.request().method() !== "GET") return route.continue();
    await route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(accountEnvelope(mode)) });
  });
  await page.goto("/#positions");
  await expect(page.getByRole("heading", { name: "持仓/风控" })).toBeVisible();
  const balances = page.getByRole("heading", { name: "余额", exact: true }).locator("..").locator("..").locator(".balance-panel");
  await expect(balances.locator(".balance-row")).toHaveCount(2);
  await expect(balances.locator(".stale-note")).toHaveCount(0);
  await expect(balances).not.toContainText("余额刷新失败");
  const positions = page.locator(".positions-main");
  await expect(page.locator(".runtime-problems")).toContainText("POSITION_READ_DEGRADED");
  await expect(positions).toContainText("request_id req-gate-positions");
  await expect(positions).not.toContainText("暂无持仓");
  const verified = balances.locator(`[title*="${FINGERPRINT}"]`);
  await expect(verified).toBeVisible();
  await expect(verified).toHaveAttribute("title", /okx.*unified_margin.*verified/i);
  const unverified = positions.locator('[title*="ACCOUNT_SCOPE_UNVERIFIED"]');
  await expect(unverified).toBeVisible();
  await expect(unverified).toHaveAttribute("title", /req-account-scope-gate.*retry 2s/i);
  mode = "warming";
  await page.reload();
  const warming = page.locator(".positions-layout");
  await expect(warming).toContainText("ACCOUNT_STATE_SNAPSHOT_WARMING");
  await expect(warming.locator('[title*="req-account-warming"]')).toBeVisible();
  await expect(warming).not.toContainText("暂无持仓");
  await expect(warming).not.toContainText("暂无可用余额");
});
test("gzip hot-path transfer records browser, Wasm, and bounded render metrics", async ({ page, request }) => {
  const upstream = await request.get(`${API_BASE}/e2e-large-tables${OPPORTUNITY_PATH}`);
  expect(upstream.ok()).toBeTruthy();
  const decoded = await upstream.body();
  const compressed = gzipSync(decoded);
  const transferBytes = { decoded: decoded.byteLength, encoded: compressed.byteLength };
  const server = createServer((request, response) => {
    const preflight = request.method === "OPTIONS";
    response.writeHead(preflight ? 204 : 200, {
      "access-control-allow-origin": WEB_BASE,
      "access-control-allow-methods": "GET,OPTIONS",
      "access-control-allow-headers": "authorization,accept,x-request-id",
      "access-control-expose-headers": "content-encoding,content-length",
      "content-encoding": "gzip",
      "content-length": preflight ? 0 : compressed.byteLength,
      "content-type": "application/json; charset=utf-8",
    });
    response.end(preflight ? undefined : compressed);
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  server.unref();
  const address = server.address();
  if (!address || typeof address === "string") throw new Error("gzip fixture did not bind TCP");
  try {
    await useApi(page, `${API_BASE}/e2e-large-tables`);
    await page.addInitScript((path) => {
      const runtime = window as any;
      runtime.__crosslineWasmSerdeMetrics = [];
      runtime.__prFaTransfers = [];
      const originalFetch = window.fetch.bind(window);
      window.fetch = async (input, init) => {
        const response = await originalFetch(input, init);
        const href = typeof input === "string" ? input : input instanceof Request ? input.url : String(input);
        if (new URL(href, location.href).pathname.endsWith(path)) {
          const started = performance.now();
          const text = await response.clone().text();
          const cloneReadMs = performance.now() - started;
          const parseStarted = performance.now();
          JSON.parse(text);
          runtime.__prFaTransfers.push({
            contentEncoding: response.headers.get("content-encoding"),
            contentLength: response.headers.get("content-length"),
            decodedBytes: new TextEncoder().encode(text).byteLength,
            cloneReadMs,
            jsonParseMs: performance.now() - parseStarted,
          });
        }
        return response;
      };
    }, OPPORTUNITY_PATH);
    await page.route(`**/e2e-large-tables${OPPORTUNITY_PATH}*`, async (route) => {
      await route.continue({ url: `http://127.0.0.1:${address.port}${OPPORTUNITY_PATH}` });
    });
    await page.goto("/#opportunities");
    await expect(page.getByRole("heading", { name: "机会扫描" })).toBeVisible();
    await expect(page.locator(".opportunity-layout table.clean-table tbody tr")).toHaveCount(50);
    await expect.poll(() => page.evaluate(() => (window as any).__prFaTransfers.length))
      .toBeGreaterThanOrEqual(1);
    const metrics = await page.evaluate(() => {
      const runtime = window as any;
      const render = document.querySelector(".opportunity-layout");
      return {
        transfer: runtime.__prFaTransfers[0],
        wasm: runtime.__crosslineWasmSerdeMetrics,
        domNodes: render?.querySelectorAll("*").length ?? 0,
        tableRows: render?.querySelectorAll("tbody tr").length ?? 0,
        tableCells: render?.querySelectorAll("th,td").length ?? 0,
      };
    });
    expect(metrics.transfer).toMatchObject({ contentEncoding: "gzip", contentLength: String(transferBytes.encoded), decodedBytes: transferBytes.decoded });
    expect(transferBytes.encoded).toBeLessThanOrEqual(64 * 1024);
    expect(metrics.transfer.decodedBytes).toBeLessThanOrEqual(180 * 1024);
    expect(metrics.transfer.cloneReadMs).toBeLessThanOrEqual(750);
    expect(metrics.transfer.jsonParseMs).toBeLessThanOrEqual(100);
    const wasm = metrics.wasm.filter((entry) => entry.path.endsWith(OPPORTUNITY_PATH));
    expect(wasm.length).toBeGreaterThanOrEqual(1);
    expect(wasm[0]).toMatchObject({ success: true });
    expect(wasm[0].decodeMs).toBeLessThanOrEqual(750);
    expect(metrics.domNodes).toBeLessThanOrEqual(2_500);
    expect(metrics.tableRows).toBeLessThanOrEqual(64);
    expect(metrics.tableCells).toBeLessThanOrEqual(1_024);
  } finally {
    server.closeAllConnections();
    await new Promise<void>((resolve) => server.close(() => resolve()));
  }
});
