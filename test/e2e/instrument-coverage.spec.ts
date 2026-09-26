import { expect, test, type Page, type Route } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";
const WEB_BASE = process.env.CROSSLINE_E2E_WEB_BASE ?? "http://127.0.0.1:18080";
const OPPORTUNITIES_PATH = "**/api/v3/arbitrage/opportunities/list**";
const CHECKED_AT_MS = 1_770_000_000_000;

type ListingState = "listed" | "unlisted" | "failed" | "stale" | "unsupported";
type MetadataSource = "official_endpoint" | "cached_snapshot" | "manual" | "unverified";

type VenueCoverage = {
  venue: string;
  nativeSymbol: string;
  state: ListingState;
  source: MetadataSource;
  executionReady: boolean;
  checkedAtMs: number;
  staleAfterMs: number | null;
  problem: null;
};

function jsonHeaders(methods = "GET,OPTIONS"): Record<string, string> {
  return {
    "access-control-allow-origin": WEB_BASE,
    "access-control-allow-methods": methods,
    "access-control-allow-headers": "content-type,authorization,accept,x-request-id",
    "content-type": "application/json; charset=utf-8",
    vary: "origin",
  };
}

function venue(
  venue: string,
  state: ListingState,
  source: MetadataSource,
  offset: number,
  executionReady = false,
): VenueCoverage {
  return {
    venue,
    nativeSymbol: "MUUSDT",
    state,
    source,
    executionReady,
    checkedAtMs: CHECKED_AT_MS + offset,
    staleAfterMs: CHECKED_AT_MS + offset + 60_000,
    problem: null,
  };
}

async function useFixtureApiBase(page: Page) {
  await page.addInitScript((apiBase) => {
    window.localStorage.clear();
    window.localStorage.setItem("api_base", JSON.stringify(apiBase));
  }, API_BASE);
}

function coverageDiagnostic(venues: VenueCoverage[]) {
  const stateLabels: Record<ListingState, string> = {
    listed: "已挂牌",
    unlisted: "未挂牌",
    failed: "探测失败",
    stale: "数据依据过期",
    unsupported: "不支持",
  };
  const sourceLabels: Record<MetadataSource, string> = {
    official_endpoint: "官方端点",
    cached_snapshot: "缓存快照",
    manual: "人工",
    unverified: "未核对",
  };
  const executableCount = venues.filter(
    (entry) => entry.state === "listed"
      && entry.source === "official_endpoint"
      && entry.executionReady,
  ).length;
  const diagnostics = venues.map(
    (entry) =>
      `${entry.venue.toUpperCase()} ${stateLabels[entry.state]} · ${sourceLabels[entry.source]} · ${entry.executionReady ? "规格就绪" : "规格阻断"} · 核验 ${entry.checkedAtMs} · 截止 ${entry.staleAfterMs}`,
  );
  return {
    canonicalSymbol: "MU",
    executableCount,
    venueCount: venues.length,
    constructible: executableCount >= 2,
    diagnosticsText: `MU 规格就绪 ${executableCount}/${venues.length} · ${
      executableCount >= 2 ? "可构建跨所双腿" : "仅观察"
    }；${diagnostics.join("；")}`,
  };
}

async function fulfillOpportunityWithCoverage(
  route: Route,
  venues: VenueCoverage[],
  observedOnly = false,
) {
  if (route.request().method() === "OPTIONS") {
    await route.fulfill({ status: 204, headers: jsonHeaders(), body: "" });
    return;
  }
  const response = await route.fetch();
  const body = await response.json();
  body.listing = coverageDiagnostic(venues).diagnosticsText;
  if (observedOnly) {
    const blocker = "挂牌覆盖仅一条 listed，不能构建跨所双腿";
    body.rows = body.rows.map((row) => ({
      ...row,
      execution: { ...row.execution, eligible: false, blockers: [blocker] },
    }));
    body.mainP0Counts = { ...body.mainP0Counts, executableCount: 0 };
    body.registryCounts = { ...body.registryCounts, executableCount: 0 };
    body.scopeMeta = { ...body.scopeMeta, executableCount: 0 };
  }
  await route.fulfill({ response, headers: jsonHeaders(), body: JSON.stringify(body) });
}

async function searchForMu(page: Page) {
  const search = page.waitForResponse(
    (response) =>
      response.url().includes("/api/v3/arbitrage/opportunities/list")
      && response.url().includes("symbol=MU")
      && response.status() === 200,
  );
  await page.getByPlaceholder("MU / kucoin / hyperliquid:xyz").fill("MU");
  await search;
}

