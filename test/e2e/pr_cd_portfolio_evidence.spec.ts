import { expect, test, type Page } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";
const FIXTURE_TIME_MS = 1_784_400_000_000;

async function usePortfolioEvidenceScenario(page: Page) {
  await page.addInitScript((apiBase) => {
    window.localStorage.setItem("api_base", JSON.stringify(apiBase));
    window.localStorage.setItem("api_auth_token", JSON.stringify("e2e-token"));
  }, API_BASE);
  await page.routeWebSocket("**/ws", (socket) => socket.close());
  await page.route("**/api/trading/portfolio/snapshot", async (route) => {
    const upstream = await route.fetch();
    const envelope = await upstream.json();
    applyPortfolioEvidenceFixture(envelope);
    await route.fulfill({ json: envelope });
  });
}

function applyPortfolioEvidenceFixture(envelope: any) {
  const pnlProblem = problem(
    "PORTFOLIO_SNAPSHOT_DEGRADED",
    "portfolio PnL contains estimated or missing ledger fields",
    "trading_sql_realized_window+execution_ledger+close_runs",
    { operation: "portfolio_pnl", missingFields: ["fee", "funding", "net"] },
  );
  const cashProblem = problem(
    "ACCOUNT_FIELD_UNKNOWN",
    "cash is an estimated residual of wallet and position equity",
    "wallet_equity_minus_position_equity",
    { operation: "portfolio_nav_components", field: "cash" },
  );
  const rows = [
    position("binance", "BTCUSDT", 20),
    position("okx", "ETH-USDT-SWAP", 18),
    position("bybit", "SOLUSDT", null),
  ];
  const fieldQuality = [
    liquidationQuality("binance", "BTCUSDT", "actual", "binance_position_liquidation_distance"),
    liquidationQuality("okx", "ETH-USDT-SWAP", "estimated", "position_liquidation_price_derived_distance"),
    liquidationQuality("bybit", "SOLUSDT", "missing", "account_position_runtime"),
  ];
  const snapshot = envelope.snapshot;

  envelope.status = "degraded";
  envelope.source = "pr_cd_portfolio_evidence";
  envelope.observedAtMs = FIXTURE_TIME_MS;
  envelope.problem = pnlProblem;
  envelope.problems = [pnlProblem];
  envelope.retryAfterMs = null;
  snapshot.snapshotVersion = "pr-cd-portfolio-v1";
  snapshot.degraded = true;
  snapshot.summary = portfolioSummary(cashProblem, pnlProblem);
  snapshot.positions = rows;
  snapshot.problems = [{
    scope: "portfolio",
    operation: "portfolio_pnl",
    code: pnlProblem.code,
    message: pnlProblem.message,
    venue: null,
    retryAfterMs: null,
    observedAtMs: FIXTURE_TIME_MS,
  }];
  snapshot.accountState.fieldQuality = fieldQuality;
  snapshot.accountState.positions = {
    ...snapshot.accountState.positions,
    rows: rows.map(accountPosition),
    rowCount: rows.length,
    status: "degraded",
    source: "account_position_runtime",
    observedAtMs: FIXTURE_TIME_MS,
    problems: [],
    operationHealth: [],
    fieldQuality,
    rowHealth: [],
    accountBindings: [],
  };
}

function portfolioSummary(cashProblem: object, pnlProblem: object) {
  return {
    totalNavUsd: 12_000,
    navEvidence: {
      status: "actual",
      source: "account_state.account_summaries.total_equity_usd",
      observedAtMs: FIXTURE_TIME_MS,
      breakdown: {
        walletEquity: valueEvidence(12_000, "actual", "account_state.account_summaries.total_equity_usd"),
        positionEquity: valueEvidence(2_500, "actual", "account_state.positions.margin_plus_unrealized_pnl"),
        cash: valueEvidence(9_500, "estimated", "wallet_equity_minus_position_equity", cashProblem),
        unrealizedPnl: valueEvidence(-125, "actual", "account_state.positions.unrealized_pnl"),
      },
      coveredVenues: ["binance", "bybit", "okx"],
      missingVenues: [],
      problem: null,
    },
    navChange24hPct: 1.5,
    netDeltaUsd: 250,
    netDeltaPctOfNav: 2.08,
    nakedExposureUsd: 0,
    nakedPositionCount: 0,
    realizedPnlTodayUsd: -42,
    pnlBreakdown: {
      fundingUsd: -3,
      priceUsd: -34,
      feeRebateUsd: -5,
      evidence: {
        quality: "missing",
        source: "trading_sql_realized_window+execution_ledger+close_runs",
        observedAtMs: FIXTURE_TIME_MS,
        realizedGroupCount: 2,
        closeRunCount: 1,
        unwindRunCount: 1,
        actualFields: ["gross"],
        estimatedFields: ["slippage"],
        missingFields: ["fee", "funding", "net"],
        problem: pnlProblem,
      },
    },
    updatedAtMs: FIXTURE_TIME_MS,
  };
}

