import { expect, test, type Page, type Route } from "@playwright/test";

const WEB_BASE = process.env.CROSSLINE_E2E_WEB_BASE ?? "http://127.0.0.1:18080";
const FIXTURE_TIME_MS = 1_789_600_000_000;

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

async function useExecutedReview(page: Page) {
  await page.addInitScript(() => {
    window.localStorage.setItem("crossline.review.activeTab", JSON.stringify("executed"));
  });
  await page.routeWebSocket("**/ws", (socket) => socket.close());
}

function identity(orderId: string, exchangeId: string) {
  return {
    internalOrderId: orderId,
    publicClientOrderId: `client-${orderId}`,
    exchangeOrderId: exchangeId,
  };
}

function orderRecord(
  orderId: string,
  exchange: string,
  side: "buy" | "sell",
  source: "private_ws" | "order_query",
) {
  const exchangeOrderId = `venue-${orderId}`;
  return {
    intent: {
      id: orderId,
      source: "arbitrage_preview",
      strategy: "perp_cross",
      mode: "live",
      exchange,
      symbol: "BTCUSDT",
      side,
      orderType: "limit",
      quantity: 1,
      price: 100,
      reduceOnly: false,
      timeInForce: "ioc",
      postOnly: false,
      marginMode: "cross",
      leverage: 1,
      clientOrderId: `client-${orderId}`,
      createdAtMs: FIXTURE_TIME_MS - 120_000,
    },
    state: "filled",
    risk: null,
    identity: identity(orderId, exchangeOrderId),
    lastUpdateSource: source,
    exchangeOrderId,
    message: null,
    filledQuantity: 1,
    filledPrice: 100,
    filledFee: 0.1,
    updatedAtMs: FIXTURE_TIME_MS - 60_000,
  };
}

function ledgerEvent(
  eventId: string,
  eventType: string,
  source: string,
  exchange: string,
  legRole: "long" | "short",
  side: "buy" | "sell",
  payload: unknown,
) {
  const orderId = `order-${legRole}`;
  return {
    eventId,
    eventType,
    source,
    order: {
      runId: "run-pr-bc",
      ticketId: "ticket-pr-bc",
      legRole,
      reduceOnly: false,
      exchange,
      symbol: "BTCUSDT",
      side,
      identity: identity(orderId, `venue-${orderId}`),
    },
    timing: {
      occurredAtMs: FIXTURE_TIME_MS - 90_000,
      capturedAtMs: FIXTURE_TIME_MS - 89_000,
    },
    payload,
  };
}

