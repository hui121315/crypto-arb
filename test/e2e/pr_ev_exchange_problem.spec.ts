import { expect, test, type Page } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";
const PREVIEW_ROUTE = "**/api/arbitrage/opportunities/mock-mu-perp/preview";

type PreviewResponse = {
  ticket: {
    longLeg: Record<string, unknown>;
  };
};

async function useApiBase(page: Page) {
  await page.addInitScript((apiBase) => {
    window.localStorage.setItem("api_base", JSON.stringify(apiBase));
    window.localStorage.setItem("api_auth_token", JSON.stringify("e2e-token"));
  }, API_BASE);
}

test("PR-EV execution price evidence exposes typed exchange problem context", async ({ page }) => {
  await useApiBase(page);
  await page.route(PREVIEW_ROUTE, async (route) => {
    const upstream = await route.fetch();
    const preview = await upstream.json() as PreviewResponse;
    preview.ticket.longLeg.marketEvidence = {
      venue: "hyperliquid:km",
      symbol: "MU",
      price: 664.63,
      health: {
        quality: "rate_limited",
        source: "rest_fallback",
        freshnessMs: 35,
        retryAfterMs: 2_000,
        lastError: "rate limited",
        observedAtMs: 1_770_000_000_000,
        coverage: { requested: 1, received: 0, coveragePct: 0 },
        problem: {
          code: "UPSTREAM_HTTP",
          message: "rate limited",
          status: 429,
          requestId: "req-pr-ev",
          retryAfterMs: 2_000,
          source: "hyperliquid:km",
          details: {
            operation: "rest_orderbooks",
            symbol: "MU",
            path: "/info action=l2Book",
            latencyMs: 35,
          },
        },
      },
    };
    await route.fulfill({ response: upstream, json: preview });
  });

  const previewResponse = page.waitForResponse((response) =>
    response.url().endsWith("/api/arbitrage/opportunities/mock-mu-perp/preview")
      && response.status() === 200,
  );
  await page.goto("/#opportunities");
  await page.getByRole("button", { name: "构建对冲" }).click();
  await previewResponse;

  const evidence = page.locator(".long-leg .leg-field.readonly").filter({
    hasText: "价格证据",
  });
  await expect(evidence).toContainText("rest_orderbooks");
  await expect(evidence).toContainText("MU");
  await expect(evidence).toContainText("/info action=l2Book");
  await expect(evidence).toContainText("HTTP耗时 35ms");
  await expect(evidence).toContainText("请求 req-pr-ev");
  await expect(evidence).toContainText("2000ms 后重试");
});
