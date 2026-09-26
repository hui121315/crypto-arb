import { expect, test, type Page } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";
const FIXTURE_TIME_MS = 1_784_000_000_000;

async function usePortfolioTruthScenario(page: Page) {
  await page.addInitScript((apiBase) => {
    window.localStorage.setItem("api_base", JSON.stringify(apiBase));
    window.localStorage.setItem("api_auth_token", JSON.stringify("e2e-token"));
  }, API_BASE);
  await page.routeWebSocket("**/ws", (socket) => socket.close());
  await page.route("**/api/trading/portfolio/snapshot", async (route) => {
    const upstream = await route.fetch();
    const envelope = await upstream.json();
    applyPortfolioTruthFixture(envelope);
    await route.fulfill({ json: envelope });
  });
}

function applyPortfolioTruthFixture(envelope: any) {
  const navProblem = apiProblem(
    "ACCOUNT_FIELD_UNKNOWN",
    "portfolio NAV is unavailable because wallet equity coverage is incomplete",
    "account_state.account_summaries",
    { operation: "portfolio_nav", missingVenues: ["gate"] },
  );
  const positionProblem = apiProblem(
    "POSITION_READ_DEGRADED",
    "gate positions retained with incomplete risk fields",
    "gate.GET /api/v4/futures/usdt/positions",
    { operation: "positions", venue: "gate" },
  );
  const balanceProblem = apiProblem(
    "BALANCE_READ_DEGRADED",
    "gate wallet equity is unavailable",
    "gate.GET /api/v4/wallet/total_balance",
    { operation: "balances", venue: "gate" },
  );
  const positionQuality = positionFieldQuality();
  const equityQuality = fieldQuality(
    { kind: "account", venue: "gate", accountScope: "futures_usdt" },
    "equity",
    "missing",
    balanceProblem,
  );
  const snapshot = envelope.snapshot;
  const accountState = snapshot.accountState;

  envelope.status = "degraded";
  envelope.source = "pr_dz_portfolio_truth";
  envelope.observedAtMs = FIXTURE_TIME_MS;
  envelope.problem = navProblem;
  envelope.problems = [navProblem, positionProblem, balanceProblem];
  envelope.operationHealth = [];
  envelope.retryAfterMs = 2_000;

  snapshot.snapshotVersion = "pr-dz-portfolio-v1";
  snapshot.degraded = true;
  snapshot.summary = {
    totalNavUsd: 0,
    navEvidence: {
      status: "missing",
      source: "account_state.account_summaries.total_equity_usd",
      observedAtMs: FIXTURE_TIME_MS,
      coveredVenues: [],
      missingVenues: ["gate"],
      problem: navProblem,
    },
    navChange24hPct: 0,
    netDeltaUsd: 15_250,
    netDeltaPctOfNav: 0,
    nakedExposureUsd: 15_250,
    nakedPositionCount: 1,
    realizedPnlTodayUsd: -125,
    pnlBreakdown: { fundingUsd: 0, priceUsd: -125, feeRebateUsd: 0 },
    updatedAtMs: FIXTURE_TIME_MS,
  };
  snapshot.positions = [portfolioPosition()];
  snapshot.problems = [
    runtimeProblem("portfolio_nav", navProblem),
    runtimeProblem("positions", positionProblem, "gate"),
  ];
  snapshot.operationHealth = [];
  snapshot.risk = {
    ...snapshot.risk,
    var991dUsd: 625,
    varPctOfNav: 0,
    varSampleSize: 100,
    fundingClustering: [],
    deltaConcentration: [],
    marginUtilization: [],
    updatedAtMs: FIXTURE_TIME_MS,
  };
  snapshot.recentCloseRuns = [durableCompensationRun()];

  accountState.status = "degraded";
  accountState.source = "account_state_runtime";
  accountState.observedAtMs = FIXTURE_TIME_MS;
  accountState.problems = [navProblem, positionProblem, balanceProblem];
  accountState.operationHealth = [];
  accountState.fieldQuality = [equityQuality, ...positionQuality];
  accountState.accountBindings = [];
  accountState.positions = {
    ...accountState.positions,
    rows: [accountPosition()],
    rowCount: 1,
    status: "degraded",
    source: "gate.GET /api/v4/futures/usdt/positions",
    observedAtMs: FIXTURE_TIME_MS,
    problems: [positionProblem],
    operationHealth: [],
    fieldQuality: positionQuality,
    rowHealth: [positionRowHealth()],
    accountBindings: [],
  };
  accountState.balances = {
    ...accountState.balances,
    status: "degraded",
    source: "gate.GET /api/v4/wallet/total_balance",
    observedAtMs: FIXTURE_TIME_MS,
    problems: [balanceProblem],
    operationHealth: [],
    fieldQuality: [equityQuality],
    rowHealth: [],
    accountSummaries: [],
    accountBindings: [],
  };
}

