import { expect, test, type Page } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";
const SCENARIO = "e2e-order-cancel-denied";

async function useExecutionScenario(page: Page) {
  await page.addInitScript(
    ({ apiBase, scenario }) => {
      window.localStorage.setItem("api_base", JSON.stringify(`${apiBase}/${scenario}`));
      window.localStorage.setItem("api_auth_token", JSON.stringify("e2e-token"));
    },
    { apiBase: API_BASE, scenario: SCENARIO },
  );
  await page.routeWebSocket("**/ws", (socket) => socket.close());
}

function preflightOutcome(
  operations: string[],
  overrides: Record<string, unknown> = {},
) {
  return {
    status: "passed",
    checkedAtMs: 1_780_000_000_000,
    scope: {
      venues: ["hyperliquid:km", "kucoin"],
      symbols: ["MU"],
      accountModes: [],
      operations,
    },
    observedVenues: ["hyperliquid:km", "kucoin"],
    source: "hedge_confirm_scoped_preflight",
    freshnessMs: 750,
    requestId: "req-pr-dv-scoped-preflight",
    problems: [],
    fieldQuality: [],
    rowHealth: [],
    ...overrides,
  };
}

function runtimeHealthRows() {
  return ["hyperliquid:km", "kucoin"].flatMap((venue) =>
    ["private_read", "order_write", "order_finality", "rest_orderbooks"].map(
      (operation) => ({
        subject: { kind: "account", venue },
        source: `venue_operation_health:${operation}:pr_dv_fixture`,
        observedAtMs: 1_780_000_000_000,
        freshnessMs: 750,
        lastSuccessMs: 1_780_000_000_000,
        requestId: `req-pr-dv-${venue}-${operation}`,
      }),
    ),
  );
}

function finalMarginHealth(venue: string) {
  return {
    status: "ready",
    source: "account_state.margin_facts",
    evidenceId: `margin_balance:${venue}:1780000001000`,
    observedAtMs: 1_780_000_001_000,
    requestId: "req-pr-cg-final-margin",
  };
}

async function injectScopedPreflight(page: Page) {
  await page.route("**/api/arbitrage/opportunities/mock-mu-perp/preview", async (route) => {
    const upstream = await route.fetch();
    const response = await upstream.json();
    response.ticket.guards.push(
      {
        key: "order_capability",
        label: "交易所下单能力",
        passed: true,
        detail: "通过",
        preflightOutcome: preflightOutcome(["capability"]),
      },
      {
        key: "account_mode",
        label: "账户模式数据依据",
        passed: true,
        detail: "通过",
        preflightOutcome: preflightOutcome(["account_mode"], {
          scope: {
            venues: ["kucoin"],
            symbols: ["MU"],
            accountModes: ["kucoin_position_mode:hedge·classic_futures"],
            operations: ["account_mode"],
          },
          observedVenues: ["kucoin"],
          source: "live_trading_adapter.get_exchange_account_mode",
          freshnessMs: 420,
        }),
      },
      {
        key: "order_write",
        label: "实盘下单准入",
        passed: true,
        detail: "通过",
        preflightOutcome: preflightOutcome(["order_write"]),
      },
      {
        key: "live_operation_health",
        label: "实盘运行状态数据依据",
        passed: true,
        detail: "通过",
        preflightOutcome: preflightOutcome([
          "private_read",
          "margin_balance",
          "order_write",
          "positions",
          "open_orders",
          "private_ws",
          "order_finality",
          "orderbook",
        ], { rowHealth: runtimeHealthRows() }),
      },
    );
    await route.fulfill({ json: response });
  });
}

async function openExecutionPreview(page: Page) {
  await page.goto("/#futures");
  await expect(page.getByRole("heading", { name: "期货套利" })).toBeVisible();
  await page.getByRole("button", { name: "构建对冲" }).click();
  await expect(page.getByRole("heading", { name: "对冲执行" })).toBeVisible();
}

