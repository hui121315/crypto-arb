import { expect, test, type Page, type WebSocketRoute } from "@playwright/test";

const API_BASE = `${process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000"}/e2e-position-close-denied`;
const WEB_ORIGIN = process.env.CROSSLINE_E2E_WEB_BASE ?? "http://127.0.0.1:18080";
const NOW = 1_784_500_000_000;

async function useActionScopeScenario(page: Page) {
  let latestSnapshot: any = null;
  let portfolioSocket: WebSocketRoute | null = null;
  let snapshotResponses = 0;
  const subscribedChannels = new Set<string>();
  await page.addInitScript((apiBase) => {
    window.localStorage.setItem("api_base", JSON.stringify(apiBase));
    window.localStorage.setItem("api_auth_token", JSON.stringify("e2e-token"));
  }, API_BASE);
  await page.routeWebSocket("**/ws", (socket) => {
    portfolioSocket = socket;
    socket.onMessage((raw) => {
      const message = JSON.parse(raw.toString());
      if (message.type !== "subscribe") return;
      for (const channel of message.channels ?? []) subscribedChannels.add(channel);
      socket.send(JSON.stringify({
        type: "ack",
        subscribed: message.channels ?? [],
        requestId: message.requestId,
      }));
    });
  });
  await page.route("**/api/trading/portfolio/snapshot", async (route) => {
    const upstream = await route.fetch();
    const envelope = await upstream.json();
    applyPairedPositions(envelope);
    latestSnapshot = envelope.snapshot;
    await route.fulfill({ response: upstream, json: envelope });
    snapshotResponses += 1;
  });
  return {
    socketReady: () =>
      portfolioSocket !== null &&
      latestSnapshot !== null &&
      snapshotResponses >= 2 &&
      subscribedChannels.has("portfolio"),
    setPortfolioControlsVisible: (visible: boolean) => {
      if (!portfolioSocket || !latestSnapshot) return false;
      const snapshot = JSON.parse(JSON.stringify(latestSnapshot));
      snapshot.snapshotVersion = visible ? "pr-bd-positions-visible" : "pr-bd-positions-hidden";
      if (!visible) {
        snapshot.positions = [];
        snapshot.operationHealth = [{
          venue: "binance",
          operation: "positions",
          status: "blocked",
          source: "pr_bd_ws_refresh",
          message: "credentials not configured",
          supported: true,
          configured: false,
          observedAtMs: NOW,
        }];
      }
      portfolioSocket.send(JSON.stringify({
        type: "message",
        channel: "portfolio",
        payload: snapshot,
      }));
      return true;
    },
  };
}

function applyPairedPositions(envelope: any) {
  const snapshot = envelope.snapshot;
  const base = snapshot.positions[0];
  const accountBase = snapshot.accountState.positions.rows[0];
  snapshot.snapshotVersion = "pr-bd-positions-v1";
  snapshot.positions = [
    pairedPosition(base, "binance", "BTCUSDT", "long", "okx", "BTC-USDT-SWAP", "short"),
    pairedPosition(base, "okx", "BTC-USDT-SWAP", "short", "binance", "BTCUSDT", "long"),
  ];
  snapshot.risk.hardLimits.killSwitchActive = false;
  snapshot.accountState.positions = {
    ...snapshot.accountState.positions,
    rows: [
      pairedAccountPosition(accountBase, "binance", "BTCUSDT", "long", "okx@BTC-USDT-SWAP"),
      pairedAccountPosition(accountBase, "okx", "BTC-USDT-SWAP", "short", "binance@BTCUSDT"),
    ],
    rowCount: 2,
    status: "fresh",
    problems: [],
  };
}

function pairedPosition(
  base: any,
  venue: string,
  symbol: string,
  side: string,
  partnerVenue: string,
  partnerSymbol: string,
  partnerSide: string,
) {
  return {
    ...base,
    venue,
    symbol,
    side,
    pairEvidence: {
      source: "execution_run",
      runId: "run-pr-bd-pair",
      ticketId: "ticket-pr-bd",
      opportunityId: "opportunity-pr-bd",
      venue,
      symbol,
      side,
      partnerVenue,
      partnerSymbol,
      partnerSide,
      legFilledQuantity: 1,
      partnerFilledQuantity: 1,
      matchedNotionalUsd: 1_000,
      updatedAtMs: NOW,
    },
    pairedWith: `${partnerVenue}@${partnerSymbol}`,
  };
}

function pairedAccountPosition(
  base: any,
  exchange: string,
  symbol: string,
  side: string,
  pairedWith: string,
) {
  return { ...base, exchange, symbol, side, pairedWith };
}

function closeRun(
  scope: "pair" | "all",
  status: "partially_submitted" | "succeeded",
  requestId: string,
  idempotencyKey: string,
) {
  const partial = status === "partially_submitted";
  return {
    id: `close-pr-bd-${scope}`,
    scope,
    status,
    actionRunId: `action-pr-bd-${scope}`,
    requestId,
    idempotencyKey,
    snapshotVersion: "pr-bd-positions-v1",
    expectedLegCount: 2,
    reason: `positions.close_${scope}`,
    legs: [
      closeLeg("binance", "BTCUSDT", "long", "filled"),
      closeLeg("okx", "BTC-USDT-SWAP", "short", partial ? "failed" : "filled"),
    ],
    submittedOrderCount: partial ? 1 : 2,
    failedLegCount: partial ? 1 : 0,
    nakedExposureUsd: partial ? 1_000 : 0,
    message: partial ? "one venue rejected the paired close" : "both close orders reached finality",
    problem: partial
      ? {
          code: "PAIR_CLOSE_PARTIAL",
          message: "one venue rejected the paired close",
          status: 409,
          requestId,
          source: "pr_bd_fixture",
        }
      : null,
    finalityProblem: null,
    finalityCheckedAtMs: NOW,
    unwindPlan: null,
    costEvents: [],
    costReconciliation: {
      evidenceOrderIds: [`order-pr-bd-${scope}-left`, `order-pr-bd-${scope}-right`],
    },
    startedAtMs: NOW - 1_000,
    updatedAtMs: NOW,
  };
}

