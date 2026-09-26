import { expect, test, type Page } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";
const SCENARIO = "e2e-order-cancel-denied";

async function useExecutionScenario(page: Page) {
  await page.addInitScript(
    ({ apiBase, scenario }) => {
      window.localStorage.setItem("api_base", JSON.stringify(`${apiBase}/${scenario}`));
      window.localStorage.setItem("api_auth_token", JSON.stringify("e2e-token"));
    },
    { apiBase: API_BASE, scenario: SCENARIO },
  );
  await page.routeWebSocket("**/ws", (socket) => socket.close());
}

async function injectHttpOperationProblem(page: Page) {
  await page.route("**/api/arbitrage/opportunities/mock-mu-perp/preview", async (route) => {
    const upstream = await route.fetch();
    const response = await upstream.json();
    response.ticket.guards.push({
      key: "live_operation_health",
      label: "实盘运行状态数据依据",
      passed: false,
      detail: "binance BTCUSDT 订单簿未通过: rate limited",
      preflightOutcome: {
        status: "blocked",
        checkedAtMs: 1_780_000_000_000,
        scope: {
          venues: ["binance"],
          symbols: ["BTCUSDT"],
          accountModes: [],
          operations: ["orderbook"],
        },
        observedVenues: ["binance"],
        source: "venue_operation_health.http+host_gate+rate_limiter",
        freshnessMs: 37,
        retryAfterMs: 2_000,
        requestId: "req-pr-an-http",
        problems: [
          {
            code: "UPSTREAM_HTTP",
            message: "rate limited",
            status: 429,
            requestId: "req-pr-an-http",
            retryAfterMs: 2_000,
            source: "binance",
            details: {
              operation: "rest_orderbooks",
              symbol: "BTCUSDT",
              path: "/fapi/v1/depth",
              latencyMs: 37,
            },
          },
        ],
        fieldQuality: [],
        rowHealth: [],
        error: "rate limited",
      },
    });
    await route.fulfill({ response: upstream, json: response });
  });
}

test("PR-AN execution preflight exposes HTTP and fanout problem context", async ({ page }) => {
  await useExecutionScenario(page);
  await injectHttpOperationProblem(page);

  await page.goto("/#futures");
  await expect(page.getByRole("heading", { name: "期货套利" })).toBeVisible();
  await page.getByRole("button", { name: "构建对冲" }).click();
  await expect(page.getByRole("heading", { name: "对冲执行" })).toBeVisible();

  const runtime = page.locator(".check-item").filter({ hasText: "实盘运行状态数据依据" });
  await expect(runtime).toContainText("rest_orderbooks");
  await expect(runtime).toContainText("BTCUSDT");
  await expect(runtime).toContainText("/fapi/v1/depth");
  await expect(runtime).toContainText("HTTP耗时 37ms");
  await expect(runtime).toContainText("req req-pr-an-http");
  await expect(runtime).toContainText("retry 2000ms");
});