function reviewEnvelope() {
  const fillPayload = (confidence: "venue_fill" | "order_query") => ({
    type: "fill",
    data: {
      quantity: 1,
      average_price: 100,
      quote_value: 100,
      quality: "actual",
      confidence,
    },
  });
  const ledgerEvents = [
    ledgerEvent(
      "fill-long-pr-bc",
      "fill_event",
      "private_ws",
      "binance",
      "long",
      "buy",
      fillPayload("venue_fill"),
    ),
    ledgerEvent(
      "fill-short-pr-bc",
      "fill_snapshot",
      "order_query",
      "okx",
      "short",
      "sell",
      fillPayload("order_query"),
    ),
    ledgerEvent("fee-long-pr-bc", "fee_snapshot", "order_query", "binance", "long", "buy", {
      type: "fee",
      data: { amount: 0.2, currency: "USDT", quality: "actual" },
    }),
    ledgerEvent(
      "funding-pr-bc",
      "funding_payment",
      "funding_poller",
      "binance",
      "long",
      "buy",
      {
        type: "funding",
        data: {
          amount: -0.4,
          currency: "USDT",
          funding_time_ms: FIXTURE_TIME_MS - 80_000,
          quality: "actual",
        },
      },
    ),
    ledgerEvent("slip-pr-bc", "slippage", "internal", "okx", "short", "sell", {
      type: "slippage",
      data: {
        amount_usd: 0.3,
        reference_price: 100.3,
        fill_price: 100,
        quantity: 1,
        quality: "actual",
      },
    }),
  ];

  return {
    rows: [
      {
        id: "exec-pr-bc-1",
        strategy: "perp_cross",
        symbol: "BTCUSDT",
        longVenue: "binance",
        shortVenue: "okx",
        openedAtMs: FIXTURE_TIME_MS - 120_000,
        closedAtMs: FIXTURE_TIME_MS - 60_000,
        holdingMinutes: 1,
        grossPnlUsd: 12.5,
        feeUsd: 0.8,
        fundingUsd: -0.4,
        slippageUsd: 1.3,
        netPnlUsd: 10,
        evidence: {
          fillEventIds: ["fill-long-pr-bc", "fill-short-pr-bc"],
          feeEventIds: ["fee-long-pr-bc"],
          fundingEventIds: ["funding-pr-bc"],
          slippageEventIds: ["slip-pr-bc"],
          estimatedSlippageFillEventIds: [],
          orderbookEventIds: [],
          ledgerEvents,
          closeRunEvidence: [
            {
              closeRunId: "close-pr-bc",
              status: "compensated",
              runId: "run-pr-bc",
              ticketId: "ticket-pr-bc",
              opportunityId: "opp-pr-bc",
              matchedNotionalUsd: 1_000,
              unwindStatus: "compensated",
              compensationAttemptCount: 1,
              costReconciliation: {
                closeFeeUsd: 0.2,
                closeSlippageUsd: 0.3,
                compensationFeeUsd: 0.4,
                compensationSlippageUsd: 0.5,
                fundingUsd: -0.4,
                manualHandlingUsd: 0.1,
                totalActualCostUsd: 1.1,
                evidenceEventIds: [
                  "close-fee-pr-bc",
                  "close-slip-pr-bc",
                  "comp-fee-pr-bc",
                  "comp-slip-pr-bc",
                  "funding-pr-bc",
                  "manual-pr-bc",
                ],
                closeFeeEventIds: ["close-fee-pr-bc"],
                closeSlippageEventIds: ["close-slip-pr-bc"],
                compensationFeeEventIds: ["comp-fee-pr-bc"],
                compensationSlippageEventIds: ["comp-slip-pr-bc"],
                fundingEventIds: ["funding-pr-bc"],
                manualHandlingEventIds: ["manual-pr-bc"],
                missingFields: [],
              },
            },
          ],
          fillConfidence: "venue_fill",
          fillConfidenceScore: 1,
        },
        actualFields: ["gross", "fee", "funding", "slippage", "net"],
        estimatedFields: [],
        missingFields: [],
        longOrders: [orderRecord("order-long", "binance", "buy", "private_ws")],
        shortOrders: [orderRecord("order-short", "okx", "sell", "order_query")],
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
      snapshotId: "review-pr-bc-snapshot",
    },
    status: "fresh",
    ledgerStatus: "ledger_backed",
    missingFields: [],
    requestId: "req-review-body-pr-bc",
    storageHealth: {
      venue: "review",
      operation: "storage:review_sql_ledger",
      status: "ok",
      source: "postgres_sql_ledger",
      message: "PostgreSQL realized facts are current",
      supported: true,
      configured: true,
      requested: 1,
      rows: 1,
      freshnessMs: 100,
      observedAtMs: FIXTURE_TIME_MS,
    },
    problems: [],
  };
}

test("PR-BC explains terminal review facts and every close-unwind cost component", async ({ page }) => {
  await useExecutedReview(page);
  const requests: URL[] = [];
  await page.route("**/api/review/executed**", async (route) => {
    if (route.request().method() !== "OPTIONS") requests.push(new URL(route.request().url()));
    await fulfillJson(route, reviewEnvelope(), "req-review-header-pr-bc");
  });

  await page.goto("/#review");
  const review = page.locator(".surface").filter({ hasText: "交易复盘" });
  const table = review.locator("table.review-table").first();
  const row = table.locator("tbody tr").filter({ hasText: "BTCUSDT" });

  await expect(table).toHaveAttribute("data-table-budget", "server-page");
  await expect(review.locator(".reason-pill").first()).toContainText("账本完整");
  await expect(review.locator(".reason-pill").first()).toContainText("request_id req-review-body-pr-bc");
  await expect(row.locator(".reason-pill").filter({ hasText: "真实" })).toHaveCount(5);
  await expect(row).toContainText("终态 2/2 Filled via 私有 WS/订单回查");
  await expect(row).toContainText("明细 5 条 via 私有 WS/订单回查/资金费轮询/内部状态");
  await expect(row).toContainText(
    "fill-long-pr-bc fill binance run:run-pr-bc ticket:ticket-pr-bc via 私有 WS qty 1 @100 actual 逐笔成交",
  );
  await expect(row).toContainText("fill-short-pr-bc fill okx run:run-pr-bc ticket:ticket-pr-bc");
  await expect(row).toContainText("fee-long-pr-bc fee binance run:run-pr-bc ticket:ticket-pr-bc");
  await expect(row).toContainText("+2 条");
  await expect(row).toContainText(
    "close-pr-bc compensated run:run-pr-bc ticket:ticket-pr-bc unwind:compensated comp:1 cost:6",
  );
  await expect(row).toContainText("close_fee:$0.2/1 close_slip:$0.3/1");
  await expect(row).toContainText("comp_fee:$0.4/1 comp_slip:$0.5/1");
  await expect(row).toContainText("funding:$-0.4/1 manual:$0.1/1 total:$1.1 missing:none");
  await expect.poll(() => requests.length).toBeGreaterThanOrEqual(1);
  expect(requests.every((request) => request.searchParams.get("limit") === "50")).toBe(true);
});
