import { expect, test, type Page } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";
const FIXTURE_TIME_MS = 1_773_000_000_000;

async function useApiBase(page: Page) {
  await page.addInitScript((apiBase) => {
    window.localStorage.setItem("api_base", JSON.stringify(apiBase));
    window.localStorage.setItem("api_auth_token", JSON.stringify("e2e-token"));
  }, API_BASE);
}

async function useAccountEvidenceScenario(page: Page) {
  await useApiBase(page);
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
      venue: "okx",
      label: "OKX",
      fields: [
        {
          key: "api_key",
          label: "API Key",
          envKey: "OKX_API_KEY",
          configured: true,
          secret: false,
        },
      ],
      publicMarket: true,
      privateRead: true,
      testnetWrite: false,
      liveWrite: false,
      note: "PR-FC account evidence browser fixture.",
    }],
    secretStorage: {
      mode: "runtime_only",
      persistent: false,
      encrypted: false,
      atomicWrite: false,
      path: null,
      label: "E2E runtime-only credentials",
      message: "PR-FC browser fixture does not persist credentials.",
      warning: "not a live credential sample",
    },
  };
}

function accountState() {
  const balanceProblem = {
    code: "BALANCE_FIELD_UNAVAILABLE",
    message: "available balance was invalid in the private account payload",
    status: 200,
    source: "account_balance_runtime",
    requestId: "req-fc-balance",
    retryAfterMs: 4_000,
    details: { venue: "okx", field: "available" },
  };
  const envelope = {
    rows: [],
    rowCount: 0,
    status: "degraded",
    source: "account_balance_runtime",
    observedAtMs: FIXTURE_TIME_MS,
    problems: [],
    operationHealth: [],
    fieldQuality: [],
    rowHealth: [],
    accountBindings: [],
  };
  return {
    balances: {
      ...envelope,
      fieldQuality: [{
        subject: { kind: "balance", venue: "okx", currency: "USDT" },
        field: "available",
        status: "invalid",
        source: "account_balance_runtime",
        observedAtMs: FIXTURE_TIME_MS,
        problem: balanceProblem,
      }],
      rowHealth: [{
        subject: { kind: "balance", venue: "okx", currency: "USDT" },
        source: "account_balance_runtime",
        observedAtMs: FIXTURE_TIME_MS,
        freshnessMs: 800,
        lastSuccessMs: FIXTURE_TIME_MS - 800,
        lastError: balanceProblem,
        retryAfterMs: 4_000,
        requestId: "req-fc-balance",
      }],
      accountBindings: [{
        venue: "okx",
        accountScope: "cross_margin",
        status: "verified",
        source: "credential_probe",
        checkedAtMs: FIXTURE_TIME_MS,
        freshnessMs: 800,
        credentialFingerprint: "fixture-fingerprint",
      }],
    },
    positions: envelope,
    openOrders: envelope,
    status: "degraded",
    source: "account_state_runtime",
    observedAtMs: FIXTURE_TIME_MS,
    problems: [balanceProblem],
    operationHealth: [],
    fieldQuality: [],
  };
}

test("PR-FC Settings exposes selected-venue account field quality and scope evidence", async ({ page }) => {
  await useAccountEvidenceScenario(page);
  const accountStateResponse = page.waitForResponse((response) =>
    response.url().includes("/api/trading/account-state") && response.status() === 200,
  );

  await page.goto("/#settings");
  await accountStateResponse;
  await expect(page.locator("h1", { hasText: "设置" })).toBeVisible();
  await expect(page.getByLabel("交易所")).toHaveValue("okx");

  const panel = page.locator(".runtime-health-panel").filter({ hasText: "账户字段证据" });
  await expect(panel).toContainText("okx USDT");
  await expect(panel).toContainText("available");
  await expect(panel).toContainText("无效");
  await expect(panel).toContainText("BALANCE_FIELD_UNAVAILABLE");
  await expect(panel).toContainText("request_id req-fc-balance");
  await expect(panel).toContainText("cross_margin");
  await expect(panel).toContainText("已验证");
  await expect(panel).not.toContainText("暂无账户范围绑定证据");
});