test("PR-DV renders ticket-scoped capability, account, finality, and request evidence", async ({
  page,
}) => {
  await useExecutionScenario(page);
  await injectScopedPreflight(page);
  await openExecutionPreview(page);

  const ticket = page.locator(".risk-notes > div").filter({ hasText: "HedgeTicket" });
  const ticketEvidence = ticket.locator("strong");
  await expect(ticketEvidence).toContainText("双腿 hyperliquid:km / kucoin · 可用 2/2");
  await expect(ticketEvidence).toHaveAttribute("title", /source hedge_confirm_scoped_preflight/);
  await expect(ticketEvidence).toHaveAttribute("title", /freshness 750ms/);
  await expect(ticketEvidence).toHaveAttribute("title", /request req-pr-dv-scoped-preflight/);
  await expect(ticketEvidence).toHaveAttribute(
    "title",
    /venue_operation_health:order_finality:pr_dv_fixture/,
  );

  const runtime = page.locator(".check-item").filter({ hasText: "实盘运行状态数据依据" });
  await expect(runtime).toContainText("私有读取");
  await expect(runtime).toContainText("订单最终结果");
  await expect(runtime).toContainText("订单簿");
  await expect(runtime).toContainText("请求 req-pr-dv");

  const accountMode = page.locator(".check-item").filter({ hasText: "账户模式数据依据" });
  await expect(accountMode).toContainText("kucoin_position_mode:hedge·classic_futures");
  await expect(accountMode).toContainText("新鲜度 420ms");
});

test("PR-DV keeps confirm-time scoped preflight failure and correlation context visible", async ({
  page,
}) => {
  await useExecutionScenario(page);
  await page.route(
    "**/api/arbitrage/opportunities/mock-mu-perp/confirm",
    async (route) => {
      await route.fulfill({
        status: 400,
        contentType: "application/json",
        json: {
          error: {
            code: "HEDGE_PRE_TRADE_REJECTED",
            message:
              "confirm scoped preflight blocked: kucoin MU 账户模式不可读; kucoin MU 订单最终结果回查数据待确认",
            status: 400,
            source: "hedge_confirm_scoped_preflight",
            requestId: "req-pr-dv-confirm-blocked",
            retryAfterMs: 15_000,
            details: {
              confirmContext: {
                opportunityId: "mock-mu-perp",
                idempotencyKey: "preview-mu-001",
                ticketId: "ticket-mu-001",
                environment: "paper",
                longVenue: "hyperliquid:km",
                shortVenue: "kucoin",
              },
              guards: [
                {
                  key: "account_mode",
                  label: "账户模式数据依据",
                  passed: false,
                  detail: "kucoin MU 账户模式不可读",
                },
                {
                  key: "live_operation_health",
                  label: "实盘运行状态数据依据",
                  passed: false,
                  detail: "kucoin MU 订单最终结果回查数据待确认",
                },
              ],
            },
          },
        },
      });
    },
  );
  await openExecutionPreview(page);
  await page.getByRole("button", { name: "提交 模拟" }).click();

  const actionBar = page.locator(".execution-actionbar");
  await expect(actionBar.locator(".run-state > span")).toContainText("模拟提交失败");
  await expect(actionBar.locator(".run-state > em").first()).toContainText(
    "confirm scoped preflight blocked",
  );
  await expect(actionBar.locator(".run-state > em").first()).toContainText(
    "code HEDGE_PRE_TRADE_REJECTED",
  );
  await expect(actionBar.locator(".run-state > em").first()).toContainText(
    "request_id req-pr-dv-confirm-blocked",
  );
  await expect(actionBar.locator(".run-state > em").first()).toContainText("retry 15000ms");
  await expect(actionBar.locator(".confirm-context-detail")).toContainText("模拟");
  await expect(actionBar.locator(".confirm-context-detail")).toContainText(
    "Ticket ticket-mu-001",
  );
  await expect(actionBar.locator(".confirm-context-detail")).toContainText(
    "Idempotency preview-mu-001",
  );
});