function apiProblem(code: string, message: string, source: string, details: object) {
  return {
    code,
    message,
    status: 200,
    requestId: "req-dz-portfolio-1",
    retryAfterMs: 2_000,
    source,
    details: { ...details, observedAtMs: FIXTURE_TIME_MS },
  };
}

function runtimeProblem(operation: string, problem: any, venue?: string) {
  return {
    scope: "portfolio",
    operation,
    code: problem.code,
    message: problem.message,
    venue: venue ?? null,
    retryAfterMs: problem.retryAfterMs,
    observedAtMs: FIXTURE_TIME_MS,
  };
}

function positionFieldQuality() {
  const subject = { kind: "position", venue: "gate", symbol: "BTCUSDT", side: "long" };
  return [
    ["markPrice", "invalid"],
    ["liquidationPrice", "missing"],
    ["liquidationDistancePct", "missing"],
    ["maintenanceMarginRatio", "unknown"],
    ["margin", "unknown"],
    ["leverage", "invalid"],
    ["fundingRate8h", "missing"],
    ["nextFundingMs", "missing"],
  ].map(([field, status]) => fieldQuality(
    subject,
    field,
    status,
    apiProblem(
      "POSITION_FIELD_UNAVAILABLE",
      `position ${field} is ${status}`,
      "gate.GET /api/v4/futures/usdt/positions",
      { operation: "positions", venue: "gate", field, status },
    ),
  ));
}

function fieldQuality(subject: object, field: string, status: string, problem: object) {
  return {
    subject,
    field,
    status,
    source: "gate.GET /api/v4/futures/usdt/positions",
    observedAtMs: FIXTURE_TIME_MS,
    problem,
  };
}

function portfolioPosition() {
  return {
    venue: "gate",
    symbol: "BTCUSDT",
    side: "long",
    quantity: 0.25,
    entryPrice: 61_000,
    markPrice: 61_000,
    leverage: 0,
    unrealizedPnlUsd: -125,
    liquidationPrice: null,
    liquidationDistancePct: null,
    nextFundingMs: null,
    fundingRate8h: 0,
    fundingRateVerified: false,
    maintenanceMarginRatio: 0,
    pairedWith: null,
    marginUsd: 0,
    severity: "unknown",
    secondsUntilFunding: null,
  };
}

function accountPosition() {
  return {
    symbol: "BTCUSDT",
    exchange: "gate",
    side: "long",
    quantity: 0.25,
    entryPrice: 61_000,
    markPrice: 0,
    unrealizedPnl: -125,
    leverage: 0,
    liquidationPrice: null,
    liquidationDistancePct: null,
    nextFundingMs: null,
    pairedWith: null,
    margin: 0,
    maintenanceMarginRatio: 0,
    positionMode: "single",
    marginMode: "cross",
  };
}

function positionRowHealth() {
  return {
    subject: { kind: "position", venue: "gate", symbol: "BTCUSDT", side: "long" },
    source: "gate.GET /api/v4/futures/usdt/positions",
    observedAtMs: FIXTURE_TIME_MS,
    freshnessMs: 250,
    lastSuccessMs: FIXTURE_TIME_MS - 250,
    lastError: null,
    retryAfterMs: null,
    requestId: "req-dz-position-1",
  };
}

