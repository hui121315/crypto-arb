import { expect, test, type Page } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";
const FIXTURE_TIME_MS = 1_784_000_000_000;

async function useAccountStateScenario(page: Page) {
  await page.addInitScript((apiBase) => {
    window.localStorage.setItem("api_base", JSON.stringify(apiBase));
    window.localStorage.setItem("api_auth_token", JSON.stringify("e2e-token"));
  }, API_BASE);
  await page.routeWebSocket("**/ws", (socket) => socket.close());
  await page.route("**/api/exchanges/credentials", async (route) => {
    await route.fulfill({ json: venueCredentials() });
  });
  await page.route("**/api/trading/account-state", async (route) => {
    await route.fulfill({ json: accountState() });
  });
  await page.route("**/api/trading/portfolio/snapshot", async (route) => {
    const upstream = await route.fetch();
    const envelope = await upstream.json();
    const state = accountState();
    envelope.snapshot.summary = portfolioSummary();
    envelope.snapshot.balances = state.balances.rows;
    envelope.snapshot.accountState = state;
    envelope.snapshot.degraded = true;
    await route.fulfill({ json: envelope });
  });
}

function venueCredentials() {
  return {
    venues: [{
      venue: "bybit",
      label: "Bybit",
      fields: [{
        key: "api_key",
        label: "API Key",
        envKey: "BYBIT_API_KEY",
        configured: true,
        secret: false,
      }],
      publicMarket: true,
      privateRead: true,
      testnetWrite: false,
      liveWrite: false,
      note: "PR-ED unified account scope fixture.",
    }],
    secretStorage: {
      mode: "runtime_only",
      persistent: false,
      encrypted: false,
      atomicWrite: false,
      path: null,
      label: "E2E runtime-only credentials",
      message: "PR-ED browser fixture does not persist credentials.",
      warning: "not a live credential sample",
    },
  };
}

function portfolioSummary() {
  return {
    totalNavUsd: 12_500,
    navEvidence: {
      status: "actual",
      source: "account_state.account_summaries.total_equity_usd",
      observedAtMs: FIXTURE_TIME_MS,
      coveredVenues: ["bybit"],
      missingVenues: [],
      problem: null,
    },
    navChange24hPct: 2.04,
    netDeltaUsd: 125,
    netDeltaPctOfNav: 1,
    nakedExposureUsd: 0,
    nakedPositionCount: 0,
    realizedPnlTodayUsd: 42,
    pnlBreakdown: { fundingUsd: 5, priceUsd: 37, feeRebateUsd: 0 },
    updatedAtMs: FIXTURE_TIME_MS,
  };
}

