import { expect, test, type Page } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";
const CHECKED_AT_MS = Date.parse("2026-07-16T08:00:00.000Z");

async function useApiBase(page: Page) {
  await page.addInitScript((apiBase) => {
    window.localStorage.setItem("api_base", JSON.stringify(apiBase));
  }, API_BASE);
}

async function routeMainP0Leak(page: Page) {
  await page.route("**/api/v3/arbitrage/opportunities/list**", async (route) => {
    if (route.request().method() !== "GET") {
      await route.continue();
      return;
    }
    const response = await route.fetch();
    const body = await response.json();
    const diagnostic = structuredClone(body.rows[0]);
    diagnostic.id = "diagnostic-triangular";
    diagnostic.symbol = "HIDDEN-DIAGNOSTIC";
    diagnostic.strategyKind = "triangular";
    diagnostic.typeLabel = "三角套利";
    diagnostic.execution.eligible = true;
    diagnostic.execution.blockers = [];
    body.rows.push(diagnostic);
    body.page.returnedCount = body.rows.length;
    body.page.totalRows = body.rows.length;
    await route.fulfill({ response, json: body });
  });
}

async function routeAuthoritativeDetail(page: Page) {
  await page.route("**/api/v3/arbitrage/opportunities/*/detail**", async (route) => {
    if (route.request().method() !== "GET") {
      await route.continue();
      return;
    }
    const response = await route.fetch();
    const body = await response.json();
    const windows = body.opportunity.fundingDiffWindows.map((window) => ({
      ...window,
      evidence: {
        source: "funding-history-sql",
        observedAtMs: CHECKED_AT_MS,
        latestAtMs: CHECKED_AT_MS - 1_000,
        freshnessMs: 1_000,
        sampleCount: window.sampleCount,
        sampleHealth: "ok",
        problem: null,
        retryAfterMs: null,
      },
    }));
    body.opportunity.fundingDiffWindows = windows;
    body.opportunity.fundingDiffWindow = windows.at(-1);
    body.opportunity.indexComposition = {
      status: "verified",
      overlapScore: 0.42,
      longQuality: "verified",
      shortQuality: "verified",
      blocker: null,
    };
    body.longIndexComposition = indexEnvelope("hyperliquid:km", "MU-LONG-INDEX");
    body.shortIndexComposition = indexEnvelope("kucoin", "MU-SHORT-INDEX");
    await route.fulfill({ response, json: body });
  });
}

function indexEnvelope(venue: string, indexId: string) {
  const components = Array.from({ length: 11 }, (_, index) => ({
    symbol: `COMP-${String(index).padStart(2, "0")}`,
    name: `Index component ${index}`,
    weight: 1 / 11,
    price: 100 + index,
  }));
  return {
    data: {
      venue,
      symbol: "MU",
      indexId,
      components,
      quality: "verified",
      source: "official-rest",
      receivedAtMs: CHECKED_AT_MS,
      freshnessMs: 1_000,
      error: null,
      retryAfterMs: null,
      sourceUrl: `https://official.example/${encodeURIComponent(venue)}/MU`,
      payloadSha256: "0123456789abcdef0123456789abcdef",
      schemaVersion: "index-composition-v2",
    },
    health: {
      quality: "fresh",
      source: "rest_baseline",
      freshnessMs: 1_000,
      retryAfterMs: null,
      lastError: null,
      observedAtMs: CHECKED_AT_MS,
      coverage: null,
      problem: null,
    },
    retryAfterMs: null,
    rowCap: null,
    rowEvidence: [],
    fanout: [],
  };
}

test("PR-AF main opportunity detail consumes complete funding and index evidence", async ({
  page,
}) => {
  await useApiBase(page);
  await routeMainP0Leak(page);
  await routeAuthoritativeDetail(page);

  await page.goto("/#opportunities");
  await expect(page.getByRole("heading", { name: "机会扫描" })).toBeVisible();

  const table = page.locator(".clean-table");
  const muRow = table.locator("tbody tr").filter({ hasText: "MU" }).first();
  await expect(muRow).toHaveCount(1);
  await expect(table.locator("tbody tr").filter({ hasText: "HIDDEN-DIAGNOSTIC" })).toHaveCount(0);
  await muRow.click();

  const funding = page.locator(".funding-cycle-window");
  await expect(funding).toHaveCount(3);
  await expect(funding.last()).toContainText(
    "P50 +0.089% · P75 +0.091% · P90 +0.094% · P95 +0.097%",
  );
  await expect(funding.last()).toContainText("历史 funding-history-sql");
  await expect(funding.last()).toContainText("样本 9 · 健康");
  await expect(funding.last()).toContainText("检查 2026-07-16T08:00:00.000Z");
  await expect(page.locator(".detail-metrics")).toContainText("已验证 42%");

  const indexPanel = page
    .locator(".index-composition-panel")
    .filter({ hasText: "MU-LONG-INDEX" });
  await expect(indexPanel).toContainText("成分 11 项 · 首屏 8 项");
  await expect(indexPanel).toContainText(
    "官方来源 https://official.example/hyperliquid%3Akm/MU",
  );
  await expect(indexPanel).toContainText("schema index-composition-v2");
  await expect(indexPanel).toContainText("取得于 2026-07-16T08:00:00.000Z");
  await expect(indexPanel.getByText("COMP-10", { exact: true })).toBeHidden();
  await indexPanel.locator("summary").click();
  await expect(indexPanel.getByText("COMP-10", { exact: true })).toBeVisible();
});
