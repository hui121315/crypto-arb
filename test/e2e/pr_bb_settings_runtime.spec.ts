import { expect, test, type Page, type Route } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";
const WEB_BASE = process.env.CROSSLINE_E2E_WEB_BASE ?? "http://127.0.0.1:18080";
const CHECKED_AT_MS = 1_790_000_000_000;

function headers(requestId: string) {
  return {
    "access-control-allow-origin": WEB_BASE,
    "access-control-allow-methods": "GET,POST,OPTIONS",
    "access-control-allow-headers":
      "authorization,accept,content-type,x-request-id,idempotency-key",
    "access-control-expose-headers": "retry-after,x-request-id",
    "content-type": "application/json; charset=utf-8",
    "x-request-id": requestId,
    vary: "origin",
  };
}

async function fulfillJson(route: Route, body: unknown, requestId: string) {
  if (route.request().method() === "OPTIONS") {
    await route.fulfill({ status: 204, headers: headers(requestId), body: "" });
    return;
  }
  await route.fulfill({
    status: 200,
    headers: headers(requestId),
    body: JSON.stringify(body),
  });
}

async function useMockApi(page: Page) {
  await page.addInitScript((apiBase) => {
    window.localStorage.setItem("api_base", JSON.stringify(apiBase));
    window.localStorage.setItem("api_auth_token", JSON.stringify("e2e-token"));
  }, API_BASE);
  await page.routeWebSocket("**/ws**", (socket) => socket.close());
}

function probe(kind: string, status: string, requestId: string) {
  return {
    kind,
    status,
    scope: `okx_${kind}`,
    source: `credential_validation.${kind}`,
    message: `${kind} ${status}`,
    checkedAtMs: CHECKED_AT_MS,
    requestId,
  };
}

function credentialsResponse() {
  return {
    venues: [{
      venue: "okx",
      label: "OKX",
      fields: [
        ["api_key", "API Key", "OKX_API_KEY"],
        ["api_secret", "API Secret", "OKX_API_SECRET"],
        ["passphrase", "Passphrase", "OKX_PASSPHRASE"],
      ].map(([key, label, envKey]) => ({
        key,
        label,
        envKey,
        configured: true,
        secret: true,
        required: true,
        source: "keychain",
      })),
      publicMarket: true,
      privateRead: true,
      testnetWrite: false,
      liveWrite: true,
      note: "runtime permission evidence remains authoritative",
      missingFields: [],
      validationEvidence: {
        status: "read_only_ok",
        checkedAtMs: CHECKED_AT_MS,
        probes: [
          probe("balance_read", "ok", "req-pr-bb-balance"),
          probe("positions_read", "ok", "req-pr-bb-positions"),
          probe("open_orders_read", "ok", "req-pr-bb-open"),
          probe("order_permission", "failed", "req-pr-bb-place"),
          probe("account_mode_read", "ok", "req-pr-bb-account"),
        ],
        permissionEvidence: [
          {
            permission: "open_orders_read",
            status: "validated",
            probeKind: "open_orders_read",
            permissionScope: "open_orders_read",
            source: "exchange_adapter.get_open_orders",
            message: "open orders read succeeded",
            checkedAtMs: CHECKED_AT_MS,
            requestId: "req-pr-bb-open",
          },
          {
            permission: "place_order",
            status: "denied",
            probeKind: "order_permission",
            permissionScope: "place_order",
            source: "okx.POST /api/v5/trade/order-precheck",
            message: "place order permission denied",
            checkedAtMs: CHECKED_AT_MS + 1,
            requestId: "req-pr-bb-place",
            error: "place order permission denied",
          },
          {
            permission: "cancel_order",
            status: "unproven",
            probeKind: "order_permission",
            permissionScope: "cancel_order",
            source: "not_probed",
            message: "cancel order permission remains unproven",
            checkedAtMs: CHECKED_AT_MS + 2,
            requestId: "req-pr-bb-cancel",
          },
        ],
      },
    }],
    secretStorage: {
      mode: "keychain",
      health: "ready",
      persistent: true,
      encrypted: true,
      atomicWrite: true,
      label: "macOS Keychain",
      message: "ready",
    },
  };
}

function operation(operation: string, status: string, source: string, usable: boolean, requestId: string) {
  return {
    operation,
    status,
    source,
    freshnessMs: 100,
    requestId,
    capabilityStatus: "supported",
    configurationStatus: "configured",
    currentlyUsable: usable,
    observedAtMs: CHECKED_AT_MS,
    ...(status === "blocked" ? {
      lastError: `${operation} blocked`,
      problem: {
        code: "VENUE_OPERATION_BLOCKED",
        message: `${operation} blocked`,
        requestId,
        source,
      },
    } : {}),
  };
}

