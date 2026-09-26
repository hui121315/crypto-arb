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

function marginOutcome() {
  return {
    status: "passed",
    checkedAtMs: 1_780_000_000_000,
    scope: {
      venues: ["bybit", "okx"],
      symbols: ["BTCUSDT", "BTC-USDT-SWAP"],
      accountModes: ["cross"],
      operations: ["margin_balance"],
    },
    observedVenues: ["bybit", "okx"],
    balanceRows: [
      { venue: "bybit", currency: "USD", total: 100, available: 80, frozen: 20 },
      { venue: "okx", currency: "USD", total: 120, available: 90, frozen: 30 },
    ],
    source: "account_state.margin_facts",
    freshnessMs: 420,
    requestId: "req-pr-ap-margin",
    problems: [],
    fieldQuality: [
      {
        subject: { kind: "balance", venue: "bybit", currency: "USD" },
        field: "margin_currency",
        status: "actual",
        source: "bybit.v5.wallet_balance:UNIFIED",
      },
      {
        subject: { kind: "balance", venue: "bybit", currency: "USDT" },
        field: "collateral_currency",
        status: "actual",
        source: "bybit.v5.wallet_balance:coin",
      },
      {
        subject: { kind: "account", venue: "okx" },
        field: "account_equity_source",
        status: "actual",
        source: "okx.v5.account_balance:multi_currency_margin",
      },
    ],
    rowHealth: [
      {
        subject: { kind: "balance", venue: "bybit", currency: "USDT" },
        source: "bybit.v5.wallet_balance:coin",
        observedAtMs: 1_780_000_000_000,
        freshnessMs: 420,
        lastSuccessMs: 1_780_000_000_000,
        requestId: "req-pr-ap-bybit-balance",
      },
      {
        subject: { kind: "account", venue: "okx" },
        source: "okx.v5.account_balance:multi_currency_margin",
        observedAtMs: 1_780_000_000_000,
        freshnessMs: 380,
        lastSuccessMs: 1_780_000_000_000,
        requestId: "req-pr-ap-okx-account",
      },
    ],
  };
}

async function injectAccountEvidence(page: Page) {
  await page.route("**/api/arbitrage/opportunities/mock-mu-perp/preview", async (route) => {
    const upstream = await route.fetch();
    const response = await upstream.json();
    response.ticket.guards.push(
      {
        key: "margin_balance",
        label: "保证金余额",
        passed: true,
        detail: "通过",
        preflightOutcome: marginOutcome(),
      },
      {
        key: "account_mode",
        label: "账户模式数据依据",
        passed: true,
        detail: "通过",
        preflightOutcome: {
          ...marginOutcome(),
          scope: {
            venues: ["bybit", "okx"],
            symbols: ["BTCUSDT", "BTC-USDT-SWAP"],
            accountModes: [
              "bybit_position_mode:hedge·UNIFIED",
              "okx_position_mode:long_short_mode·multi_currency_margin",
            ],
            operations: ["account_mode"],
          },
          balanceRows: [],
          fieldQuality: [],
          rowHealth: [],
          source: "live_trading_adapter.get_exchange_account_mode",
        },
      },
    );
    await route.fulfill({ response: upstream, json: response });
  });
}

test("PR-AP execution exposes scoped margin currency, account source, and mode evidence", async ({
  page,
}) => {
  await useExecutionScenario(page);
  await injectAccountEvidence(page);

  await page.goto("/#futures");
  await expect(page.getByRole("heading", { name: "期货套利" })).toBeVisible();
  await page.getByRole("button", { name: "构建对冲" }).click();
  await expect(page.getByRole("heading", { name: "对冲执行" })).toBeVisible();

  const margin = page.locator(".check-item").filter({ hasText: "保证金余额" });
  await expect(margin).toContainText("bybit USD 可用 $80");
  await expect(margin).toContainText("bybit USDT bybit.v5.wallet_balance:coin");
  await expect(margin).toContainText("account_equity_source OK");
  await expect(margin).toContainText("okx.v5.account_balance:multi_currency_margin");
  await expect(margin).toContainText("请求 req-pr-a");

  const accountMode = page.locator(".check-item").filter({ hasText: "账户模式数据依据" });
  await expect(accountMode).toContainText("bybit_position_mode:hedge·UNIFIED");
  await expect(accountMode).toContainText(
    "okx_position_mode:long_short_mode·multi_currency_margin",
  );
});
