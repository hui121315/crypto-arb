import { expect, test, type Page, type Route } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";
const WEB_BASE = process.env.CROSSLINE_E2E_WEB_BASE ?? "http://127.0.0.1:18080";
const API_VERSION = "9.8.7-pr-au";

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

function systemHealthEnvelope() {
  return {
    data: {
      apiVersion: API_VERSION,
      api: { healthy: 8, total: 8, failedVenues: [] },
      ws: { channels: 3, disconnected: [] },
      orderElapsedMs: 24,
      risk: "ok",
      netDeltaUsd: 0,
      netDeltaPctOfNav: 0,
      nextFunding: null,
      updatedAtMs: 1_789_000_000_000,
      degraded: false,
      problems: [],
    },
    status: "ready",
    source: "system-health-snapshot",
    observedAtMs: 1_789_000_000_000,
    problems: [],
  };
}

function appWsChannelSnapshot() {
  const rows = [{
    venue: "app",
    operation: "app_ws_broadcast:execution",
    status: "warn",
    source: "app_ws_hub",
    message:
      "app WS channel execution: subscribers=1, lag_events=1, skipped_messages=3",
    supported: true,
    configured: true,
    requested: 1,
    rows: 3,
    freshnessMs: 125,
    retryAfterMs: 250,
    latencyMs: null,
    latencyP95Ms: null,
    error: "execution channel lagged",
    evidence: null,
    problem: {
      code: "WS_BROADCAST_LAGGED",
      message: "execution channel skipped 3 broadcast messages",
      requestId: "req-pr-au-execution",
      retryAfterMs: 250,
      source: "app_ws_hub",
      details: { channel: "execution", lagEvents: 1, skippedMessages: 3 },
    },
    observedAtMs: 1_789_000_000_000,
  }];
  return {
    rows,
    generatedAtMs: 1_789_000_000_125,
    rowCount: rows.length,
    attentionCount: 1,
    retryAfterMs: 250,
  };
}

async function usePrAuScenario(page: Page) {
  const channels = new Set<string>();
  let socketCount = 0;
  await page.addInitScript((apiBase) => {
    window.localStorage.setItem("api_base", JSON.stringify(apiBase));
    window.localStorage.setItem("api_auth_token", JSON.stringify("e2e-token"));
  }, API_BASE);
  await page.route("**/api/system/health**", (route) =>
    fulfillJson(route, systemHealthEnvelope()),
  );
  await page.route("**/api/auth/ws-ticket**", (route) =>
    fulfillJson(route, { ticket: "pr-au-ticket", expiresAtMs: Date.now() + 60_000 }),
  );
  await page.route("**/api/system/venue-operation-health**", (route) =>
    fulfillJson(route, appWsChannelSnapshot()),
  );
  await page.routeWebSocket("**/ws", (socket) => {
    socketCount += 1;
    socket.onMessage((raw) => {
      const message = JSON.parse(raw.toString());
      if (message.type !== "subscribe") return;
      for (const channel of message.channels ?? []) channels.add(channel);
      socket.send(JSON.stringify({
        type: "ack",
        subscribed: message.channels ?? [],
        requestId: message.requestId,
      }));
    });
  });
  return {
    channels,
    socketCount: () => socketCount,
  };
}

test("PR-AU validates API version and surfaces per-channel Settings health", async ({ page }) => {
  await usePrAuScenario(page);
  await page.goto("/#settings");
  await page.getByRole("tab", { name: "诊断" }).click();

  await page.getByRole("button", { name: "验证连通" }).click();
  const editor = page.locator(".api-base-editor", { has: page.getByLabel("API Base") });
  await expect(editor).toContainText(`version ${API_VERSION}`);
  await expect(editor).toContainText("/api/auth/ws-ticket 探测通过");

  await expect(page.getByText(/AppWS channels 1 · lag 1 · 丢帧 3/)).toBeVisible();
  const table = page.getByRole("table").filter({
    has: page.getByRole("columnheader", { name: "频道", exact: true }),
  });
  const row = table.getByRole("row").filter({ hasText: "app_ws_broadcast:execution" });
  await expect(row).toHaveCount(1);
  await expect(row).toContainText("WS_BROADCAST_LAGGED");
  await expect(row).toContainText("request_id req-pr-au-execution");
  await expect(row).toContainText("retry 250ms");
});

test("PR-AU execution page exposes the multiplexed execution channel ACK", async ({ page }) => {
  const runtime = await usePrAuScenario(page);
  await page.goto("/#execution");
  await expect(page.getByRole("heading", { name: "对冲执行" })).toBeVisible();

  const status = page.locator(".execution-status-bar");
  await expect(status).toContainText("execution 通道");
  await expect(status).toContainText("WS已订阅，等待首帧");
  await expect(status).toContainText("帧 0 · 错误 0");
  await expect.poll(() => runtime.socketCount()).toBe(1);
  await expect.poll(() => Array.from(runtime.channels).sort()).toEqual(
    expect.arrayContaining(["execution", "orders", "system"]),
  );
});