function valueEvidence(valueUsd: number, status: string, source: string, evidenceProblem: object | null = null) {
  return {
    valueUsd,
    status,
    source,
    observedAtMs: FIXTURE_TIME_MS,
    problem: evidenceProblem,
  };
}

function position(venue: string, symbol: string, distance: number | null) {
  const markPrice = venue === "bybit" ? 150 : 100;
  return {
    venue,
    symbol,
    side: "long",
    quantity: 1,
    entryPrice: markPrice - 1,
    markPrice,
    leverage: 5,
    unrealizedPnlUsd: venue === "bybit" ? -25 : -50,
    liquidationPrice: distance === null ? null : markPrice * (1 - distance / 100),
    liquidationDistancePct: distance,
    nextFundingMs: FIXTURE_TIME_MS + 3_600_000,
    fundingRate8h: 0.0001,
    fundingRateVerified: true,
    maintenanceMarginRatio: 0.01,
    pairEvidence: null,
    pairedWith: null,
    marginUsd: 500,
    severity: distance === null ? "unknown" : "ok",
    secondsUntilFunding: 3_600,
  };
}

function accountPosition(row: any) {
  return {
    symbol: row.symbol,
    exchange: row.venue,
    side: row.side,
    quantity: row.quantity,
    entryPrice: row.entryPrice,
    markPrice: row.markPrice,
    unrealizedPnl: row.unrealizedPnlUsd,
    leverage: row.leverage,
    liquidationPrice: row.liquidationPrice,
    liquidationDistancePct: row.liquidationDistancePct,
    nextFundingMs: row.nextFundingMs,
    pairedWith: null,
    margin: row.marginUsd,
    maintenanceMarginRatio: row.maintenanceMarginRatio,
    positionMode: "single",
    marginMode: "cross",
  };
}

function liquidationQuality(venue: string, symbol: string, status: string, source: string) {
  const evidenceProblem = status === "actual"
    ? null
    : problem(
      "POSITION_FIELD_UNAVAILABLE",
      status === "estimated"
        ? "liquidation distance is derived from mark and liquidation prices"
        : "liquidation distance is unavailable",
      source,
      { operation: "positions", venue, symbol, field: "liquidationDistancePct", status },
    );
  return {
    subject: { kind: "position", venue, symbol, side: "long" },
    field: "liquidationDistancePct",
    status,
    source,
    observedAtMs: FIXTURE_TIME_MS,
    problem: evidenceProblem,
  };
}

function problem(code: string, message: string, source: string, details: object) {
  return {
    code,
    message,
    status: 200,
    requestId: "req-cd-portfolio-1",
    retryAfterMs: null,
    source,
    details: { ...details, observedAtMs: FIXTURE_TIME_MS },
  };
}

test("PR-CD keeps NAV components PnL ledger quality and liquidation provenance visible", async ({ page }) => {
  await usePortfolioEvidenceScenario(page);
  const response = page.waitForResponse((item) =>
    item.url().includes("/api/trading/portfolio/snapshot") && item.status() === 200
  );

  await page.goto("/#positions");
  const envelope = await (await response).json();

  expect(envelope.snapshot.summary.navEvidence.breakdown.cash).toMatchObject({
    valueUsd: 9_500,
    status: "estimated",
    source: "wallet_equity_minus_position_equity",
  });
  expect(envelope.snapshot.summary.pnlBreakdown.evidence).toMatchObject({
    quality: "missing",
    closeRunCount: 1,
    unwindRunCount: 1,
    missingFields: ["fee", "funding", "net"],
  });

  const breakdown = page.locator(".nav-breakdown");
  await expect(breakdown).toContainText("钱包权益");
  await expect(breakdown).toContainText("持仓权益");
  await expect(breakdown).toContainText("现金残差");
  await expect(breakdown).toContainText("估算");
  await expect(breakdown).toContainText("未实现 PnL");

  const pnlCard = page.locator(".summary-card").filter({ hasText: "当日已实现 PnL" });
  await expect(pnlCard).toContainText("-$42");
  await expect(pnlCard).toContainText("缺失 费用/资金费/净额");
  await expect(pnlCard).toContainText("SQL 账本");
  await expect(pnlCard).toContainText("1 次补偿");

  await expect(positionRow(page, "BTCUSDT")).toContainText("交易所距离");
  await expect(positionRow(page, "ETH-USDT-SWAP")).toContainText("估算距离");
  await expect(positionRow(page, "SOLUSDT")).toContainText("强平距离不可用");
});

function positionRow(page: Page, symbol: string) {
  return page.locator(".positions-table tbody tr").filter({ hasText: symbol });
}