test.describe("instrument coverage diagnostics", () => {
  test("PR-EC keeps listed-but-incomplete specs observed-only across all registry venues", async ({ page }) => {
    const rawCoverageRequests: string[] = [];
    page.on("request", (request) => {
      const url = new URL(request.url());
      if (url.pathname.endsWith("/api/venues/instrument-coverage")) {
        rawCoverageRequests.push(request.url());
      }
    });
    const venues = [
      venue("binance", "listed", "official_endpoint", 1, true),
      venue("okx", "listed", "official_endpoint", 2),
      venue("bybit", "failed", "manual", 3),
      venue("gate", "stale", "unverified", 4),
      venue("bitget", "unsupported", "official_endpoint", 5),
      venue("kucoin", "stale", "unverified", 7),
      venue("hyperliquid", "failed", "manual", 8),
      venue("hyperliquid:xyz", "unlisted", "cached_snapshot", 9),
      venue("hyperliquid:cash", "unlisted", "official_endpoint", 10),
      venue("hyperliquid:flx", "unsupported", "unverified", 11),
      venue("hyperliquid:km", "stale", "official_endpoint", 12),
      venue("hyperliquid:vntl", "unlisted", "official_endpoint", 13),
    ];
    await useFixtureApiBase(page);
    await page.route(OPPORTUNITIES_PATH, (route) =>
      fulfillOpportunityWithCoverage(route, venues, true),
    );

    await page.goto("/#opportunities");
    await expect(page.getByRole("heading", { name: "机会扫描" })).toBeVisible();
    await searchForMu(page);
    expect(rawCoverageRequests).toEqual([]);

    const diagnostics = page.locator(".module-toolbar").filter({ hasText: "MU 规格就绪 1/12" });
    await expect(diagnostics).toContainText("MU 规格就绪 1/12 · 仅观察");
    await expect(diagnostics).toContainText("BINANCE 已挂牌 · 官方端点 · 规格就绪 · 核对 1770000000001");
    await expect(diagnostics).toContainText("OKX 已挂牌 · 官方端点 · 规格阻断 · 核对 1770000000002");
    await expect(diagnostics).toContainText("BYBIT 探测失败 · 人工 · 规格阻断 · 核对 1770000000003");
    await expect(diagnostics).toContainText("GATE 数据依据过期 · 未核对 · 规格阻断 · 核对 1770000000004");
    await expect(diagnostics).toContainText("HYPERLIQUID:XYZ 未挂牌 · 缓存快照 · 规格阻断");
    await expect(diagnostics).toContainText("HYPERLIQUID:VNTL 未挂牌 · 官方端点 · 规格阻断");
    await expect(diagnostics).not.toContainText("可构建跨所双腿");

    await expect(diagnostics).toContainText("BYBIT 探测失败");
    await expect(diagnostics).toContainText("GATE 数据依据过期");
    await expect(diagnostics).toContainText("BITGET 不支持");

    const action = page.getByRole("button", { name: "观察" });
    await expect(action).toBeDisabled();
    await expect(action).toHaveAttribute("title", /仅一条 listed/);
    await expect(page.getByRole("button", { name: "构建对冲" })).toHaveCount(0);
  });

  test("PR-EC retains the build action only for two execution-ready specs", async ({ page }) => {
    const venues = [
      venue("binance", "listed", "official_endpoint", 10, true),
      venue("okx", "listed", "official_endpoint", 11, true),
      venue("bybit", "unlisted", "official_endpoint", 12),
      venue("gate", "unlisted", "official_endpoint", 13),
      venue("bitget", "unlisted", "official_endpoint", 14),
      venue("kucoin", "unlisted", "official_endpoint", 16),
      venue("hyperliquid", "unlisted", "official_endpoint", 17),
      venue("hyperliquid:xyz", "unlisted", "official_endpoint", 18),
      venue("hyperliquid:cash", "unlisted", "official_endpoint", 19),
      venue("hyperliquid:flx", "unlisted", "official_endpoint", 20),
      venue("hyperliquid:km", "unlisted", "official_endpoint", 21),
      venue("hyperliquid:vntl", "unlisted", "official_endpoint", 22),
    ];
    await useFixtureApiBase(page);
    await page.route(OPPORTUNITIES_PATH, (route) =>
      fulfillOpportunityWithCoverage(route, venues),
    );

    await page.goto("/#opportunities");
    await expect(page.getByRole("heading", { name: "机会扫描" })).toBeVisible();
    await searchForMu(page);

    const diagnostics = page.locator(".module-toolbar").filter({ hasText: "MU 规格就绪 2/12" });
    await expect(diagnostics).toContainText("MU 规格就绪 2/12 · 可构建跨所双腿");
    await expect(diagnostics).toContainText("BINANCE 已挂牌 · 官方端点 · 规格就绪 · 核对 1770000000010");
    await expect(diagnostics).toContainText("OKX 已挂牌 · 官方端点 · 规格就绪 · 核对 1770000000011");
    await expect(page.getByRole("button", { name: "构建对冲" })).toBeEnabled();
  });

  test("PR-EC Settings exposes exact instrument schema evidence and refresh failure", async ({ page }) => {
    await useFixtureApiBase(page);
    await page.route("**/api/system/venue-operation-health**", async (route) => {
      const evidence = {
        method: "GET",
        path: "/fapi/v1/exchangeInfo",
        checkedAt: "2026-07-11",
        docVersion: "binance-usdm-futures-exchange-information-usdt-usdc-2026-07-11",
        schemaHash: "sha256:10c40d2f5bce94b4f4dcf57226e90fd559f5f63dc79fc7867b6b9866f59a9525",
        fixtureId: "crates/exchange/fixtures/binance/usdm_exchange_info_usdt_usdc.json",
        parserTest: "registry_projection_uses_compiled_usdt_and_usdc_specs",
        requestBuilderTest: "exchange_info_uses_official_path_without_query",
        authKind: "public",
        requestId: null,
        requestContext: [
          "rows=640",
          "execution_ready_rows=618",
          "schema_versions=binance-usdm-futures-exchange-information-usdt-usdc-2026-07-11",
          "fail_closed=true",
        ],
        docUrls: [
          "https://developers.binance.com/docs/derivatives/usds-margined-futures/market-data/rest-api/Exchange-Information",
        ],
        useCases: ["metadata", "instrument_registry", "hedge_sizing"],
        dataKinds: ["instrument_metadata", "instrument_spec"],
        rateScopes: ["ip"],
        weight: 1,
      };
      const rows = [
        {
          venue: "binance",
          operation: "rest_instrument_specs",
          status: "ok",
          source: "instrument_registry",
          message: "official instrument registry contains 618/640 execution-ready rows",
          supported: true,
          configured: true,
          requested: 1,
          rows: 640,
          freshnessMs: 1_000,
          retryAfterMs: null,
          latencyMs: null,
          latencyP95Ms: null,
          error: null,
          evidence,
          problem: null,
          observedAtMs: CHECKED_AT_MS,
        },
        {
          venue: "okx",
          operation: "rest_instrument_specs",
          status: "warn",
          source: "instrument_registry",
          message: "instrument refresh failed; retained 231 cached rows",
          supported: true,
          configured: true,
          requested: 1,
          rows: 231,
          freshnessMs: 5_000,
          retryAfterMs: null,
          latencyMs: null,
          latencyP95Ms: null,
          error: "OKX instrument refresh rate limited",
          evidence: {
            ...evidence,
            path: "/api/v5/public/instruments",
            checkedAt: "2026-06-03",
            docVersion: "okx-v5-public-get-instruments-2026-06-03",
            fixtureId: "crates/exchange/fixtures/okx/public_instruments_swap.json",
            parserTest: "okx_instrument_rule_parses_official_swap_fixture",
            requestBuilderTest: "swap_inst_ids_rest_parses_official_fixture_and_uses_swap_query",
            requestContext: ["rows=231", "execution_ready_rows=0", "fail_closed=true"],
          },
          problem: {
            code: "INSTRUMENT_COVERAGE_REFRESH_FAILED",
            message: "OKX instrument refresh rate limited",
            status: 429,
            requestId: "req-pr-ec-okx-refresh",
            retryAfterMs: 60_000,
            source: "instrument-registry",
            details: { retainedRows: 231, executionReadyRows: 0 },
          },
          observedAtMs: CHECKED_AT_MS + 1,
        },
      ];
      await route.fulfill({
        status: 200,
        headers: jsonHeaders(),
        body: JSON.stringify({
          rows,
          generatedAtMs: CHECKED_AT_MS,
          rowCount: rows.length,
          attentionCount: 1,
        }),
      });
    });

    const health = page.waitForResponse((response) =>
      response.url().includes("/api/system/venue-operation-health") && response.status() === 200,
    );
    await page.goto("/#settings");
    await expect(page.locator("h1", { hasText: "设置" })).toBeVisible();
    await page.getByRole("tab", { name: "诊断" }).click();
    await health;

    const table = page.locator("table.settings-table").filter({
      has: page.getByRole("columnheader", { name: "当前可用" }),
    });
    await page.getByLabel("搜索状态").fill("registry_projection_uses_compiled_usdt_and_usdc_specs");
    const ready = table.locator("tbody tr").filter({ hasText: "binance" });
    await expect(ready).toHaveCount(1);
    await expect(ready).toContainText("REST 合约规格");
    await expect(ready).toContainText("正常");
    await expect(ready.locator("td").last()).toHaveAttribute("title", /execution_ready_rows=618/);
    await expect(ready.locator("td").last()).toHaveAttribute("title", /doc binance-usdm-futures-exchange-information-usdt-usdc-2026-07-11/);
    await expect(ready.locator("td").last()).toHaveAttribute("title", /fixture crates\/exchange\/fixtures\/binance\/usdm_exchange_info_usdt_usdc\.json/);

    await page.getByLabel("搜索状态").fill("INSTRUMENT_COVERAGE_REFRESH_FAILED");
    const failed = table.locator("tbody tr").filter({ hasText: "okx" });
    await expect(failed).toHaveCount(1);
    await expect(failed).toContainText("观察");
    await expect(failed).toContainText("不可用");
    await expect(failed).toContainText("INSTRUMENT_COVERAGE_REFRESH_FAILED");
    await expect(failed.locator("td").last()).toHaveAttribute("title", /execution_ready_rows=0/);
    await expect(failed.locator("td").last()).toHaveAttribute("title", /request_id req-pr-ec-okx-refresh/);
  });
});