function venueRuntime(venue: string, placeStatus: string, cancelStatus: string) {
  return {
    venue,
    privateRest: operation("private_rest", "ok", `${venue}.private`, true, `req-${venue}-private`),
    openOrders: operation("open_orders", "ok", `${venue}.open`, true, `req-${venue}-open`),
    placeOrder: operation("place_order", placeStatus, `${venue}.place`, placeStatus === "ok", `req-${venue}-place`),
    cancelOrder: operation("cancel_order", cancelStatus, `${venue}.cancel`, cancelStatus === "ok", `req-${venue}-cancel`),
    orderStream: operation("order_stream", "ok", `${venue}.orders`, true, `req-${venue}-orders`),
    finality: operation("finality", "ok", `${venue}.finality`, true, `req-${venue}-finality`),
    generatedAtMs: CHECKED_AT_MS,
  };
}

function runtimeSnapshot() {
  return {
    venues: [
      venueRuntime("hyperliquid:km", "blocked", "unknown"),
      venueRuntime("kucoin", "ok", "ok"),
    ],
    generatedAtMs: CHECKED_AT_MS,
    venueCount: 2,
    operationCount: 12,
    currentlyUsableCount: 10,
    attentionCount: 2,
  };
}

function tradingStatus() {
  return {
    adapter: "live_router",
    environment: "live",
    openOrderCount: 0,
    risk: {
      liveTradingEnabled: true,
      killSwitchActive: false,
      maxOrderNotional: 5_000,
      maxOpenOrders: 10,
      maxHedgeImbalancePct: 0.05,
      liquidationWarnPct: 0.2,
      liquidationDangerPct: 0.1,
      allowedExchanges: ["hyperliquid:km", "kucoin"],
      allowedSymbols: ["MU"],
    },
    wsChannels: {
      orders: "orders",
      execution: "execution",
      riskAlerts: "risk_alerts",
    },
  };
}

function moduleTab(page: Page, title: string) {
  return page.locator(".module-tabs button").filter({ hasText: title });
}

test("PR-BB renders explicit open, place, and cancel credential permission evidence", async ({ page }) => {
  await useMockApi(page);
  await page.route("**/api/exchanges/credentials", (route) =>
    fulfillJson(route, credentialsResponse(), "req-pr-bb-credentials"));
  await page.goto("/#settings");

  const matrix = page.locator('[data-settings-table="credential-permissions"]');
  await expect(matrix).toBeVisible();
  await expect(matrix.locator("tbody tr")).toHaveCount(3);
  const open = matrix.locator("tbody tr").filter({ hasText: "读取挂单" });
  const place = matrix.locator("tbody tr").filter({ hasText: "下单" });
  const cancel = matrix.locator("tbody tr").filter({ hasText: "撤单" });
  await expect(open).toContainText("true");
  await expect(open).toContainText("req-pr-bb-open");
  await expect(place).toContainText("false");
  await expect(place).toContainText("denied");
  await expect(place).toContainText("place_order");
  await expect(place).toContainText("req-pr-bb-place");
  await expect(place).toContainText("place order permission denied");
  await expect(cancel).toContainText("unproven");
  await expect(cancel).toContainText("req-pr-bb-cancel");
  await expect(matrix).toContainText(String(CHECKED_AT_MS));
});

test("PR-BB converges live mode with the selected HedgeTicket venue pair", async ({ page }) => {
  await useMockApi(page);
  await page.route("**/api/trading/status", (route) =>
    fulfillJson(route, tradingStatus(), "req-pr-bb-status"));
  await page.route("**/api/system/venue-runtime-health**", (route) =>
    fulfillJson(route, runtimeSnapshot(), "req-pr-bb-runtime"));

  await page.goto("/#futures");
  await page.getByRole("button", { name: "构建对冲" }).click();
  await moduleTab(page, "设置").click();
  await page.getByRole("tab", { name: "诊断" }).click();

  const matrix = page.locator('[data-settings-table="ticket-venue-health"]');
  await expect(matrix).toBeVisible();
  await expect(matrix).toContainText("实盘");
  await expect(matrix).toContainText("mock-mu-perp");
  const long = matrix.locator("tbody tr").filter({ hasText: "Long" });
  const short = matrix.locator("tbody tr").filter({ hasText: "Short" });
  await expect(long).toContainText("hyperliquid:km");
  await expect(long.locator("td").nth(3)).toContainText("阻断");
  await expect(long.locator("td").nth(3)).toHaveAttribute("title", /req-hyperliquid:km-place/);
  await expect(short).toContainText("kucoin");
  await expect(short.locator("td").nth(3)).toContainText("正常");
  await expect(short.locator("td").nth(4)).toContainText("正常");
  await expect(matrix).toContainText("HedgeTicket 双腿预检是最终提交权威");
});
