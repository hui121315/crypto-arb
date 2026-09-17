import { expect, test } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";
const WEB_BASE = process.env.CROSSLINE_E2E_WEB_BASE ?? "http://127.0.0.1:18080";

async function useApiBase(page) {
  await page.addInitScript((apiBase) => {
    window.localStorage.setItem("api_base", JSON.stringify(apiBase));
  }, API_BASE);
}

async function routeFundingEnvelope(page, envelope) {
  await page.route("**/api/arbitrage/funding-rates", async (route) => {
    const headers = {
      "access-control-allow-origin": WEB_BASE,
      "access-control-allow-methods": "GET,OPTIONS",
      "access-control-allow-headers":
        "content-type,authorization,accept,x-request-id,idempotency-key",
      "access-control-expose-headers": "retry-after,x-request-id",
      "content-type": "application/json; charset=utf-8",
      vary: "origin",
    };
    if (route.request().method() === "OPTIONS") {
      await route.fulfill({ status: 204, headers, body: "" });
      return;
    }
    await route.fulfill({ status: 200, headers, body: JSON.stringify(envelope) });
  });
}

async function openFundingDiagnostics(page) {
  await page.goto("/#settings");
  await expect(page.locator("h1", { hasText: "设置" })).toBeVisible();
  await page.getByRole("tab", { name: "诊断" }).click();
  return page.locator(".funding-runtime-health");
}

test("PR-ET funding cold start stays warming instead of becoming a healthy empty state", async ({
  page,
}) => {
  await useApiBase(page);
  await routeFundingEnvelope(page, {
    data: [],
    health: {
      quality: "missing",
      source: "rest_cold_start",
      lastError: "funding snapshot warming",
      observedAtMs: 1_770_000_000_000,
    },
    rowEvidence: [],
    fanout: [],
  });

  const runtime = await openFundingDiagnostics(page);

  await expect(runtime).toContainText("0 条 funding 行");
  await expect(runtime).toContainText("缺数据");
  await expect(runtime).toContainText("REST 冷启动");
  await expect(runtime).toContainText("funding snapshot warming");
  await expect(runtime).not.toContainText("新鲜");
});

test("PR-ET funding degradation exposes typed request source and retry context", async ({ page }) => {
  await useApiBase(page);
  await routeFundingEnvelope(page, {
    data: [],
    health: {
      quality: "rate_limited",
      source: "rest_fallback",
      freshnessMs: 12_000,
      retryAfterMs: 5_000,
      observedAtMs: 1_770_000_000_000,
      problem: {
        code: "FUNDING_REFRESH_RATE_LIMITED",
        message: "funding refresh rate limited",
        status: 429,
        requestId: "req-funding-pr-et",
        retryAfterMs: 5_000,
        source: "funding_refresh",
      },
    },
    retryAfterMs: 8_000,
    rowEvidence: [],
    fanout: [],
  });

  const runtime = await openFundingDiagnostics(page);

  await expect(runtime).toContainText("限频");
  await expect(runtime).toContainText("REST 兜底");
  await expect(runtime).toContainText("FUNDING_REFRESH_RATE_LIMITED");
  await expect(runtime).toContainText("HTTP 429");
  await expect(runtime).toContainText("request_id req-funding-pr-et");
  await expect(runtime).toContainText("source funding_refresh");
  await expect(runtime).toContainText("5000ms 后重试");
  await expect(runtime).toContainText("envelope retry 8000ms");
});
