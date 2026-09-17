import { expect, test, type Locator, type Page } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";

async function openIdentityPlan(page: Page, scenario: string): Promise<Locator> {
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

test("PR-EL renders Binance USDC canonical and native identity with client-id policy", async ({
  page,
}) => {
  const plan = await openIdentityPlan(page, "e2e-pr-el-binance-identity-ready");

  await expect(plan).toContainText("BTCUSDC > BTCUSDC");
  await expect(plan).toHaveAttribute("title", /canonical BTCUSDC · native BTCUSDC/);
  await expect(plan).toHaveAttribute("title", /settle USDC · quote USDC · product perp/);
  await expect(plan).toHaveAttribute("title", /public client id public-long-order/);
  await expect(plan).toHaveAttribute("title", /venue client id venue-long-order/);
  await expect(plan).toHaveAttribute(
    "title",
    /finality private_user_stream_with_rest_fallback/,
  );
  await expect(page.locator(".confirm-action.primary")).toBeEnabled();
});

test("PR-EL blocks submit when the shared identity evidence contract is missing", async ({
  page,
}) => {
  const plan = await openIdentityPlan(page, "e2e-pr-el-binance-identity-missing");

  await expect(plan).toContainText("BTCUSDC > 缺原生标识");
  await expect(plan).toContainText("2 条阻断");
  await expect(plan).toHaveAttribute("title", /ORDER_IDENTITY_CANONICAL_SYMBOL_MISSING/);
  await expect(plan).toHaveAttribute("title", /ORDER_IDENTITY_METADATA_EVIDENCE_UNAVAILABLE/);
  await expect(page.locator(".confirm-action.primary")).toBeDisabled();
});

test("PR-EL exposes canonical mismatch and unavailable live runtime evidence fail closed", async ({
  page,
}) => {
  const plan = await openIdentityPlan(page, "e2e-pr-el-binance-runtime-unavailable");

  await expect(plan).toContainText("BTCUSDC > BTCUSDC");
  await expect(plan).toHaveAttribute("title", /canonical BTCUSDT · native BTCUSDC/);
  await expect(plan).toHaveAttribute("title", /ORDER_IDENTITY_CANONICAL_SYMBOL_MISMATCH/);
  await expect(plan).toHaveAttribute("title", /user-stream=UNAVAILABLE/);
  await expect(plan).toHaveAttribute("title", /finality=UNAVAILABLE/);
  await expect(plan).toHaveAttribute("title", /fee=UNAVAILABLE/);
  await expect(plan).toHaveAttribute("title", /unavailable without live credential capture/);
  await expect(page.locator(".confirm-action.primary")).toBeDisabled();
});
