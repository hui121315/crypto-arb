import { expect, test, type Page } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";
const WEB_BASE = process.env.CROSSLINE_E2E_WEB_BASE ?? "http://127.0.0.1:18080";
const SCENARIO = "pr-dh-review-ledger";
const REVIEW_ROUTE = `**/${SCENARIO}/api/review/executed**`;
const SNAPSHOT_ID = "review-e2e-snapshot";
const TOTAL_ROWS = 1_000;
const PAGE_LIMIT = 50;
const FIXTURE_TIME_MS = 1_789_000_000_000;

function routeHeaders() {
  return {
    "access-control-allow-origin": WEB_BASE,
    "access-control-allow-methods": "GET,OPTIONS",
    "access-control-allow-headers":
      "content-type,authorization,accept,x-request-id,idempotency-key",
    "access-control-expose-headers": "x-request-id",
    "content-type": "application/json; charset=utf-8",
    "x-request-id": "req-dh-review-ledger",
    vary: "origin",
  };
}

async function useScenarioApiBase(page: Page) {
  await page.addInitScript(
    ({ apiBase, scenario }) => {
      window.localStorage.setItem("api_base", JSON.stringify(`${apiBase}/${scenario}`));
    },
    { apiBase: API_BASE, scenario: SCENARIO },
  );
}

function cursor(offset: number) {
  return `rv1:${offset}:${SNAPSHOT_ID}`;
}

function pageContract(offset: number, returnedCount: number) {
  const nextOffset = offset + returnedCount;
  return {
    limit: PAGE_LIMIT,
    maxLimit: 100,
    startOffset: offset,
    returnedCount,
    totalRows: TOTAL_ROWS,
    hasMore: nextOffset < TOTAL_ROWS,
    previousCursor: offset > 0 ? cursor(Math.max(0, offset - PAGE_LIMIT)) : null,
    nextCursor: nextOffset < TOTAL_ROWS ? cursor(nextOffset) : null,
    lastCursor: offset < TOTAL_ROWS - PAGE_LIMIT ? cursor(TOTAL_ROWS - PAGE_LIMIT) : null,
    snapshotId: SNAPSHOT_ID,
  };
}

function fieldQuality(index: number) {
  if (index % 3 === 0) {
    return {
      actualFields: ["gross", "fee", "funding", "slippage", "net"],
      estimatedFields: [],
      missingFields: [],
    };
  }
  if (index % 3 === 1) {
    return {
      actualFields: ["gross", "fee", "funding"],
      estimatedFields: ["slippage", "net"],
      missingFields: [],
    };
  }
  return {
    actualFields: ["gross", "fee", "slippage"],
    estimatedFields: [],
    missingFields: ["funding", "net"],
  };
}

function executedTrade(index: number) {
  const row = index + 1;
  const estimatedSlippage = index % 3 === 1;
  return {
    id: `pr-dh-exec-${row}`,
    strategy: "perp_cross",
    symbol: `PRDH${String(row).padStart(4, "0")}`,
    longVenue: "binance",
    shortVenue: "okx",
    openedAtMs: FIXTURE_TIME_MS - row * 60_000,
    closedAtMs: FIXTURE_TIME_MS - row * 30_000,
    holdingMinutes: 8,
    grossPnlUsd: 12.5 + index * 0.01,
    feeUsd: 0.21,
    fundingUsd: 0.7,
    slippageUsd: 0.12,
    netPnlUsd: 12.87 + index * 0.01,
    evidence: {
      fillEventIds: [`fill-long-${row}`, `fill-short-${row}`],
      feeEventIds: [`fill-long-${row}`, `fill-short-${row}`],
      fundingEventIds: index % 3 === 2 ? [] : [`funding-${row}`],
      slippageEventIds: estimatedSlippage ? [] : [`slippage-long-${row}`, `slippage-short-${row}`],
      estimatedSlippageFillEventIds: estimatedSlippage
        ? [`fill-long-${row}`, `fill-short-${row}`]
        : [],
      orderbookEventIds: [`book-long-${row}`, `book-short-${row}`],
      fillConfidence: "venue_order_snapshot",
      fillConfidenceScore: 0.95,
    },
    ...fieldQuality(index),
    longOrders: [],
    shortOrders: [],
  };
}

