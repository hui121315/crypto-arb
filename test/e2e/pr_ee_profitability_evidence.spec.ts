import { expect, test, type Page } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";

async function useProfitabilityScenario(page: Page) {
  await page.addInitScript((apiBase) => {
    window.localStorage.setItem("api_base", JSON.stringify(apiBase));
    window.localStorage.setItem("api_auth_token", JSON.stringify("e2e-token"));
  }, API_BASE);
  await page.routeWebSocket("**/ws", (socket) => socket.close());
}

test("PR-EE fee registry and profitability evidence fail closed end to end", async ({ page }) => {
  await useProfitabilityScenario(page);

  await page.goto("/#settings");
  await page.getByLabel("交易所").selectOption("okx");
  const registry = page.locator(".ws-venue-panel").filter({ hasText: "Fee schedule fixture 注册表" });
  await expect(registry).toContainText("fee_schedule_registry_v2");
  await expect(registry).toContainText("2.0000 / 5.0000");
  await expect(registry).toContainText("okx-fee-schedule-2026-07-02");
  await expect(registry).toContainText("e2e-okx-perp-fee-schedule");
  await expect(registry.locator(".num")).toHaveAttribute("title", "7e08b1bd44fe99ef");

  await page.route("**/api/v3/arbitrage/opportunities/list**", async (route) => {
    const upstream = await route.fetch();
    const envelope = await upstream.json();
    const row = envelope.rows[0];
    row.cost = {
      ...row.cost,
      verified: false,
      oneCycleNetBps: null,
      feeEvidenceCount: 2,
      feeEvidenceComplete: false,
      feeEvidenceIds: [],
    };
    row.metrics = { ...row.metrics, oneCycleNetBps: null };
    row.execution = {
      ...row.execution,
      eligible: false,
      blockers: ["成本未验证：缺 profitability evidence"],
    };
    await route.fulfill({ json: envelope });
  });

  await page.goto("/#opportunities");
  await expect(page.getByRole("heading", { name: "机会扫描" })).toBeVisible();
  const row = page.locator(".clean-table tbody tr").filter({ hasText: "MU" }).first();
  await expect(row).toContainText("成本未验证");
  await expect(row.getByRole("button", { name: "观察" })).toBeDisabled();
  await expect(page.getByRole("button", { name: "构建对冲" })).toHaveCount(0);
});
