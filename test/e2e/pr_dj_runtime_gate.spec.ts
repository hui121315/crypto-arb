import { expect, test, type Page } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";
const WEB_BASE = process.env.CROSSLINE_E2E_WEB_BASE ?? "http://127.0.0.1:18080";

function operation(operation, status, source, message, extra = {}) {
  return {
    venue: "gate",
    operation,
    status,
    source,
    message,
    supported: true,
    configured: true,
    requested: 1,
    rows: status === "ok" ? 1 : 0,
    freshnessMs: 250,
    retryAfterMs: null,
    latencyMs: null,
    latencyP95Ms: null,
    error: status === "ok" ? null : message,
    evidence: null,
    problem: null,
    observedAtMs: 1_789_000_000_000,
    ...extra,
  };
}

function operationHealthSnapshot() {
  const rows = [
    operation("rest_funding_rates", "ok", "market_data_cache", "fresh funding rows"),
    operation("private_read", "ok", "private_read", "account read verified"),
    operation("private_ws_session", "warn", "private_ws_runtime", "private WS reconnecting", {
      retryAfterMs: 5_000,
      problem: {
        code: "PRIVATE_WS_RUNTIME_FAILED",
        message: "private WS reconnecting",
        retryAfterMs: 5_000,
        source: "private_ws_runtime",
      },
    }),
    operation(
      "host_gate:api.gateio.ws",
      "blocked",
      "host_gate",
      "HostGate api.gateio.ws 熔断开启 12000ms，连续失败 5",
      {
        configured: null,
        requested: null,
        rows: 3,
        retryAfterMs: 12_000,
        problem: {
          code: "CIRCUIT_BREAKER_OPEN",
          message: "HostGate api.gateio.ws 熔断开启 12000ms，连续失败 5",
          retryAfterMs: 12_000,
          source: "host_gate",
          details: {
            venue: "gate",
            host: "api.gateio.ws",
            cause: "circuit_open",
            circuitRetryAfterMs: 12_000,
          },
        },
      },
    ),
    operation(
      "rate_limiter:gate",
      "warn",
      "rate_limiter",
      "RateLimiter gate 最近等待 80ms，try_acquire 拒绝 2 次",
      {
        configured: null,
        requested: 20,
        rows: 4,
        problem: {
          code: "RATE_LIMITER_PRESSURE",
          message: "RateLimiter gate 最近等待 80ms，try_acquire 拒绝 2 次",
          source: "rate_limiter",
        },
      },
    ),
  ];
  return {
    rows,
    generatedAtMs: 1_789_000_000_000,
    rowCount: rows.length,
    attentionCount: 3,
    retryAfterMs: 12_000,
  };
}

async function useRuntimeGateScenario(page: Page) {
  await page.addInitScript((apiBase) => {
    window.localStorage.setItem("api_base", JSON.stringify(apiBase));
    window.localStorage.setItem("api_auth_token", JSON.stringify("e2e-token"));
  }, API_BASE);
  await page.routeWebSocket("**/ws", (socket) => socket.close());
  await page.route("**/api/system/venue-operation-health**", async (route) => {
    const headers = {
      "access-control-allow-origin": WEB_BASE,
      "access-control-allow-methods": "GET,OPTIONS",
      "access-control-allow-headers": "authorization,accept,x-request-id",
      "content-type": "application/json; charset=utf-8",
      vary: "origin",
    };
    if (route.request().method() === "OPTIONS") {
      await route.fulfill({ status: 204, headers, body: "" });
      return;
    }
    await route.fulfill({
      status: 200,
      headers,
      body: JSON.stringify(operationHealthSnapshot()),
    });
  });
}

test("PR-DJ top status keeps four runtime scopes distinct and exposes circuit retry", async ({
  page,
}) => {
  await useRuntimeGateScenario(page);
  await page.goto("/#futures");

  const status = page.getByTestId("top-status-bar");
  const market = status.getByTestId("status-market-data");
  const trading = status.getByTestId("status-api-runtime");
  const privateWs = status.getByTestId("status-private-ws");
  const appWs = status.getByTestId("status-app-ws");

  await expect(market).toContainText("MarketData");
  await expect(market).toContainText("1/1");
  await expect(trading).toContainText("TradingAPI");
  await expect(trading).toContainText("1可用/1配置");
  await expect(trading).not.toHaveClass(/degraded/);
  await expect(trading).toHaveAttribute("title", /private_read/);
  await expect(trading).not.toHaveAttribute("title", /Transport：gate/);
  await expect(trading).not.toHaveAttribute("title", /host_gate:api\.gateio\.ws/);
  await expect(trading).not.toHaveAttribute("title", /CIRCUIT_BREAKER_OPEN/);
  await expect(privateWs).toContainText("PrivateWS");
  await expect(privateWs).toContainText("0可用\/1配置");
  await expect(privateWs).toHaveAttribute("title", /retry 5000ms/);
  await expect(appWs).toContainText("AppWS");
  await expect(appWs).not.toContainText("1可用/1配置");
});

test("PR-DJ Settings exposes typed HostGate and RateLimiter diagnostics", async ({ page }) => {
  await useRuntimeGateScenario(page);
  await page.goto("/#settings");
  await page.getByRole("tab", { name: "诊断" }).click();

  const hostGate = page
    .locator(".settings-table tbody tr")
    .filter({ hasText: "host_gate:api.gateio.ws" });
  await expect(hostGate).toHaveCount(1);
  await expect(hostGate).toContainText("CIRCUIT_BREAKER_OPEN");
  await expect(hostGate).toContainText("source host_gate");
  await expect(hostGate).toContainText("retry 12000ms");

  const limiter = page
    .locator(".settings-table tbody tr")
    .filter({ hasText: "rate_limiter:gate" });
  await expect(limiter).toHaveCount(1);
  await expect(limiter).toContainText("RATE_LIMITER_PRESSURE");
  await expect(limiter).toContainText("source rate_limiter");
  await expect(limiter).toContainText("拒绝 2 次");
});
