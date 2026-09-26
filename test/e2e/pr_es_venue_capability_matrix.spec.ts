import { expect, test, type Page } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";
const WEB_BASE = process.env.CROSSLINE_E2E_WEB_BASE ?? "http://127.0.0.1:18080";
const SCENARIO_BASE = `${API_BASE}/e2e-pr-es-venue-capability-matrix`;

const VENUES = [
  "binance",
  "okx",
  "bybit",
  "bitget",
  "gate",
  "kucoin",
  "hyperliquid",
];

function order(requestedOrderType, effectiveOrderType, timeInForce, venueOrderKind,
  payloadPricePolicy, marketOrderStyles = []) {
  return {
    requestedOrderType,
    effectiveOrderType,
    timeInForce,
    marketOrderStyles,
    venueOrderKind,
    payloadPricePolicy,
  };
}

function matrix(venue) {
  const limitTif = venue === "gate"
    ? ["ioc", "fok", "gtc"]
    : venue === "hyperliquid"
      ? ["ioc", "gtc"]
      : ["ioc", "fok", "gtc", "gtx"];
  let market = order("market", "market", ["ioc"], "native_market", "omit");
  if (venue === "gate") {
    market = order("market", "market", ["ioc"], "price_zero_ioc", "zero_price");
  } else if (venue === "hyperliquid") {
    market = order("market", "limit", ["ioc"], "protected_ioc", "protection_price");
  }
  return {
    venue,
    product: "perp",
    orders: [
      order("limit", "limit", limitTif, "limit", "limit_price"),
      market,
      order("post_only", "post_only", ["gtc"], "post_only", "limit_price"),
    ],
    account: {
      orderMarginModes: ["cross", "isolated"],
      accountModeScope: venue === "kucoin" ? "classic_futures" : `${venue}_perp`,
      runtimeAccountModeRead: venue !== "hyperliquid",
    },
    clientOrderId: {
      venueField: venue === "hyperliquid" ? "c/cloid" : "clientOrderId",
      policyVersion: "client-order-id-policy-v1",
      officialFormat: "official venue format",
      maxLength: 32,
      supportsQueryByClientId: true,
      supportsCancelByClientId: true,
    },
    instrument: {
      nativeSymbolRequired: true,
      instrumentSpecRequired: true,
      nativeSizingRequired: true,
    },
    finality: {
      ackIsFinal: false,
      privateOrderStream: true,
      privateFillStream: true,
      orderStatusRead: true,
      evidenceSources: [`https://example.test/${venue}/orders`],
    },
    source: "exchange.venue_capability_matrix",
    officialDocUrls: [`https://example.test/${venue}/place-order`],
  };
}

function capabilities(marketOrders) {
  return {
    spot: false,
    perp: true,
    limitOrders: true,
    marketOrders,
    postOnly: true,
    reduceOnly: true,
  };
}

function adaptersResponse() {
  return {
    current: "live_router",
    currentEnvironment: "live",
    options: [
      {
        id: "mock",
        label: "Paper",
        environment: "paper",
        enabled: true,
        credentialsAvailable: false,
        capabilities: capabilities(true),
        disabledReason: null,
      },
      {
        id: "live_router",
        label: "Live",
        environment: "live",
        enabled: true,
        credentialsAvailable: true,
        capabilities: capabilities(true),
        disabledReason: null,
      },
    ],
    venues: VENUES.map((venue) => ({
      venue,
      environment: "live",
      credentialsAvailable: venue === "okx",
      capabilities: capabilities(true),
      matrix: matrix(venue),
      source: "exchange.static_venue_capability_matrix",
    })),
  };
}

async function useCapabilityFixture(page: Page) {
  await page.addInitScript((apiBase) => {
    window.localStorage.setItem("api_base", JSON.stringify(apiBase));
    window.localStorage.setItem("api_auth_token", JSON.stringify("e2e-token"));
  }, SCENARIO_BASE);
  await page.route("**/e2e-pr-es-venue-capability-matrix/api/trading/adapters", async (route) => {
    const headers = {
      "access-control-allow-origin": WEB_BASE,
      "access-control-allow-methods": "GET,OPTIONS",
      "access-control-allow-headers": "content-type,authorization,accept,x-request-id",
      "content-type": "application/json; charset=utf-8",
      vary: "origin",
    };
    if (route.request().method() === "OPTIONS") {
      await route.fulfill({ status: 204, headers, body: "" });
      return;
    }
    await route.fulfill({ status: 200, headers, body: JSON.stringify(adaptersResponse()) });
  });
}

test("PR-ES Settings renders all venue compiler contracts without credential filtering", async ({
  page,
}) => {
  await useCapabilityFixture(page);
  const response = page.waitForResponse((candidate) =>
    candidate.url().includes("/e2e-pr-es-venue-capability-matrix/api/trading/adapters")
    && candidate.status() === 200,
  );
  await page.goto("/#settings");
  await page.getByRole("tab", { name: "诊断" }).click();
  await response;

  const table = page.locator('[data-settings-table="venue-capabilities"]');
  await expect(table).toBeVisible();
  await expect(table.locator("tbody tr")).toHaveCount(7);

  const gate = table.locator("tbody tr").filter({
    has: page.getByText("gate", { exact: true }),
  });
  await expect(gate).toContainText("Price-zero IOC 市价");
  await expect(gate).not.toContainText("GTX");

  const hyperliquid = table.locator("tbody tr").filter({
    has: page.getByText("hyperliquid", { exact: true }),
  });
  await expect(hyperliquid).toContainText("保护 IOC 市价");
  await expect(hyperliquid).toContainText("Client ID c/cloid");
  await expect(hyperliquid).toContainText("受理确认 非最终结果");

  const okx = table.locator("tbody tr").filter({
    has: page.getByText("okx", { exact: true }),
  });
  await expect(okx).toContainText("凭证字段已填写；仍需票据级运行 gate");
  const binance = table.locator("tbody tr").filter({
    has: page.getByText("binance", { exact: true }),
  });
  await expect(binance).toContainText("静态契约可用；凭证字段待填写");
  await expect(table).not.toContainText("可下单");
  await expect(table).not.toContainText("权限验证完整");
});
