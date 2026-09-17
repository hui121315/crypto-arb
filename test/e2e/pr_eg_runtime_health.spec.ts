import { expect, test, type Page } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";
const WEB_BASE = process.env.CROSSLINE_E2E_WEB_BASE ?? "http://127.0.0.1:18080";
const VENUES = ["binance", "bitget", "bybit", "gate", "hyperliquid", "kucoin", "okx"];

function operation(operation, status, source, usable, extra = {}) {
  return {
    operation,
    status,
    source,
    freshnessMs: 120,
    capabilityStatus: "supported",
    configurationStatus: "configured",
    currentlyUsable: usable,
    observedAtMs: 1_788_000_000_000,
    ...extra,
  };
}

function runtimeSnapshot() {
  const venues = VENUES.map((venue) => ({
    venue,
    publicRest: operation("public_rest", "ok", "http_metrics", true, {
      latencyMs: 12,
      latencyP95Ms: 20,
      requested: 4,
      rows: 4,
    }),
    privateRest: operation("private_rest", venue === "okx" ? "warn" : "blocked", "private_read", false, {
      latencyMs: 31,
      latencyP95Ms: 45,
      requested: 4,
      rows: venue === "okx" ? 3 : 0,
      lastError: "private read requires attention",
      retryAfterMs: 15000,
      requestId: `req-${venue}`,
      problem: {
        code: venue === "okx" ? "PRIVATE_READ_PARTIAL" : "CREDENTIAL_NOT_CONFIGURED",
        message: "private read requires attention",
        requestId: `req-${venue}`,
        retryAfterMs: 15000,
        source: "venue-runtime-health",
      },
    }),
    privateWs: operation("private_ws", "blocked", "private_ws_runtime", false),
    placeOrder: operation("place_order", "unknown", "live_order_proof_runtime", false, {
      requested: 2,
      rows: 0,
    }),
    cancelOrder: operation("cancel_order", "unknown", "live_order_proof_runtime", false, {
      requested: 2,
      rows: 0,
    }),
    orderStream: operation("order_stream", venue === "okx" ? "ok" : "unknown", "private_ws_runtime", venue === "okx", {
      requested: 4,
      rows: venue === "okx" ? 4 : 0,
    }),
    ...(venue === "hyperliquid"
      ? {}
      : { finality: operation("finality", "unknown", "run_finality_runtime", false) }),
    generatedAtMs: 1_788_000_000_000,
  }));
  const operationCount = venues.reduce((count, venue) =>
    count + ["publicRest", "privateRest", "privateWs", "placeOrder", "cancelOrder", "orderStream", "finality"]
      .filter((field) => venue[field]).length, 0);
  return {
    venues,
    generatedAtMs: 1_788_000_000_000,
    venueCount: venues.length,
    operationCount,
    currentlyUsableCount: 9,
    attentionCount: operationCount - 9,
  };
}

async function useRuntimeHealthScenario(page: Page) {
  await page.addInitScript((apiBase) => {
    window.localStorage.setItem("api_base", JSON.stringify(apiBase));
    window.localStorage.setItem("api_auth_token", JSON.stringify("e2e-token"));
  }, API_BASE);
  await page.routeWebSocket("**/ws", (socket) => socket.close());
  await page.route("**/api/system/venue-runtime-health**", async (route) => {
    const headers = {
      "access-control-allow-origin": WEB_BASE,
      "access-control-allow-methods": "GET,OPTIONS",
      "access-control-allow-headers": "authorization,accept,x-request-id",
      "content-type": "application/json; charset=utf-8",
      vary: "origin",
    };
    if (route.request().method() === "OPTIONS") {
      await route.fulfill({ status: 204, headers, body: "" });
      return;
    }
    await route.fulfill({
      status: 200,
      headers,
      body: JSON.stringify(runtimeSnapshot()),
    });
  });
}

test("PR-EG Settings consumes typed venue runtime health and fails closed without evidence", async ({ page }) => {
  await useRuntimeHealthScenario(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/#settings");
  await page.getByRole("tab", { name: "诊断" }).click();

  const matrix = page.locator('[data-settings-table="venue-runtime-health"]');
  await expect(matrix).toBeVisible();
  await expect(matrix).toContainText("7 家交易所");
  await expect(matrix).toContainText("当前可用 9 / 55 项");
  await expect(matrix.locator("tbody tr")).toHaveCount(7);

  const okx = matrix.locator("tbody tr").filter({ hasText: "okx" });
  await expect(okx).toContainText("3/4");
  await expect(okx).toContainText("4/4");
  await expect(okx.locator("td").nth(2)).toHaveAttribute("title", /PRIVATE_READ_PARTIAL/);
  await expect(okx.locator("td").nth(2)).toHaveAttribute("title", /request_id req-okx/);
  await expect(okx.locator("td").nth(2)).toHaveAttribute("title", /p95 45ms/);

  const hyperliquid = matrix.locator("tbody tr").filter({ hasText: "hyperliquid" });
  await expect(hyperliquid).toContainText("无证据");
  await expect(hyperliquid).not.toContainText("11/11");

  const desktop = await matrix.evaluate((section) => {
    const scroller = section.querySelector<HTMLElement>(".table-wrap");
    const table = section.querySelector<HTMLTableElement>("table");
    return {
      rootOverflow: document.documentElement.scrollWidth - window.innerWidth,
      sectionWidth: section.getBoundingClientRect().width,
      scrollerWidth: scroller?.clientWidth ?? 0,
      tableWidth: table?.getBoundingClientRect().width ?? 0,
    };
  });
  expect(desktop.rootOverflow).toBeLessThanOrEqual(1);
  expect(desktop.sectionWidth).toBeGreaterThan(1000);
  expect(desktop.scrollerWidth).toBeGreaterThan(1000);
  expect(Math.abs(desktop.tableWidth - desktop.scrollerWidth)).toBeLessThanOrEqual(2);

  await page.setViewportSize({ width: 390, height: 844 });
  const mobile = await matrix.evaluate((section) => {
    const scroller = section.querySelector<HTMLElement>(".table-wrap");
    const table = section.querySelector<HTMLTableElement>("table");
    const cell = table?.querySelector<HTMLTableCellElement>("tbody td");
    return {
      rootOverflow: document.documentElement.scrollWidth - window.innerWidth,
      sectionWidth: section.getBoundingClientRect().width,
      scrollerWidth: scroller?.clientWidth ?? 0,
      scrollerScrollWidth: scroller?.scrollWidth ?? 0,
      overflowX: scroller ? getComputedStyle(scroller).overflowX : "",
      cellWhiteSpace: cell ? getComputedStyle(cell).whiteSpace : "",
    };
  });
  expect(mobile.rootOverflow).toBeLessThanOrEqual(1);
  expect(mobile.sectionWidth).toBeLessThanOrEqual(370);
  expect(mobile.scrollerScrollWidth).toBeGreaterThan(mobile.scrollerWidth);
  expect(mobile.overflowX).toBe("auto");
  expect(mobile.cellWhiteSpace).toBe("normal");
});
