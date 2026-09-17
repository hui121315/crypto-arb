import { expect, test, type Page, type Route } from "@playwright/test";

const WEB_BASE = process.env.CROSSLINE_E2E_WEB_BASE ?? "http://127.0.0.1:18080";
const FIXTURE_TIME_MS = 1_789_500_000_000;

function routeHeaders(requestId: string) {
  return {
    "access-control-allow-origin": WEB_BASE,
    "access-control-allow-methods": "GET,OPTIONS",
    "access-control-allow-headers":
      "content-type,authorization,accept,x-request-id,idempotency-key",
    "access-control-expose-headers": "x-request-id,retry-after",
    "content-type": "application/json; charset=utf-8",
    "x-request-id": requestId,
    vary: "origin",
  };
}

async function fulfillJson(route: Route, body: unknown, requestId: string) {
  const headers = routeHeaders(requestId);
  if (route.request().method() === "OPTIONS") {
    await route.fulfill({ status: 204, headers, body: "" });
    return;
  }
  await route.fulfill({ status: 200, headers, body: JSON.stringify(body) });
}

async function useReviewTable(page: Page) {
  await page.addInitScript(() => {
    window.localStorage.setItem("crossline.review.activeTab", JSON.stringify("executed"));
    window.localStorage.setItem(
      "crossline.review.venueQuality.view",
      JSON.stringify("table"),
    );
  });
  await page.routeWebSocket("**/ws", (socket) => socket.close());
}

function executedEnvelope() {
  return {
    rows: [
      {
        id: "exec-pr-dw-1",
        strategy: "perp_cross",
        symbol: "PRDW",
        longVenue: "binance",
        shortVenue: "okx",
        openedAtMs: FIXTURE_TIME_MS - 120_000,
        closedAtMs: FIXTURE_TIME_MS - 60_000,
        holdingMinutes: 1,
        grossPnlUsd: 12.5,
        feeUsd: 0.5,
        fundingUsd: 1.25,
        slippageUsd: 0.2,
        netPnlUsd: 13.05,
        evidence: {
          fillEventIds: ["fill-long-pr-dw", "fill-short-pr-dw"],
          feeEventIds: ["fee-long-pr-dw", "fee-short-pr-dw"],
          fundingEventIds: ["funding-pr-dw"],
          slippageEventIds: ["slip-long-pr-dw", "slip-short-pr-dw"],
          estimatedSlippageFillEventIds: [],
          orderbookEventIds: ["book-long-pr-dw", "book-short-pr-dw"],
          closeRunEvidence: [
            {
              closeRunId: "close-pr-dw",
              status: "compensated",
              runId: "run-pr-dw",
              ticketId: "ticket-pr-dw",
              opportunityId: "opp-pr-dw",
              matchedNotionalUsd: 1_000,
              unwindStatus: "compensated",
              compensationAttemptCount: 1,
              costReconciliation: {
                fundingUsd: 1.25,
                manualHandlingUsd: 0.1,
                totalActualCostUsd: 1.35,
                evidenceEventIds: ["funding-pr-dw", "manual-pr-dw"],
                fundingEventIds: ["funding-pr-dw"],
                manualHandlingEventIds: ["manual-pr-dw"],
              },
            },
          ],
          fillConfidence: "venue_fill",
          fillConfidenceScore: 1,
        },
        actualFields: ["gross", "fee", "funding", "slippage", "net"],
        estimatedFields: [],
        missingFields: [],
        longOrders: [],
        shortOrders: [],
      },
    ],
    generatedAtMs: FIXTURE_TIME_MS,
    days: 30,
    source: "execution_ledger",
    rowCount: 1,
    page: {
      limit: 50,
      maxLimit: 100,
      startOffset: 0,
      returnedCount: 1,
      totalRows: 1,
      hasMore: false,
      snapshotId: "review-pr-dw-snapshot",
    },
    status: "fresh",
    ledgerStatus: "ledger_backed",
    missingFields: [],
    requestId: "req-review-body-pr-dw",
    storageHealth: {
      venue: "review",
      operation: "storage:review_execution_ledger",
      status: "warn",
      source: "postgres+jsonl_fallback",
      message: "PostgreSQL authoritative; JSONL fallback available",
      supported: true,
      configured: true,
      requested: 1,
      rows: 1,
      freshnessMs: 250,
      latencyMs: 12,
      observedAtMs: FIXTURE_TIME_MS,
    },
    fundingPaymentIngest: {
      observedAtMs: FIXTURE_TIME_MS,
      windowStartMs: FIXTURE_TIME_MS - 86_400_000,
      windowEndMs: FIXTURE_TIME_MS,
      fetched: 3,
      mapped: 3,
      ledgerEvents: 2,
      skipped: 1,
      invalid: 0,
      duplicateOrAlreadyRecorded: 1,
      invalidMatchKey: 0,
      noMatchingOrder: 0,
      noFilledAnchor: 0,
      ambiguousOrderGroup: 0,
      unmatchedOrAmbiguousOrder: 0,
      routeFailures: 0,
      unsupported: false,
      skipReasons: [{ reason: "duplicate_or_already_recorded", count: 1 }],
    },
    problems: [],
  };
}