test("PR-DV restores submitted legs without promoting acknowledgements to Hedged", async ({
  page,
}) => {
  await useExecutionScenario(page);
  let replayRun: Record<string, unknown> | undefined;
  let replayRequests = 0;
  await page.route("**/api/arbitrage/opportunities/mock-mu-perp/confirm", async (route) => {
    const upstream = await route.fetch();
    const response = await upstream.json();
    const evidence = response.executionRun.evidence ?? {};
    const ticketView = evidence.hedgeTicketView ?? {};
    const longLeg = response.executionRun.longLeg;
    const shortLeg = response.executionRun.shortLeg;
    replayRun = {
      ...response.executionRun,
      state: "second_leg_submitted",
      statusReason: "venue acknowledgements pending terminal fill evidence",
      finalityCheckedAtMs: 1_780_000_000_000,
      longLeg: { ...response.executionRun.longLeg, state: "accepted" },
      shortLeg: { ...response.executionRun.shortLeg, state: "accepted" },
      evidence: {
        schemaVersion: evidence.schemaVersion ?? 2,
        events: evidence.events ?? [],
        droppedEventCount: evidence.droppedEventCount ?? 0,
        longLeg: evidence.longLeg ?? { role: "long", finalityConfidence: "unknown" },
        shortLeg: evidence.shortLeg ?? { role: "short", finalityConfidence: "unknown" },
        ...evidence,
        hedgeTicketView: {
          ...ticketView,
          longLeg: {
            ...ticketView.longLeg,
            role: ticketView.longLeg?.role ?? longLeg.role,
            venue: ticketView.longLeg?.venue ?? longLeg.exchange,
            symbol: ticketView.longLeg?.symbol ?? longLeg.symbol,
            balance: finalMarginHealth("hyperliquid:km"),
          },
          shortLeg: {
            ...ticketView.shortLeg,
            role: ticketView.shortLeg?.role ?? shortLeg.role,
            venue: ticketView.shortLeg?.venue ?? shortLeg.exchange,
            symbol: ticketView.shortLeg?.symbol ?? shortLeg.symbol,
            balance: finalMarginHealth("kucoin"),
          },
        },
      },
    };
    response.executionRun = replayRun;
    await route.fulfill({ json: response });
  });
  await page.route("**/api/trading/execution-runs**", async (route) => {
    const upstream = await route.fetch();
    const response = await upstream.json();
    replayRequests += 1;
    response.rows = replayRun ? [replayRun] : [];
    response.page = {
      ...response.page,
      rowCount: response.rows.length,
      totalRows: response.rows.length,
    };
    await route.fulfill({ json: response });
  });

  await openExecutionPreview(page);
  await page.getByRole("button", { name: "提交 模拟" }).click();
  const status = page.locator(".execution-status-bar");
  await expect(status).toContainText("第二腿已提交，等待成交确认");
  await expect(status).not.toContainText("双腿完成");
  const workflow = page.getByTestId("hedge-workflow-status");
  await expect(workflow.locator('[data-health="balance"]')).toHaveCount(2);
  await expect(workflow.locator('[data-health="balance"]').first()).toHaveAttribute(
    "title",
    /request req-pr-cg-final-margin/,
  );
  const beforeReload = replayRequests;

  await page.reload();
  await expect(page.getByRole("heading", { name: "对冲执行" })).toBeVisible();
  await page.getByRole("button", { name: "切换到期货套利" }).click();
  await page.getByRole("button", { name: "构建对冲" }).click();

  await expect.poll(() => replayRequests).toBeGreaterThan(beforeReload);
  await expect(page.locator(".execution-status-bar")).toContainText(
    "第二腿已提交，等待成交确认",
  );
  await expect(page.locator(".execution-status-bar")).not.toContainText("双腿完成");
  await expect(
    page.getByTestId("hedge-workflow-status").locator('[data-health="balance"]').first(),
  ).toHaveAttribute("title", /request req-pr-cg-final-margin/);
});
