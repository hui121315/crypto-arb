import { expect, test, type Locator, type Page } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";

async function useKucoinProWsRuntimeGateScenario(page: Page) {
  await page.addInitScript((apiBase) => {
    window.localStorage.setItem("api_base", JSON.stringify(apiBase));
    window.localStorage.setItem("api_auth_token", JSON.stringify("e2e-token"));
  }, API_BASE);

  await page.route("**/api/exchanges/credentials", async (route) => {
    await route.fulfill({ json: kucoinCredentials() });
  });
  await page.route("**/api/trading/ws/venues", async (route) => {
    await route.fulfill({ json: kucoinWsVenues() });
  });
}

async function openKucoinIdentityPlan(page: Page, scenario: string): Promise<Locator> {
  await page.addInitScript(
    ({ apiBase, scenarioName }) => {
      window.localStorage.setItem("api_base", JSON.stringify(`${apiBase}/${scenarioName}`));
      window.localStorage.setItem("api_auth_token", JSON.stringify("e2e-token"));
    },
    { apiBase: API_BASE, scenarioName: scenario },
  );
  await page.goto("/#futures");
  await expect(page.getByRole("heading", { name: "期货套利" })).toBeVisible();
  const preview = page.waitForResponse((response) =>
    response.url().includes(`/${scenario}/api/arbitrage/opportunities/mock-mu-perp/preview`)
    && response.status() === 200,
  );
  await page.getByRole("button", { name: "构建对冲" }).click();
  await preview;
  await page.locator(".execution-evidence-details summary").click();
  const row = page.locator(".execution-risk-section .risk-notes > div").filter({
    hasText: "订单编译",
  });
  await expect(row).toBeVisible();
  return row.locator("strong");
}

test("PR-EO renders verified KuCoin native multiplier identity and Classic finality", async ({
  page,
}) => {
  const plan = await openKucoinIdentityPlan(page, "e2e-pr-eo-kucoin-native-ready");

  await expect(plan).toContainText("BTC-USDC > XBTUSDCM");
  await expect(plan).toHaveAttribute("title", /canonical BTC-USDC · native XBTUSDCM/);
  await expect(plan).toHaveAttribute("title", /settle USDC · quote USDC · product perp/);
  await expect(plan).toHaveAttribute(
    "title",
    /finality private_user_stream_with_rest_fallback/,
  );
  await expect(plan).toHaveAttribute("title", /KuCoin native USDC multiplier sizing plan/);
  await expect(plan).toHaveAttribute("title", /source=kucoin-futures-capture-registry/);
  await expect(page.locator(".confirm-action.primary")).toBeEnabled();
});

test("PR-EO exposes missing KuCoin live credential evidence and blocks submit", async ({
  page,
}) => {
  const plan = await openKucoinIdentityPlan(page, "e2e-pr-eo-kucoin-runtime-unavailable");

  await expect(plan).toContainText("BTC-USDC > XBTUSDCM");
  await expect(plan).toHaveAttribute("title", /user-stream=UNAVAILABLE/);
  await expect(plan).toHaveAttribute("title", /finality=UNAVAILABLE/);
  await expect(plan).toHaveAttribute("title", /fee=UNAVAILABLE/);
  await expect(plan).toHaveAttribute("title", /unavailable without live credential capture/);
  await expect(page.locator(".confirm-action.primary")).toBeDisabled();
});

test("PR-EO KuCoin Pro WS production schema remains runtime-gated", async ({ page }) => {
  await useKucoinProWsRuntimeGateScenario(page);
  await page.goto("/#settings");
  await expect(page.locator("h1", { hasText: "设置" })).toBeVisible();
  await page.getByLabel("交易所").selectOption("kucoin");

  const panel = page.locator(".ws-venue-panel");
  await expect(panel).toContainText("官方生产 schema 已发布");
  await expect(panel).toContainText("REST 单次提交");
  await expect(panel).toContainText("禁止 WS 失败后 REST 重放");

  for (const label of ["下单", "撤单"]) {
    const capability = panel.locator(".ws-cap").filter({ hasText: label });
    await expect(capability).toContainText("缺认证运行数据依据");
    await expect(capability).toContainText("官方 schema 已发布");
    await expect(capability).toContainText("官方 WS 已发布但等待认证运行数据依据");
    await expect(capability).toContainText("authenticated session");
    await expect(capability).toContainText("order finality 运行状态数据依据");
    await expect(capability).not.toContainText("静态实现");
    await expect(capability).not.toContainText("Beta 禁止生产");
  }
});

