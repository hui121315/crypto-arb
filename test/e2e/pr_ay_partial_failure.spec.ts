import { expect, test, type APIRequestContext, type Page } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";
const WEB_BASE = process.env.CROSSLINE_E2E_WEB_BASE ?? "http://127.0.0.1:18080";
const SYSTEM_SCENARIO = "e2e-system-502";
const PORTFOLIO_SCENARIO = "e2e-portfolio-snapshot-502";

function responseHeaders(requestId: string) {
  return {
    "access-control-allow-origin": WEB_BASE,
    "access-control-expose-headers": "retry-after,x-request-id",
    "content-type": "application/json; charset=utf-8",
    "retry-after": "3",
    "x-request-id": requestId,
  };
}

function typedProblem(code: string, message: string, requestId: string, source: string) {
  return {
    code,
    message,
    status: 502,
    requestId,
    retryAfterMs: 3_000,
    source,
    recoveryAction: "check_runtime_health",
    details: {
      venue: "gate",
      operation: "positions",
      method: "GET",
      path: "/api/v4/futures/usdt/positions",
    },
  };
}

async function useScenarioApiBase(page: Page, scenario: string) {
  await page.addInitScript(
    ({ apiBase, scenario }) => {
      window.localStorage.setItem("api_base", JSON.stringify(`${apiBase}/${scenario}`));
      window.localStorage.setItem("api_auth_token", JSON.stringify("e2e-token"));
    },
    { apiBase: API_BASE, scenario },
  );
}

async function baselineJson(request: APIRequestContext, path: string) {
  const response = await request.get(`${API_BASE}${path}`, { timeout: 5_000 });
  expect(response.ok()).toBeTruthy();
  return response.json();
}

test("PR-AY degraded system envelope keeps usable scalar data isolated", async ({
  page,
}) => {
  const requestId = "req-pr-ay-system-gate";
  const problem = typedProblem(
    "GATE_POSITION_PARSE",
    "Gate position payload changed; other venues remain available",
    requestId,
    "gate.GET /api/v4/futures/usdt/positions",
  );
  const envelope = {
    data: {
      apiVersion: "0.1.0",
      api: { healthy: 7, total: 8, failedVenues: ["gate"] },
      ws: { channels: 8, disconnected: [] },
      orderElapsedMs: 24,
      risk: "warn",
      netDeltaUsd: 12.5,
      netDeltaPctOfNav: 0.12,
      nextFunding: null,
      updatedAtMs: 1_770_000_000_000,
      degraded: true,
      problems: [
        {
          scope: "trading_api",
          operation: "positions",
          code: problem.code,
          message: problem.message,
          venue: "gate",
          retryAfterMs: problem.retryAfterMs,
          problem,
          observedAtMs: 1_770_000_000_000,
        },
      ],
    },
    status: "degraded",
    source: "system-health-snapshot",
    observedAtMs: 1_770_000_000_000,
    problems: [problem],
  };

  await useScenarioApiBase(page, SYSTEM_SCENARIO);
  await page.route(`**/${SYSTEM_SCENARIO}/api/system/health`, async (route) => {
    await route.fulfill({
      status: 200,
      headers: responseHeaders(requestId),
      body: JSON.stringify(envelope),
    });
  });
  const response = page.waitForResponse((candidate) =>
    candidate.url().includes(`/${SYSTEM_SCENARIO}/api/system/health`)
      && candidate.status() === 200
  );

  await page.goto("/#futures");
  await expect(page.getByRole("heading", { name: "期货套利" })).toBeVisible();
  await response;

  const orderElapsed = page.getByTestId("status-order-elapsed");
  await expect(orderElapsed).not.toHaveClass(/degraded/);
  await expect(orderElapsed).toContainText("24ms");
  await expect(orderElapsed).toHaveAttribute(
    "title",
    "订单终态耗时：从创建到 Filled/Cancelled/Rejected/Failed 的平均耗时（OrderRecord updated_at - created_at）；包含交易所处理、重试与本地状态推进，不代表网络 RTT",
  );
  await expect(orderElapsed).not.toHaveAttribute("title", new RegExp(`request_id ${requestId}`));
});

