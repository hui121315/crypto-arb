import { expect, test, type Page } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";
const STATUS_BAR = "top-status-bar";
const API_SLOT = "status-api-runtime";

async function useScenario(page: Page, scenario: string) {
  await page.addInitScript(
    ({ apiBase, scenarioName }) => {
      localStorage.setItem("api_base", JSON.stringify(`${apiBase}/${scenarioName}`));
      localStorage.setItem("api_auth_token", JSON.stringify("e2e-token"));
    },
    { apiBase: API_BASE, scenarioName: scenario },
  );
}

test("PR-BK HTTP RTT stays traceable across operation health, top bar, and Settings", async ({
  page,
}) => {
  await useScenario(page, "e2e-api-transport");
  const operationHealth = page.waitForResponse((response) =>
    response.url().includes("/e2e-api-transport/api/system/venue-operation-health")
      && response.status() === 200
  );

  await page.goto("/#futures");
  await expect(page.getByRole("heading", { name: "期货套利" })).toBeVisible();
  await operationHealth;

  const apiSlot = page.getByTestId(STATUS_BAR).getByTestId(API_SLOT);
  await expect(apiSlot).toHaveAttribute("title", /HTTP RTT 41ms \/ p95≤80ms/);
  await expect(apiSlot).toHaveAttribute("title", /request_id req-api-transport/);
  await expect(apiSlot).not.toHaveAttribute("title", /订单终态耗时 41ms/);

  await page.goto("/#settings");
  await expect(page.locator("h1", { hasText: "设置" })).toBeVisible();
  await page.getByRole("tab", { name: "诊断" }).click();
  await page.getByLabel("搜索状态").fill("http_rest:GET /api/v4/orders");

  const row = page
    .locator("tr")
    .filter({ hasText: "gate" })
    .filter({ hasText: "http_rest:GET /api/v4/orders" });
  await expect(row).toBeVisible();
  await expect(row).toContainText("HTTP RTT 41ms");
  await expect(row).toContainText("p95≤80ms");
  await expect(row).toContainText("request_id req-api-transport");
  await expect(page.getByText(/send\(\) 到响应头返回/)).toBeVisible();
  await expect(page.getByText(/不含 HostGate\/singleflight\/RateLimiter 本地等待和响应体处理/))
    .toBeVisible();
});

test("PR-BK safe permission probes remain fail closed until live runtime proof", async ({ page }) => {
  await useScenario(page, "e2e-pr-fu-settings-environment");
  const credentials = page.waitForResponse((response) =>
    response.url().includes("/e2e-pr-fu-settings-environment/api/exchanges/credentials")
      && response.status() === 200
  );
  const operationHealth = page.waitForResponse((response) =>
    response.url().includes(
      "/e2e-pr-fu-settings-environment/api/system/venue-operation-health",
    ) && response.status() === 200
  );

  await page.goto("/#settings");
  await expect(page.locator("h1", { hasText: "设置" })).toBeVisible();
  await credentials;
  await operationHealth;

  const validation = page.locator(".runtime-health-panel").filter({ hasText: "保存期验证" });
  await expect(validation).toContainText("权限未完整");
  await expect(validation).toContainText("safe/noop probe 未授予 live_write");

  const permission = page
    .locator("tr")
    .filter({ hasText: "下单/撤单权限" })
    .filter({ hasText: "credential_probe:order_permission" });
  await expect(permission).toContainText("待验证");
  await expect(permission).toContainText("credential_validation");
  await expect(permission).toContainText("does_not_grant_live_write=true");

  const orderWrite = page.locator("tr").filter({ hasText: "order_write" });
  await expect(orderWrite).toContainText("待证据");
  await expect(orderWrite).toContainText("live place/cancel/finality 证据");
  await expect(orderWrite).not.toContainText("可下单");
});