function closeLeg(venue: string, symbol: string, side: string, status: string) {
  return {
    venue,
    symbol,
    side,
    status,
    quantity: 1,
    markPrice: 1_000,
    notionalUsd: 1_000,
    costEvents: [],
  };
}

async function fulfillClose(route: any, scope: "pair" | "all", status: "partially_submitted" | "succeeded") {
  if (route.request().method() === "OPTIONS") {
    await route.fulfill({ status: 204, headers: corsHeaders() });
    return;
  }
  const headers = route.request().headers();
  const requestId = headers["x-request-id"] ?? `req-pr-bd-${scope}`;
  const idempotencyKey = headers["idempotency-key"] ?? `idem-pr-bd-${scope}`;
  await route.fulfill({
    status: 200,
    headers: { ...corsHeaders(), "content-type": "application/json", "x-request-id": requestId },
    json: closeRun(scope, status, requestId, idempotencyKey),
  });
}

function corsHeaders() {
  return {
    "access-control-allow-origin": WEB_ORIGIN,
    "access-control-allow-methods": "GET,POST,OPTIONS",
    "access-control-allow-headers": "authorization,content-type,x-request-id,idempotency-key",
  };
}

test("PR-BD keeps typed scope and identity for partial paired close", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 720 });
  await useActionScopeScenario(page);
  await page.route("**/api/trading/portfolio/positions/binance/BTCUSDT/close-pair", (route) =>
    fulfillClose(route, "pair", "partially_submitted"),
  );
  await page.goto("/#positions");

  const row = page.locator(".positions-table tbody tr").first();
  const tableWrap = page.locator(".positions-table-wrap");
  const closeButton = row.getByRole("button", { name: "平配对", exact: true });
  await expect(row).toContainText("okx@BTC-USDT-SWAP");
  const wrapBox = await tableWrap.boundingBox();
  const buttonBox = await closeButton.boundingBox();
  expect(wrapBox).not.toBeNull();
  expect(buttonBox).not.toBeNull();
  expect(await tableWrap.evaluate((element) => element.scrollLeft)).toBe(0);
  expect(buttonBox!.x).toBeGreaterThanOrEqual(wrapBox!.x);
  expect(buttonBox!.x + buttonBox!.width).toBeLessThanOrEqual(wrapBox!.x + wrapBox!.width);
  await closeButton.click();

  const message = page.locator(".positions-main .positions-action-message");
  await expect(message).toContainText("配对平仓未完全完成");
  await expect(message).toContainText("裸露 $1000");
  const evidence = page.locator(".positions-main .positions-action-evidence");
  await expect(evidence).toContainText("action_kind portfolio_close_pair");
  await expect(evidence).toContainText("action_run_id action-pr-bd-pair");
  await expect(evidence).toContainText("run_id close-pr-bd-pair");
  await expect(evidence).toContainText("order_id order-pr-bd-pair-left,order-pr-bd-pair-right");
  await expect(evidence).toContainText("venue binance,okx");
  await expect(evidence).toContainText("symbol BTCUSDT,BTC-USDT-SWAP");
  await expect(evidence).toContainText("idempotency positions-pair:");
  await expect(message).not.toContainText("已完成");
});

test("PR-BD keeps portfolio scope for close-all and kill-switch", async ({ page }) => {
  const scenario = await useActionScopeScenario(page);
  await page.route("**/api/trading/portfolio/close-all", (route) =>
    fulfillClose(route, "all", "succeeded"),
  );
  await page.goto("/#positions");

  const phrase = page.getByPlaceholder("CLOSE_ALL_POSITIONS");
  const closeAll = page.getByRole("button", { name: "全部平仓", exact: true });
  const controls = page.locator(".kill-switch-bar");
  await phrase.fill("CLOSE_ALL_POSITIONS");
  await expect.poll(scenario.socketReady).toBe(true);
  expect(scenario.setPortfolioControlsVisible(false)).toBe(true);
  await expect(controls).toHaveAttribute("hidden", "");
  expect(scenario.setPortfolioControlsVisible(true)).toBe(true);
  await expect(controls).not.toHaveAttribute("hidden", "");
  await expect(phrase).toHaveValue("CLOSE_ALL_POSITIONS");
  await closeAll.click();
  const message = page.locator(".kill-switch-bar .positions-action-message");
  await expect(message).toContainText("action_kind portfolio_close_all");
  await expect(message).toContainText("action_run_id action-pr-bd-all");
  await expect(message).toContainText("run_id close-pr-bd-all");
  await expect(message).toContainText("venue binance,okx");
  await expect(message).toContainText("symbol BTCUSDT,BTC-USDT-SWAP");

  await page.getByRole("button", { name: "开启总闸", exact: true }).click();
  await expect(message).toContainText("action_kind trading_kill_switch");
  await expect(message).toContainText("action_run_id e2e-kill-switch-action");
  await expect(message).toContainText("request_id e2e-kill-switch");
  await expect(message).toContainText("idempotency kill-switch-");
});