function reviewEnvelope(offset: number) {
  const end = Math.min(offset + PAGE_LIMIT, TOTAL_ROWS);
  const rows = Array.from({ length: end - offset }, (_, index) => executedTrade(offset + index));
  return {
    rows,
    generatedAtMs: FIXTURE_TIME_MS,
    days: 30,
    source: "execution_ledger",
    rowCount: TOTAL_ROWS,
    page: pageContract(offset, rows.length),
    status: "degraded",
    ledgerStatus: "partial_evidence",
    missingFields: ["funding", "net"],
    problems: [],
  };
}

async function routeReviewLedger(page: Page) {
  const requests: URL[] = [];
  await page.route(REVIEW_ROUTE, async (route) => {
    const headers = routeHeaders();
    if (route.request().method() === "OPTIONS") {
      await route.fulfill({ status: 204, headers, body: "" });
      return;
    }
    const url = new URL(route.request().url());
    requests.push(url);
    const requestedCursor = url.searchParams.get("cursor");
    const match = requestedCursor?.match(/^rv1:(\d+):review-e2e-snapshot$/);
    const offset = match ? Number(match[1]) : 0;
    await route.fulfill({ status: 200, headers, body: JSON.stringify(reviewEnvelope(offset)) });
  });
  return requests;
}

test("PR-DH review ledger keeps explicit PnL quality and snapshot-bound 50-row budget", async ({
  page,
}) => {
  await useScenarioApiBase(page);
  const requests = await routeReviewLedger(page);

  await page.goto("/#review");
  await expect(page.locator("h1", { hasText: "复盘" })).toBeVisible();

  const review = page.locator(".surface").filter({ hasText: "交易复盘" });
  const table = review.locator("table.review-table").first();
  const rows = table.locator("tbody tr");
  await expect(table).toHaveAttribute("data-table-budget", "server-page");
  await expect(review.getByText("1-50 / 1000")).toBeVisible();
  await expect(rows).toHaveCount(PAGE_LIMIT);

  const actualRow = rows.filter({ hasText: "PRDH0001" });
  await expect(actualRow.locator(".reason-pill").filter({ hasText: "真实" })).toHaveCount(5);
  await expect(actualRow).toContainText("真实 5 · 估算 0 · 缺证据 0");

  const estimatedRow = rows.filter({ hasText: "PRDH0002" });
  await expect(estimatedRow.locator(".reason-pill").filter({ hasText: "估算" })).toHaveCount(2);
  await expect(estimatedRow).toContainText("真实 3 · 估算 2 · 缺证据 0");

  const missingRow = rows.filter({ hasText: "PRDH0003" });
  await expect(missingRow.locator(".reason-pill").filter({ hasText: "缺证据" })).toHaveCount(2);
  await expect(missingRow).toContainText("真实 3 · 估算 0 · 缺证据 2");

  const pager = review.locator(".table-pager");
  await pager.getByRole("button", { name: "下一页" }).click();
  await expect(review.getByText("51-100 / 1000")).toBeVisible();
  await expect(rows).toHaveCount(PAGE_LIMIT);
  await expect(rows.first()).toContainText("PRDH0051");

  await pager.getByRole("button", { name: "上一页" }).click();
  await expect(review.getByText("1-50 / 1000")).toBeVisible();

  await pager.getByRole("button", { name: "末页" }).click();
  await expect(review.getByText("951-1000 / 1000")).toBeVisible();
  await expect(rows).toHaveCount(PAGE_LIMIT);
  await expect(rows.last()).toContainText("PRDH1000");

  await expect.poll(() => requests.length).toBeGreaterThanOrEqual(4);
  expect(requests.every((request) => request.searchParams.get("limit") === "50")).toBe(true);
  expect(requests.some((request) => request.searchParams.get("cursor") === cursor(50))).toBe(true);
  expect(requests.some((request) => request.searchParams.get("cursor") === cursor(0))).toBe(true);
  expect(requests.some((request) => request.searchParams.get("cursor") === cursor(950))).toBe(true);
});
