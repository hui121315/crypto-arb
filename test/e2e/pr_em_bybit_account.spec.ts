import { expect, test, type Page } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";
const FIXTURE_TIME_MS = 1_783_913_200_000;

async function useBybitUnifiedScenario(page: Page) {
  await page.addInitScript((apiBase) => {
    window.localStorage.setItem("api_base", JSON.stringify(apiBase));
    window.localStorage.setItem("api_auth_token", JSON.stringify("e2e-token"));
  }, API_BASE);
  await page.route("**/api/exchanges/credentials", async (route) => {
    await route.fulfill({ json: venueCredentials() });
  });
  await page.route("**/api/trading/account-state", async (route) => {
    await route.fulfill({ json: accountState() });
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
      note: "PR-EM Bybit UNIFIED account evidence fixture.",
    }],
    secretStorage: {
      mode: "runtime_only",
      persistent: false,
      encrypted: false,
      atomicWrite: false,
      path: null,
      label: "E2E runtime-only credentials",
      message: "PR-EM browser fixture does not persist credentials.",
      warning: "not a live credential sample",
    },
  };
}

function accountState() {
  const fieldQuality: Record<string, unknown>[] = [
    "equity",
    "availableBalance",
    "initialMargin",
    "maintenanceMargin",
    "initialMarginRate",
    "maintenanceMarginRate",
    "equityScope",
  ].map((field) => ({
    subject: { kind: "account", venue: "bybit" },
    field,
    status: "actual",
    source: "bybit.v5.wallet-balance",
    observedAtMs: FIXTURE_TIME_MS,
  }));
  fieldQuality.push({
    subject: { kind: "account", venue: "bybit" },
    field: "withdrawableBalance",
    status: "missing",
    source: "bybit.v5.wallet-balance",
    observedAtMs: FIXTURE_TIME_MS,
    problem: {
      code: "ACCOUNT_FIELD_UNKNOWN",
      message: "Bybit UNIFIED transferable balance requires a separate endpoint",
      source: "bybit.v5.wallet-balance",
    },
  });
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
    accountBindings: [],
    accountSummaries: [],
  };
  return {
    balances: {
      ...envelope,
      accountSummaries: [{
        venue: "bybit",
        accountType: "UNIFIED",
        equityScope: "unified",
        totalEquityUsd: 10_262.91335023,
        totalAvailableBalanceUsd: 9_556.6056555,
        withdrawableBalanceUsd: null,
        totalInitialMarginUsd: 127.85731614,
        totalMaintenanceMarginUsd: 54.32846287,
        accountImRate: 0.021,
        accountMmRate: 0.009,
        source: "bybit.v5.wallet-balance",
        observedAtMs: FIXTURE_TIME_MS,
        freshnessMs: 500,
        problem: null,
      }],
    },
    positions: envelope,
    openOrders: envelope,
    status: "degraded",
    source: "account_state_runtime",
    observedAtMs: FIXTURE_TIME_MS,
    problems: [],
    operationHealth: [],
    fieldQuality,
  };
}

test("PR-EM renders Bybit UNIFIED equity available and margin evidence", async ({ page }) => {
  await useBybitUnifiedScenario(page);
  const accountStateResponse = page.waitForResponse((response) =>
    response.url().includes("/api/trading/account-state") && response.status() === 200
  );

  await page.goto("/#settings");
  await accountStateResponse;
  await expect(page.locator("h1", { hasText: "设置" })).toBeVisible();
  await expect(page.getByLabel("交易所")).toHaveValue("bybit");

  const panel = page.locator(".runtime-health-panel").filter({ hasText: "账户字段证据" });
  await expect(panel).toContainText("bybit · 账户事实 1 · 字段 8");
  await expect(panel).toContainText("bybit UNIFIED");
  await expect(panel).toContainText("Equity $10262.91");
  await expect(panel).toContainText("Available $9556.61 · Withdrawable 未知 · IM $127.86 · MM $54.33");
  await expect(panel).toContainText("IM 2.1000% · MM 0.9000%");
  await expect(panel).toContainText("bybit.v5.wallet-balance");
  await expect(panel).toContainText("统一账户权益");
  await expect(panel).toContainText("withdrawableBalance");
  await expect(panel).toContainText("ACCOUNT_FIELD_UNKNOWN");
  await expect(panel).not.toContainText("当前交易所没有账户级 equity / margin 事实");
  await expect(panel).not.toContainText("无效");
});
