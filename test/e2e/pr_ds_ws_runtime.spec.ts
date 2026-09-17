import { expect, test, type Page, type Route } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";
const WEB_BASE = process.env.CROSSLINE_E2E_WEB_BASE ?? "http://127.0.0.1:18080";

const corsHeaders = {
  "access-control-allow-origin": WEB_BASE,
  "access-control-allow-methods": "GET,POST,OPTIONS",
  "access-control-allow-headers": "authorization,accept,content-type,x-request-id",
  "content-type": "application/json; charset=utf-8",
  vary: "origin",
};

async function fulfillJson(route: Route, body: unknown) {
  if (route.request().method() === "OPTIONS") {
    await route.fulfill({ status: 204, headers: corsHeaders, body: "" });
    return;
  }
  await route.fulfill({ status: 200, headers: corsHeaders, body: JSON.stringify(body) });
}

function appWsLagSnapshot() {
  const rows = [
    {
      venue: "app",
      operation: "app_ws_broadcast:orders",
      status: "warn",
      source: "app_ws_hub",
      message:
        "app WS channel orders: subscribers=1, lag_events=2, skipped_messages=7, last_lag_at_ms=1789000000000",
      supported: true,
      configured: true,
      requested: 2,
      rows: 7,
      freshnessMs: 250,
      retryAfterMs: 100,
      latencyMs: null,
      latencyP95Ms: null,
      error: "channel orders lagged",
      evidence: null,
      problem: {
        code: "WS_BROADCAST_LAGGED",
        message: "channel orders lagged; skipped 7 broadcast messages",
        retryAfterMs: 100,
        source: "app_ws_hub",
        details: { channel: "orders", lagEvents: 2, skippedMessages: 7 },
      },
      observedAtMs: 1_789_000_000_000,
    },
  ];
  return {
    rows,
    generatedAtMs: 1_789_000_000_250,
    rowCount: rows.length,
    attentionCount: 1,
    retryAfterMs: 100,
  };
}

async function usePrDsScenario(page: Page) {
  await page.addInitScript((apiBase) => {
    window.localStorage.setItem("api_base", JSON.stringify(apiBase));
    window.localStorage.setItem("api_auth_token", JSON.stringify("e2e-token"));
  }, API_BASE);
  await page.route("**/api/auth/ws-ticket**", (route) =>
    fulfillJson(route, { ticket: "pr-ds-ticket", expiresAtMs: Date.now() + 60_000 }),
  );
  await page.route("**/api/system/venue-operation-health**", (route) =>
    fulfillJson(route, appWsLagSnapshot()),
  );
  await page.routeWebSocket("**/ws", (socket) => {
    socket.onMessage((raw) => {
      const message = JSON.parse(raw.toString());
      if (message.type !== "subscribe") return;
      socket.send(
        JSON.stringify({
          type: "ack",
          subscribed: message.channels ?? [],
          requestId: message.requestId,
        }),
      );
    });
  });
}

test("PR-DS surfaces app WS lag in the top status and Settings diagnostics", async ({ page }) => {
  await usePrDsScenario(page);
  await page.goto("/#settings");

  const appWs = page.getByTestId("status-app-ws");
  await expect(appWs).toContainText("AppWS");
  await expect(appWs).toContainText("丢帧 7");
  await expect(appWs).toHaveClass(/degraded/);
  await expect(appWs).toHaveAttribute("title", /lag 事件 2/);
  await expect(appWs).toHaveAttribute("title", /累计丢帧 7/);

  await page.getByRole("tab", { name: "诊断" }).click();
  await expect(page.getByText(/AppWS channels 1 · lag 2 · 丢帧 7/)).toBeVisible();
  const wsTable = page.getByRole("table").filter({
    has: page.getByRole("columnheader", { name: "频道", exact: true }),
  });
  const row = wsTable.getByRole("row").filter({ hasText: "app_ws_broadcast:orders" });
  await expect(row).toHaveCount(1);
  await expect(row).toContainText("WS_BROADCAST_LAGGED");
  await expect(row).toContainText("source app_ws_hub");
  await expect(row).toContainText("retry 100ms");
});
