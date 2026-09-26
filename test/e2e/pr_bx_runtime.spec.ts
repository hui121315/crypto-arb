import { expect, test } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";
const SCENARIO_BASE = `${API_BASE}/e2e-pr-bx-runtime`;

async function usePrBxRuntime(page) {
  await page.addInitScript((apiBase) => {
    window.localStorage.setItem("api_base", JSON.stringify(apiBase));
    window.localStorage.setItem("api_auth_token", JSON.stringify("e2e-token"));
  }, SCENARIO_BASE);
}

test.beforeEach(async ({ page }) => {
  await usePrBxRuntime(page);
});

test("PR-BX status bar keeps market, trading, private WS, and app WS sources distinct", async ({
  page,
}) => {
  const operationHealth = page.waitForResponse((response) =>
    response.url().includes("/e2e-pr-bx-runtime/api/system/venue-operation-health")
    && response.status() === 200,
  );

  await page.goto("/#futures");
  await expect(page.getByRole("heading", { name: "期货套利" })).toBeVisible();
  await operationHealth;

  const status = page.getByTestId("top-status-bar");
  const market = status.getByTestId("status-market-data");
  const trading = status.getByTestId("status-api-runtime");
  const privateWs = status.getByTestId("status-private-ws");
  const appWs = status.getByTestId("status-app-ws");

  await expect(market).toContainText("MarketData");
  await expect(market).toContainText("1/1");
  await expect(market).toHaveAttribute("title", /rest_orderbooks/);
  await expect(market).toHaveAttribute("title", /PR-BX market-data source is fresh/);
  await expect(market).not.toHaveAttribute("title", /order_write|private_ws_order_stream/);

  await expect(trading).toContainText("TradingAPI");
  await expect(trading).toContainText("1可用\/2配置");
  await expect(trading).toHaveClass(/degraded/);
  await expect(trading).toHaveAttribute("title", /order_write/);
  await expect(trading).toHaveAttribute("title", /source pr_bx_trading_runtime/);
  await expect(trading).not.toHaveAttribute("title", /rest_orderbooks|private_ws_order_stream/);

  await expect(privateWs).toContainText("PrivateWS");
  await expect(privateWs).toContainText("1可用\/2配置");
  await expect(privateWs).toHaveClass(/degraded/);
  await expect(privateWs).toHaveAttribute("title", /private_ws_order_stream/);
  await expect(privateWs).toHaveAttribute("title", /source pr_bx_private_ws_runtime/);
  await expect(privateWs).not.toHaveAttribute("title", /App WS channel|rest_orderbooks/);

  await expect(appWs).toContainText("AppWS");
  await expect(appWs).toContainText("已订阅");
  await expect(appWs).not.toHaveClass(/degraded/);
  await expect(appWs).toHaveAttribute("title", /App WS channel system/);
  await expect(appWs).not.toHaveAttribute("title", /private_ws_order_stream|pr_bx_private_ws_runtime/);

  const elapsed = status.getByTestId("status-order-elapsed");
  await expect(elapsed).toContainText("订单最终结果");
  await expect(elapsed).toContainText("24ms");
  await expect(elapsed).not.toContainText("RTT");
  await expect(elapsed).toHaveAttribute("title", /OrderRecord updated_at - created_at/);
  await expect(elapsed).toHaveAttribute("title", /不代表网络 RTT/);
});

test("PR-BX Settings separates configuration, capability, and current usability", async ({
  page,
}) => {
  await page.goto("/#settings");
  await expect(page.locator("h1", { hasText: "设置" })).toBeVisible();
  await page.getByRole("tab", { name: "诊断" }).click();

  const table = page.locator("table.settings-table").filter({
    has: page.getByRole("columnheader", { name: "当前可用" }),
  });
  await expect(table).toBeVisible();
  await expect(table.locator("thead")).toContainText("配置状态");
  await expect(table.locator("thead")).toContainText("能力支持");
  await expect(table.locator("thead")).toContainText("当前可用");

  await page.getByLabel("搜索状态").fill("req-pr-bx-trading-warn");
  const row = table
    .locator("tbody tr")
    .filter({ hasText: "okx" })
    .filter({ hasText: "order_write" });

  await expect(row).toBeVisible();
  await expect(row).toContainText("观察");
  await expect(row).toContainText("配置存在");
  await expect(row).toContainText("支持");
  await expect(row).toContainText("不可用");
  await expect(row).toContainText("pr_bx_trading_runtime");
  await expect(row).toContainText("request_id req-pr-bx-trading-warn");
  await expect(row).toContainText("retry 17000ms");
  await expect(row).toContainText("runtime retry 23000ms");
  await expect(row.locator("td").last()).toHaveAttribute(
    "title",
    /runtime_source=pr_bx_trading_runtime/,
  );
});