test("PR-AY portfolio account and market fanout isolate one venue failure", async ({
  page,
  request,
}) => {
  const requestId = "req-pr-ay-gate-partial";
  const problem = typedProblem(
    "GATE_POSITION_PARSE",
    "Gate positions failed; balances and other venues remain available",
    requestId,
    "gate.GET /api/v4/futures/usdt/positions",
  );
  const portfolio = await baselineJson(request, "/api/trading/portfolio/snapshot");
  portfolio.status = "degraded";
  portfolio.source = "pr-ay-partial-failure";
  portfolio.problem = problem;
  portfolio.problems = [problem];
  portfolio.retryAfterMs = problem.retryAfterMs;
  portfolio.snapshot.degraded = true;
  portfolio.snapshot.problems = [
    {
      scope: "portfolio",
      operation: "positions",
      code: problem.code,
      message: problem.message,
      venue: "gate",
      retryAfterMs: problem.retryAfterMs,
      problem,
      observedAtMs: portfolio.observedAtMs,
    },
  ];
  portfolio.snapshot.accountState.status = "degraded";
  portfolio.snapshot.accountState.problems = [problem];
  portfolio.snapshot.accountState.positions.status = "degraded";
  portfolio.snapshot.accountState.positions.problems = [problem];
  const accountState = portfolio.snapshot.accountState;

  const funding = await baselineJson(request, "/api/arbitrage/funding-rates");
  const marketProblem = {
    ...problem,
    code: "MARKET_DATA_RATE_LIMITED",
    message: "Gate funding refresh rate limited; cached rows retained",
    details: {
      ...problem.details,
      operation: "funding_rates",
      path: "/api/v4/futures/usdt/funding_rate",
    },
  };
  funding.health = {
    ...funding.health,
    quality: "stale_allowed",
    source: "rest_fallback",
    retryAfterMs: marketProblem.retryAfterMs,
    lastError: marketProblem.message,
    problem: marketProblem,
  };
  funding.retryAfterMs = marketProblem.retryAfterMs;
  funding.fanout = [
    {
      venue: "gate",
      operation: "funding_rates",
      health: {
        ...funding.health,
        quality: "rate_limited",
      },
    },
  ];

  await useScenarioApiBase(page, PORTFOLIO_SCENARIO);
  await page.route(`**/${PORTFOLIO_SCENARIO}/api/trading/portfolio/snapshot`, async (route) => {
    await route.fulfill({
      status: 200,
      headers: responseHeaders(requestId),
      body: JSON.stringify(portfolio),
    });
  });
  await page.route(`**/${PORTFOLIO_SCENARIO}/api/trading/account-state`, async (route) => {
    await route.fulfill({
      status: 200,
      headers: responseHeaders(requestId),
      body: JSON.stringify(accountState),
    });
  });
  await page.route(`**/${PORTFOLIO_SCENARIO}/api/arbitrage/funding-rates`, async (route) => {
    await route.fulfill({
      status: 200,
      headers: responseHeaders(requestId),
      body: JSON.stringify(funding),
    });
  });
  const portfolioResponse = page.waitForResponse((candidate) =>
    candidate.url().includes(`/${PORTFOLIO_SCENARIO}/api/trading/portfolio/snapshot`)
      && candidate.status() === 200
  );

  await page.goto("/#positions");
  await expect(page.getByRole("heading", { name: "持仓/风控" })).toBeVisible();
  await portfolioResponse;

  const banner = page.locator(".runtime-problems");
  await expect(banner).toBeVisible();
  await expect(banner).toContainText("gate · portfolio/positions");
  await expect(banner).toHaveAttribute("title", new RegExp(`request_id ${requestId}`));
  await expect(banner).toHaveAttribute(
    "title",
    /source gate.GET \/api\/v4\/futures\/usdt\/positions/,
  );
  await expect(page.locator(".balance-row").filter({ hasText: "mock" })).toHaveCount(1);
  await expect(page.getByText("暂无可用余额")).toHaveCount(0);

  const partialContracts = await page.evaluate(async () => {
    const apiBase = JSON.parse(window.localStorage.getItem("api_base") ?? '""');
    const headers = { Authorization: "Bearer e2e-token" };
    const [accountResponse, fundingResponse] = await Promise.all([
      fetch(`${apiBase}/api/trading/account-state`, { headers }),
      fetch(`${apiBase}/api/arbitrage/funding-rates`, { headers }),
    ]);
    return {
      accountStatus: accountResponse.status,
      account: await accountResponse.json(),
      fundingStatus: fundingResponse.status,
      funding: await fundingResponse.json(),
    };
  });

  expect(partialContracts.accountStatus).toBe(200);
  expect(partialContracts.account).toMatchObject({
    status: "degraded",
    balances: { status: "fresh", rowCount: 1 },
    positions: { status: "degraded" },
    problems: [expect.objectContaining({ requestId })],
  });
  expect(partialContracts.fundingStatus).toBe(200);
  expect(partialContracts.funding.data.length).toBeGreaterThan(0);
  expect(partialContracts.funding.fanout).toEqual([
    expect.objectContaining({
      venue: "gate",
      operation: "funding_rates",
      health: expect.objectContaining({
        quality: "rate_limited",
        problem: expect.objectContaining({ requestId }),
      }),
    }),
  ]);
});
