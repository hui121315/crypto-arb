import { expect, test, type Locator, type Page } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";
const SCENARIO = "e2e-pr-eo-kucoin-native-ready";

type CompilePlan = {
  role: "long" | "short";
  exchange: string;
  symbol: string;
  instrumentSpec?: Record<string, unknown>;
  sizingPlan?: Record<string, number>;
};

function attachInstrumentSizing(plan: CompilePlan, referencePrice: number) {
  const targetNotionalUsd = 750;
  const contractSize = 0.001;
  const qtyStep = 1;
  const rawBaseQty = targetNotionalUsd / referencePrice;
  const rawContracts = rawBaseQty / contractSize;
  const roundedContracts = Math.floor(rawContracts / qtyStep) * qtyStep;
  const roundedBaseQty = roundedContracts * contractSize;
  const actualNotionalUsd = roundedBaseQty * referencePrice;
  const roundingDeltaUsd = Math.max(0, targetNotionalUsd - actualNotionalUsd);

  plan.instrumentSpec = {
    venue: plan.exchange,
    nativeSymbol: "XBTUSDCM",
    canonicalSymbol: plan.symbol,
    displaySymbol: "BTC-USDC Perp",
    assetClass: "crypto",
    productType: "perp",
    quoteAsset: "USDC",
    settleAsset: "USDC",
    marginAsset: "USDC",
    contractSize,
    executionSupported: true,
    priceTick: 0.1,
    qtyStep,
    minQty: 1,
    listingStatus: "trading",
    fundingIntervalMs: 28_800_000,
    source: "official_endpoint",
    sourceUrl:
      "https://www.kucoin.com/docs-new/rest/futures-trading/market-data/get-all-symbols",
    checkedAtMs: 1_770_000_000_000,
    schemaVersion: "kucoin-futures-symbols-v1",
  };
  plan.sizingPlan = {
    targetNotionalUsd,
    referencePrice,
    priceTick: 0.1,
    contractSize,
    qtyStep,
    minQty: 1,
    minNotional: 0,
    rawContracts,
    rawBaseQty,
    roundedContracts,
    roundedBaseQty,
    actualNotionalUsd,
    roundingDeltaUsd,
    roundingLossBps: roundingDeltaUsd / targetNotionalUsd * 10_000,
  };
}

async function openInstrumentPlan(page: Page, complete: boolean): Promise<Locator> {
  await page.addInitScript(
    ({ apiBase, scenario }) => {
      window.localStorage.setItem("api_base", JSON.stringify(`${apiBase}/${scenario}`));
      window.localStorage.setItem("api_auth_token", JSON.stringify("e2e-token"));
    },
    { apiBase: API_BASE, scenario: SCENARIO },
  );
  await page.routeWebSocket("**/ws", (socket) => socket.close());
  await page.route(
    `**/${SCENARIO}/api/arbitrage/opportunities/mock-mu-perp/preview`,
    async (route) => {
      const upstream = await route.fetch();
      const response = await upstream.json();
      response.longLeg.mode = "live";
      response.shortLeg.mode = "live";
      if (complete) {
        attachInstrumentSizing(response.ticketOrderPlans.long.compilePlan, response.longLeg.price);
        attachInstrumentSizing(response.ticketOrderPlans.short.compilePlan, response.shortLeg.price);
      }
      await route.fulfill({ json: response });
    },
  );

  await page.goto("/#futures");
  await expect(page.getByRole("heading", { name: "期货套利" })).toBeVisible();
  const preview = page.waitForResponse((response) =>
    response.url().includes(`/${SCENARIO}/api/arbitrage/opportunities/mock-mu-perp/preview`)
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

test("PR-EB renders ticket-bound native symbol, sizing and rounding evidence", async ({ page }) => {
  const plan = await openInstrumentPlan(page, true);

  await expect(plan).toContainText("BTC-USDC > XBTUSDCM");
  await expect(plan).toContainText("提交 XBTUSDCM：数量 1.128 / 1128 张");
  await expect(plan).toHaveAttribute("title", /合约乘数 0.001/);
  await expect(plan).toHaveAttribute("title", /价格刻度 0.1 · 数量步长 1 · 最小数量 1/);
  await expect(plan).toHaveAttribute("title", /提交数量 1.128 \/ 1128 张/);
  await expect(plan).toHaveAttribute("title", /实际 \$749.70264 · 差额 \$0.29736/);
  await expect(plan).toHaveAttribute("title", /舍入损耗 3.9648 基点/);
  await expect(plan).toHaveAttribute("title", /schema kucoin-futures-symbols-v1 · 合同 VALID/);
  await expect(page.locator(".confirm-action.primary")).toBeEnabled();
});

test("PR-EB live preview disables submit when instrument sizing evidence is absent", async ({
  page,
}) => {
  const plan = await openInstrumentPlan(page, false);

  await expect(plan).toContainText("BTC-USDC > XBTUSDCM");
  await expect(plan).toContainText("2 条阻断");
  await expect(plan).toHaveAttribute("title", /instrument\/sizing MISSING/);
  await expect(page.locator(".confirm-action.primary")).toBeDisabled();
});