function durableCompensationRun() {
  return {
    id: "close-dz-durable-1",
    scope: "pair",
    status: "compensation_submitted",
    actionRunId: "action-dz-auto-1",
    requestId: "req-dz-close-1",
    idempotencyKey: "auto-compensation:close-dz-durable-1:0",
    snapshotVersion: "pr-dz-portfolio-v1",
    expectedLegCount: 2,
    reason: "positions.close_pair.auto_compensation",
    legs: [],
    submittedOrderCount: 1,
    failedLegCount: 1,
    nakedExposureUsd: 15_250,
    message: "automatic compensation accepted; durable finality pending",
    problem: null,
    finalityProblem: null,
    finalityCheckedAtMs: FIXTURE_TIME_MS - 500,
    unwindPlan: {
      status: "compensation_submitted",
      filledLegs: [],
      failedLegs: [],
      compensationCandidates: [],
      remainingPositions: [],
      compensationAttempts: [],
      nextActions: [{
        kind: "wait_for_compensation_finality",
        label: "等待补偿最终结果",
        requiresConfirmation: false,
        requiredEvidence: ["durable_order_finality"],
        reason: "automatic compensation order is accepted",
      }],
      requiredEvidence: ["close_run_projection", "order_finality"],
    },
    costEvents: [],
    startedAtMs: FIXTURE_TIME_MS - 5_000,
    updatedAtMs: FIXTURE_TIME_MS,
  };
}

test("PR-DZ portfolio envelope keeps wallet NAV, position evidence, partial state, and durable compensation visible", async ({ page }) => {
  await usePortfolioTruthScenario(page);
  const response = page.waitForResponse((item) =>
    item.url().includes("/api/trading/portfolio/snapshot") && item.status() === 200
  );

  await page.goto("/#positions");
  const envelope = await (await response).json();

  expect(envelope.status).toBe("degraded");
  expect(envelope.problem.code).toBe("ACCOUNT_FIELD_UNKNOWN");
  expect(envelope.snapshot.summary.navEvidence.status).toBe("missing");
  expect(envelope.snapshot.positions[0].severity).toBe("unknown");
  expect(envelope.snapshot.accountState.positions.rowHealth[0]).toMatchObject({
    source: "gate.GET /api/v4/futures/usdt/positions",
    freshnessMs: 250,
    requestId: "req-dz-position-1",
  });
  expect(envelope.snapshot.recentCloseRuns[0]).toMatchObject({
    status: "compensation_submitted",
    actionRunId: "action-dz-auto-1",
  });

  await expect(page.locator(".summary-card").filter({ hasText: "账户净值" })).toContainText("未知");
  await expect(page.locator(".runtime-problems")).toContainText("数据降级");
  await expect(page.locator(".risk-panel")).toContainText("权益占比数据待确认");

  const positionRow = page.locator(".positions-table tbody tr").filter({ hasText: "BTCUSDT" });
  await expect(positionRow).toHaveClass(/unknown-row/);
  await expect(positionRow).toContainText("来源");
  await expect(positionRow).toContainText("标记价数据待确认");
  await expect(positionRow).toContainText("强平距离不可用");
  await expect(positionRow).toContainText("资金费 数据待确认");
  await expect(positionRow).toContainText("结算时间数据待确认");
  const healthChip = positionRow.locator(".balance-evidence-chip").filter({ hasText: "来源" });
  await expect(healthChip).toHaveAttribute("title", /freshness 250ms/);
  await expect(healthChip).toHaveAttribute("title", /request req-dz-position-1/);

  const closeRun = page.locator(".close-runs-table tbody tr").filter({ hasText: "close-dz-durable-1" });
  await expect(closeRun).toContainText("补偿中");
  await expect(closeRun).toContainText("等待补偿最终结果");
  // CompensationSubmitted 状态下的文案（close_runs_panel/derive.rs）：
  // 快照无裸露仓位但补偿终态尚未确认。
  await expect(closeRun).toContainText("当前快照无裸露仓位，仍待补偿最终结果");
});
