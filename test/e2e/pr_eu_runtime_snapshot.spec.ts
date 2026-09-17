import { expect, test, type Page } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";

async function useApiBase(page: Page) {
  await page.addInitScript((apiBase) => {
    window.localStorage.setItem("api_base", JSON.stringify(apiBase));
    window.localStorage.setItem("api_auth_token", JSON.stringify("e2e-token"));
  }, API_BASE);
}

test("PR-EU preview keeps the selected opportunity snapshot generation", async ({ page }) => {
  await useApiBase(page);
  await page.goto("/#opportunities");

  await expect(page.getByRole("heading", { name: "机会扫描" })).toBeVisible();
  await expect(page.getByRole("button", { name: "构建对冲" })).toHaveCount(1);

  const previewRequest = page.waitForRequest((request) =>
    request.method() === "POST"
      && request.url().endsWith("/api/arbitrage/opportunities/mock-mu-perp/preview"),
  );
  await page.getByRole("button", { name: "构建对冲" }).click();

  expect((await previewRequest).postDataJSON()).toMatchObject({
    opportunitySnapshotId: "e2e-snapshot-1",
  });
  await page.locator(".execution-evidence-details summary").click();
  await expect(page.getByText("后端 RiskDecision 通过", { exact: true })).toBeVisible();
});