function kucoinCredentials() {
  return {
    venues: [{
      venue: "kucoin",
      label: "KuCoin",
      fields: [
        { key: "api_key", label: "API Key", envKey: "KUCOIN_API_KEY", configured: false, secret: true },
        { key: "api_secret", label: "API Secret", envKey: "KUCOIN_API_SECRET", configured: false, secret: true },
        { key: "passphrase", label: "Passphrase", envKey: "KUCOIN_PASSPHRASE", configured: false, secret: true },
      ],
      publicMarket: true,
      privateRead: true,
      testnetWrite: false,
      liveWrite: true,
      note: "REST single-submit until authenticated Pro WS runtime evidence is captured",
    }],
    secretStorage: {
      mode: "runtime_only",
      persistent: false,
      encrypted: false,
      atomicWrite: false,
      path: null,
      label: "仅进程内缓存",
      message: "PR-EO browser fixture",
      warning: "no live credentials",
    },
  };
}

function kucoinWsVenues() {
  const runtimeGateNote = "官方生产 schema 已发布；缺 authenticated session、live place/cancel 受理确认 与 order finality 运行状态数据依据；当前 REST 单次提交，禁止 WS 失败后 REST 重放";
  return {
    venues: [{
      venue: "kucoin",
      label: "KuCoin",
      publicEndpoint: "wss://ws-api-futures.kucoin.com",
      privateEndpoint: "classic bullet-private negotiated endpoint",
      tradeEndpoint: "wss://wsapi.kucoin.com/v1/private",
      accountStream: wsOperation("ready", "/contractAccount/wallet", "Futures", "Classic private balance stream"),
      positionStream: wsOperation("ready", "/contract/positionAll", "Futures", "Classic private position stream"),
      fillStream: wsOperation("ready", "/contractMarket/tradeOrders", "Futures", "Classic match fill evidence"),
      orderStream: wsOperation("ready", "/contractMarket/tradeOrders", "Futures", "Classic private order stream"),
      placeOrder: wsOperation("requires_permission", "futures.order", "Futures", runtimeGateNote, true),
      cancelOrder: wsOperation("requires_permission", "futures.cancel", "Futures", runtimeGateNote, true),
      closePosition: wsOperation("requires_permission", "futures.order reduceOnly", "Futures", runtimeGateNote, true),
      orderStatus: wsOperation("ready", "/contractMarket/tradeOrders", "Futures", "Classic private order status"),
      authFields: ["api_key", "api_secret", "passphrase"],
      docs: [
        { label: "pro ws add order", url: "https://www.kucoin.com/docs-new/3470252w0" },
        { label: "pro ws cancel order", url: "https://www.kucoin.com/docs-new/3470253w0" },
      ],
      note: "Classic 私有流继续使用 bullet token；Pro WS 官方生产 schema 已发布，当前 REST 单次提交并等待认证运行数据依据；禁止 WS 失败后 REST 重放。",
    }],
  };
}

function wsOperation(
  status: string,
  operation: string | null,
  product: string,
  note: string,
  runtimeGated = false,
) {
  return {
    supported: true,
    status,
    operation,
    product,
    note,
    evidence: {
      releaseStatus: "production_ready",
      requiresAuthenticatedRuntimeEvidence: runtimeGated,
      authenticatedRuntimeEvidence: false,
      checkedAt: "2026-07-27",
      docVersion: "kucoin-pro-ws-production-2026-03-26",
      docUrl: "https://www.kucoin.com/docs-new/3470252w0",
      parserTest: "parses_order_balance_position_and_pro_ack",
      subscriptionTest: "pro_order_and_cancel_payloads_use_official_ops",
      authKind: "signed_wsapi_query_challenge",
    },
  };
}