function accountState() {
  const withdrawableProblem = {
    code: "ACCOUNT_FIELD_UNKNOWN",
    message: "Bybit UNIFIED transferable balance requires a separate verified endpoint",
    status: 200,
    source: "bybit.GET /v5/account/wallet-balance",
    details: { venue: "bybit", field: "withdrawableBalance" },
  };
  const binding = {
    venue: "bybit",
    accountScope: "unified",
    status: "verified",
    source: "credential_probe:account_mode_read",
    checkedAtMs: FIXTURE_TIME_MS,
    freshnessMs: 250,
    credentialFingerprint: "fixture-fingerprint",
    problem: null,
  };
  const fieldQuality: Record<string, unknown>[] = [
    "equity",
    "availableBalance",
    "initialMargin",
    "maintenanceMargin",
    "initialMarginRate",
    "maintenanceMarginRate",
    "equityScope",
  ].map((field) => ({
    subject: { kind: "account", venue: "bybit", accountScope: "unified" },
    field,
    status: "actual",
    source: "bybit.GET /v5/account/wallet-balance",
    observedAtMs: FIXTURE_TIME_MS,
  }));
  fieldQuality.push({
    subject: { kind: "account", venue: "bybit", accountScope: "unified" },
    field: "withdrawableBalance",
    status: "missing",
    source: "bybit.GET /v5/account/wallet-balance",
    observedAtMs: FIXTURE_TIME_MS,
    problem: withdrawableProblem,
  });
  fieldQuality.push({
    subject: {
      kind: "open_order",
      venue: "bybit",
      accountScope: "unified",
      orderId: "order-ed-1",
      symbol: "BTCUSDT",
      side: "buy",
    },
    field: "quantity",
    status: "actual",
    source: "account_open_orders_runtime",
    observedAtMs: FIXTURE_TIME_MS,
  });
  for (const field of ["clientOrderId", "venueTimeInForce", "reduceOnly"]) {
    fieldQuality.push({
      subject: {
        kind: "open_order",
        venue: "bybit",
        accountScope: "unified",
        orderId: "order-ed-1",
        symbol: "BTCUSDT",
        side: "buy",
      },
      field,
      status: "actual",
      source: "account_open_orders_runtime",
      observedAtMs: FIXTURE_TIME_MS,
    });
  }
  const envelope = {
    rows: [],
    rowCount: 0,
    status: "fresh",
    source: "account_runtime",
    observedAtMs: FIXTURE_TIME_MS,
    problems: [],
    operationHealth: [],
    fieldQuality: [],
    rowHealth: [],
    accountBindings: [binding],
  };
  return {
    balances: {
      ...envelope,
      rows: [{
        venue: "bybit",
        currency: "USDT",
        total: 12_500,
        available: 10_000,
        frozen: 2_500,
        unrealizedPnl: 0,
      }],
      rowCount: 1,
      accountSummaries: [{
        venue: "bybit",
        accountType: "UNIFIED",
        equityScope: "unified",
        totalEquityUsd: 12_500,
        totalAvailableBalanceUsd: 10_000,
        withdrawableBalanceUsd: null,
        totalInitialMarginUsd: 2_500,
        totalMaintenanceMarginUsd: 500,
        accountImRate: 0.2,
        accountMmRate: 0.04,
        source: "bybit.GET /v5/account/wallet-balance",
        observedAtMs: FIXTURE_TIME_MS,
        freshnessMs: 250,
        problem: null,
      }],
    },
    positions: envelope,
    openOrders: {
      ...envelope,
      rows: [{
        orderId: "order-ed-1",
        symbol: "BTCUSDT",
        exchange: "bybit",
        side: "buy",
        orderType: "limit",
        venueTimeInForce: "GTC",
        clientOrderId: "bybit-client-ed-1",
        reduceOnly: false,
        status: "open",
        quantity: 0.1,
        price: 60_000,
        filledQuantity: 0,
        filledPrice: 0,
        fees: 0,
        createdAt: "2026-07-14T00:00:00Z",
      }],
      rowCount: 1,
    },
    status: "degraded",
    source: "account_state_runtime",
    observedAtMs: FIXTURE_TIME_MS,
    problems: [withdrawableProblem],
    operationHealth: [],
    fieldQuality,
    accountBindings: [binding],
  };
}

test("PR-ED account equity NAV open orders and scoped evidence stay unified", async ({ page }) => {
  await useAccountStateScenario(page);
  const portfolioResponse = page.waitForResponse((response) =>
    response.url().includes("/api/trading/portfolio/snapshot") && response.status() === 200
  );

  await page.goto("/#positions");
  const portfolio = await (await portfolioResponse).json();
  expect(portfolio.snapshot.summary.navEvidence).toMatchObject({
    status: "actual",
    source: "account_state.account_summaries.total_equity_usd",
    coveredVenues: ["bybit"],
    missingVenues: [],
  });
  expect(portfolio.snapshot.accountState.openOrders.rows).toHaveLength(1);
  expect(portfolio.snapshot.accountState.accountBindings[0].accountScope).toBe("unified");
  await expect(page.locator(".summary-card").filter({ hasText: "账户净值" })).toContainText("$12.5K");

  const accountStateResponse = page.waitForResponse((response) =>
    response.url().includes("/api/trading/account-state") && response.status() === 200
  );
  await page.goto("/#settings");
  await accountStateResponse;
  await page.getByLabel("交易所").selectOption("bybit");
  const panel = page.locator(".runtime-health-panel").filter({ hasText: "账户字段证据" });
  await expect(panel).toContainText("统一账户权益");
  await expect(panel).toContainText("Withdrawable 未知");
  await expect(panel).toContainText("withdrawableBalance");
  await expect(panel).toContainText("order-ed-1 · unified");
  await expect(panel).toContainText("已验证");
});

test("PR-AQ private-read order semantics stay visible in account evidence", async ({ page }) => {
  await useAccountStateScenario(page);
  const accountStateResponse = page.waitForResponse((response) =>
    response.url().includes("/api/trading/account-state") && response.status() === 200
  );

  await page.goto("/#settings");
  const snapshot = await (await accountStateResponse).json();
  await page.getByLabel("交易所").selectOption("bybit");

  expect(snapshot.openOrders.rows[0]).toMatchObject({
    venueTimeInForce: "GTC",
    clientOrderId: "bybit-client-ed-1",
    reduceOnly: false,
  });
  const panel = page.locator(".runtime-health-panel").filter({ hasText: "账户字段证据" });
  await expect(panel).toContainText("clientOrderId");
  await expect(panel).toContainText("venueTimeInForce");
  await expect(panel).toContainText("reduceOnly");
  await expect(panel).toContainText("account_open_orders_runtime");
  await expect(panel).toContainText("实际");
});
