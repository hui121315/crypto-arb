import { expect, test, type Page } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";

async function openSettings(page: Page): Promise<void> {
  await page.addInitScript((apiBase) => {
    window.localStorage.setItem("api_base", JSON.stringify(apiBase));
    window.localStorage.setItem("api_auth_token", JSON.stringify("e2e-token"));
  }, API_BASE);
  await page.goto("/#settings");
  await expect(page.locator("h1", { hasText: "设置" })).toBeVisible();
}

test("HTX is absent from the product settings surface", async ({ page }) => {
  await openSettings(page);

  const venue = page.getByLabel("交易所");
  await expect(venue.locator('option[value="htx"]')).toHaveCount(0);
  await expect(venue.locator('option[value="HTX"]')).toHaveCount(0);
});

test("HTX is absent from public transport registries", async ({ request }) => {
  for (const path of [
    "/api/trading/ws/venues",
    "/api/trading/ws/operations",
    "/api/trading/rest/endpoints",
    "/api/trading/fee-schedules",
    "/api/trading/credentials/env-template",
  ]) {
    const response = await request.get(`${API_BASE}${path}`, {
      headers: { Authorization: "Bearer e2e-token" },
    });
    expect(response.ok(), `${path} should be readable`).toBeTruthy();
    expect(JSON.stringify(await response.json()).toLowerCase()).not.toContain('"htx"');
  }
});
