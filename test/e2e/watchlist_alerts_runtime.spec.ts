import { expect, test } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";
const WEB_BASE = process.env.CROSSLINE_E2E_WEB_BASE ?? "http://127.0.0.1:18080";

test("settings exposes bounded watchlist prewarm and truthful toast queue runtime", async ({
  page,
  request,
}) => {
  await page.addInitScript((apiBase) => {
    window.localStorage.setItem(
      "api_base",
      JSON.stringify(`${apiBase}/e2e-watchlist-alerts-runtime`),
    );
    window.localStorage.setItem("api_auth_token", JSON.stringify("e2e-token"));
  }, API_BASE);

  await page.goto("/#settings");
  await expect(page.locator("h1", { hasText: "设置" })).toBeVisible();

  await expect.poll(async () => {
    const response = await request.get(`${API_BASE}/api/e2e/ws-auth-events`);
    const body = await response.json();
    return body.events
      .filter((event) => event.scenario === "watchlist-alerts-runtime")
      .flatMap((event) => event.channels ?? []);
  }).toEqual(expect.arrayContaining(["watchlist", "alerts"]));

  await page.getByRole("tab", { name: "诊断" }).click();
  const panel = page
    .getByText("自选与提醒运行状态", { exact: true })
    .locator("..")
    .locator("..");
  await expect(panel).toBeVisible();
  expect(await panel.locator(":scope > .settings-message.is-error").allTextContents()).toEqual([]);
  await expect(panel.locator(".settings-summary-line").first())
    .toContainText(
      "watchlist 已订阅 · 帧 1 · 错误 0 · alerts 已订阅 · 帧 2 · 错误 0",
    );

  const toast = page.locator(".toast-item").filter({ hasText: "BTC-USDT" });
  await expect(toast).toContainText("binance / okx");
  await expect(toast).toContainText("score 91.25");
  await expect(toast).toContainText("net 0.1234%");

  const watchlistRow = panel.locator("tbody tr").filter({ hasText: "BTC-USDT" });
  await expect(watchlistRow).toContainText("binance");
  await expect(watchlistRow).toContainText("okx");
  await expect(watchlistRow).toContainText("已截断");
  await expect(watchlistRow).toContainText("req 2 · ob 1 · ticker 1 · dedup 0 · capped 1");
  await expect(watchlistRow).toContainText("WATCHLIST_PREWARM_CAPPED");

  const queuedRule = panel
    .locator("tbody tr")
    .filter({ hasText: "app_websocket_toast" })
    .filter({ hasText: "已入队" });
  await expect(queuedRule).toContainText("public legs 2 · private WS 0");
  await expect(queuedRule).toContainText("count 3");

  const blockedRule = panel
    .locator("tbody tr")
    .filter({ hasText: "app_websocket_toast" })
    .filter({ hasText: "阻断" });
  await expect(blockedRule).toContainText("public legs 2 · private WS 0");
  await expect(blockedRule).toContainText("ALERT_TOAST_NOT_QUEUED");

  await expect(panel.locator(".settings-summary-line").filter({ hasText: "最近提醒" }))
    .toContainText("BTC-USDT · binance / okx · score 91.25");
  await expect(panel).not.toContainText("risk-alerts");
});

test("settings exposes durable watchlist storage and delivery provenance", async ({ page }) => {
  await page.addInitScript((apiBase) => {
    window.localStorage.setItem("api_base", JSON.stringify(apiBase));
    window.localStorage.setItem("api_auth_token", JSON.stringify("e2e-token"));
  }, API_BASE);
  await page.route(`${API_BASE}/api/watchlist`, async (route) => {
    await route.fulfill({
      contentType: "application/json",
      headers: { "access-control-allow-origin": WEB_BASE },
      body: JSON.stringify(durableWatchlistEnvelope()),
    });
  });
  await page.route(`${API_BASE}/api/alerts/rules`, async (route) => {
    await route.fulfill({
      contentType: "application/json",
      headers: { "access-control-allow-origin": WEB_BASE },
      body: JSON.stringify(durableAlertRulesEnvelope()),
    });
  });

  await page.goto("/#settings");
  await page.getByRole("tab", { name: "诊断" }).click();
  const panel = page
    .getByText("自选与提醒运行状态", { exact: true })
    .locator("..")
    .locator("..");
  await expect(panel).toBeVisible();
  await expect(panel).toContainText("durable · SQLite rev 12 正常", { timeout: 15_000 });

  const watchlistRow = panel.locator("tbody tr").filter({ hasText: "ETH-USDT" });
  await expect(watchlistRow).toContainText("v4 · API · 已持久化 · api-token:operator");
  await expect(watchlistRow).toContainText("正常");

  const alertRow = panel.locator("tbody tr").filter({ hasText: "app_websocket_toast" });
  await expect(alertRow).toContainText("v2 · API · 已持久化 · api-token:operator");
  await expect(alertRow).toContainText("冷却中");
  await expect(alertRow).toContainText("已入队");
  await expect(alertRow).toContainText("private WS 0");
});

function durableRuntime() {
  return {
    featureGate: "api_surface.watchlist_alerts",
    persistence: "sqlite_snapshot",
    volatile: false,
    restartBehavior: "restored_from_sqlite_snapshot",
    storage: {
      backend: "sqlite",
      configured: true,
      status: "ready",
      schemaVersion: 1,
      revision: 12,
      watchlistItemCount: 1,
      alertRuleCount: 1,
      persistAttempts: 12,
      persistSuccesses: 12,
      lastPersistedAtMs: 1_770_000_000_000,
    },
    publicOrderbookPrewarmLimit: 32,
    publicTickerSymbolsPerVenueLimit: 32,
    privateWsSymbolsFromWatchlist: 0,
  };
}

function durableWatchlistEnvelope() {
  return {
    items: [
      {
        id: 41,
        symbol: "ETH-USDT",
        venueLong: "binance",
        venueShort: "okx",
        minNetYield: 0.12,
        minVolume24h: 1000,
        enabled: true,
        createdAtMs: 1_769_999_000_000,
        source: "user_api",
        createdBy: "api-token:operator",
        updatedAtMs: 1_769_999_000_000,
        version: 4,
        persistStatus: "persisted",
        runtime: {
          status: "fresh",
          requestedPublicLegs: 2,
          plannedOrderbookLegs: 2,
          plannedTickerLegs: 2,
          deduplicatedLegs: 0,
          cappedLegs: 0,
          lastPrewarmAtMs: 1_770_000_000_000,
        },
      },
    ],
    runtime: durableRuntime(),
  };
}

function durableAlertRulesEnvelope() {
  return {
    rules: [
      {
        id: 51,
        watchlistId: 41,
        channel: { kind: "toast" },
        cooldownSecs: 300,
        enabled: true,
        createdAtMs: 1_769_999_000_000,
        source: "user_api",
        createdBy: "api-token:operator",
        updatedAtMs: 1_769_999_000_000,
        version: 2,
        persistStatus: "persisted",
        deliveryKind: "toast",
        lastFiredAtMs: 1_770_000_000_000,
        lastDeliveryStatus: "queued",
        runtime: {
          status: "cooldown",
          transport: "app_websocket_toast",
          deliverySupported: true,
          watchlistPublicPrewarmLegs: 2,
          privateWsSymbols: 0,
          lastEvaluatedAtMs: 1_770_000_000_500,
          lastTriggeredAtMs: 1_770_000_000_000,
          nextEligibleAtMs: 1_770_000_300_000,
          triggerCount: 3,
          lastOpportunityId: "opp-eth",
        },
      },
    ],
    runtime: durableRuntime(),
  };
}