function qualityEnvelope() {
  const problem = {
    code: "HTTP_RATE_LIMITED",
    message: "Binance orderbook rate limited",
    status: 429,
    source: "exchange_http_metrics",
    requestId: "req-operation-pr-dw",
    retryAfterMs: 60_000,
    details: {
      venue: "binance",
      operation: "http_rest:GET /fapi/v1/depth",
    },
  };
  return {
    rows: [
      {
        venue: "binance",
        source: "exchange_http_metrics",
        sampleStatus: "warming_up",
        avgRestLatencyMs: 30,
        restLatencySamples: 4,
        wsJitterP99Ms: 0,
        wsJitterSamples: 0,
        fillRatePct: 0,
        fillWindowSamples: 0,
        avgSlippageBps: 0,
        slippageSamples: 0,
        uptimeWindowPct: 75,
        uptimeWindowSamples: 4,
        sampleWindow: {
          maxSamplesPerMetric: 600,
          readySampleMin: 100,
          oldestOperationObservedAtMs: FIXTURE_TIME_MS - 2_000,
          latestOperationObservedAtMs: FIXTURE_TIME_MS - 500,
        },
        operationHealth: [
          {
            venue: "binance",
            operation: "http_rest:GET /fapi/v1/depth",
            status: "warn",
            source: "exchange_http_metrics",
            message: "Binance orderbook rate limited",
            supported: true,
            requested: 1,
            rows: 1,
            freshnessMs: 500,
            retryAfterMs: 60_000,
            latencyMs: 30,
            latencyP95Ms: 100,
            error: "rate limited",
            problem,
            observedAtMs: FIXTURE_TIME_MS - 500,
          },
        ],
        retryAfterMs: 60_000,
        lastProblem: problem,
      },
      {
        venue: "okx",
        source: "no_sample",
        sampleStatus: "no_sample",
        avgRestLatencyMs: 0,
        restLatencySamples: 0,
        wsJitterP99Ms: 0,
        wsJitterSamples: 0,
        fillRatePct: 0,
        fillWindowSamples: 0,
        avgSlippageBps: 0,
        slippageSamples: 0,
        uptimeWindowPct: 0,
        uptimeWindowSamples: 0,
        sampleWindow: { maxSamplesPerMetric: 600, readySampleMin: 100 },
        operationHealth: [],
      },
    ],
    generatedAtMs: FIXTURE_TIME_MS,
    source: "runtime_samples",
    rowCount: 2,
    sampledCount: 1,
    operationCount: 1,
    attentionCount: 1,
    retryAfterMs: 60_000,
    requestId: "req-quality-body-pr-dw",
  };
}

test("PR-DW keeps durable review, funding, close, unwind, storage, and body request evidence visible", async ({
  page,
}) => {
  await useReviewTable(page);
  await page.route("**/api/review/executed**", async (route) => {
    await fulfillJson(route, executedEnvelope(), "req-review-header-pr-dw");
  });

  await page.goto("/#review");
  await expect(page.locator("h1", { hasText: "复盘" })).toBeVisible();

  const review = page.locator(".surface").filter({ hasText: "交易复盘" });
  const meta = review.locator(".reason-pill").first();
  await expect(meta).toContainText("执行账本");
  await expect(meta).toContainText("账本完整");
  await expect(meta).toContainText("request_id req-review-body-pr-dw");
  await expect(meta).toContainText("资金费入账 2/3");
  await expect(meta).toContainText("存储警告");

  const row = review.locator("tbody tr").filter({ hasText: "PRDW" });
  await expect(row).toContainText("事件 fill:2 fee:2 funding:1 slip:2 book:2");
  await expect(row).toContainText("CloseRun:1 cost:2");
  await expect(row).toContainText(
    "close-pr-dw compensated run:run-pr-dw ticket:ticket-pr-dw unwind:compensated comp:1 cost:2",
  );
});

test("PR-DW renders bounded per-operation quality without cross-venue latency leakage and honors retry", async ({
  page,
}) => {
  await useReviewTable(page);
  let qualityRequests = 0;
  await page.route("**/api/trading/venues/quality", async (route) => {
    if (route.request().method() !== "OPTIONS") qualityRequests += 1;
    await fulfillJson(route, qualityEnvelope(), "req-quality-header-pr-dw");
  });

  await page.goto("/#review");
  await expect(page.locator("h1", { hasText: "复盘" })).toBeVisible();

  const quality = page.locator(".surface").filter({ hasText: "场所执行质量" });
  const meta = quality.locator(".reason-pill").first();
  await expect(meta).toContainText("1 operation");
  await expect(meta).toContainText("1 需关注");
  await expect(meta).toContainText("retry 60000ms");
  await expect(meta).toContainText("request_id req-quality-body-pr-dw");
  await expect(meta).toContainText("HTTP_RATE_LIMITED");

  const binance = quality.locator("tbody tr").filter({ hasText: "binance" });
  const okx = quality.locator("tbody tr").filter({ hasText: "okx" });
  await expect(binance).toContainText("30ms");
  await expect(binance).toContainText("75.0%");
  await expect(binance).toContainText("1 项 · 1 需关注 · retry 60000ms");
  await expect(binance.locator("td").last()).toHaveAttribute(
    "title",
    /http_rest:GET \/fapi\/v1\/depth.*100ms.*HTTP_RATE_LIMITED request_id req-operation-pr-dw/,
  );
  await expect(okx).toContainText("未采样");
  await expect(okx).not.toContainText("30ms");

  await expect.poll(() => qualityRequests).toBeGreaterThanOrEqual(1);
  const beforeRetryWindow = qualityRequests;
  await page.waitForTimeout(5_500);
  expect(qualityRequests).toBe(beforeRetryWindow);
});
