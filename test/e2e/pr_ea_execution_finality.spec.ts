import { expect, test, type Page } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";
const SCENARIO = "e2e-order-cancel-denied";

type CompilePlan = Record<string, unknown> & {
  exchange: string;
  symbol: string;
};

type ExecutionRun = Record<string, unknown> & {
  runId: string;
};

function attachExecutionEvidence(plan: CompilePlan, role: "long" | "short") {
  const nativeSymbol = role === "long" ? "MU-PERP-HL" : "MUSDTM";
  plan.instrumentSpec = {
    venue: plan.exchange,
    nativeSymbol,
    canonicalSymbol: "MU",
    displaySymbol: "MU Perp",
    assetClass: "crypto",
    productType: "perp",
    quoteAsset: "USDT",
    settleAsset: "USDT",
    marginAsset: "USDT",
    contractSize: 0.001,
    executionSupported: true,
    priceTick: 0.01,
    qtyStep: 1,
    minQty: 1,
    listingStatus: "trading",
    source: "official_endpoint",
    checkedAtMs: 1_770_000_000_000,
    schemaVersion: `${role}-instrument-v1`,
  };
  plan.venueCapability = {
    venue: plan.exchange,
    symbol: nativeSymbol,
    product: "perp",
    source: "ticket_bound_capability_registry",
  };
}

function timelineEvent(
  eventId: string,
  kind: string,
  state: string,
  source: string,
  message: string,
  requestId: string,
  overrides: Record<string, unknown> = {},
) {
  return {
    eventId,
    kind,
    state,
    source,
    message,
    occurredAtMs: 1_770_000_000_000,
    requestId,
    finalityConfidence: "unknown",
    ...overrides,
  };
}

function enrichRun(
  source: Record<string, unknown>,
  longPlan: CompilePlan,
  shortPlan: CompilePlan,
): ExecutionRun {
  return {
    ...source,
    state: "second_leg_submitted",
    statusReason: "venue acknowledgements retained while terminal fill evidence is pending",
    finalityCheckedAtMs: 1_770_000_000_000,
    longLeg: {
      ...(source.longLeg as Record<string, unknown>),
      state: "accepted",
      finalitySource: "adapter_ack",
    },
    shortLeg: {
      ...(source.shortLeg as Record<string, unknown>),
      state: "accepted",
      finalitySource: "order_query",
    },
    evidence: {
      schemaVersion: 1,
      requestId: "req-pr-ea-run",
      droppedEventCount: 2,
      lastReconciledAtMs: 1_770_000_000_000,
      longLeg: {
        role: "long",
        compilePlan: longPlan,
        finalityConfidence: "adapter_ack",
        lastFinalityEventId: "pr-ea-long-ack",
      },
      shortLeg: {
        role: "short",
        compilePlan: shortPlan,
        finalityConfidence: "order_query",
        lastFinalityEventId: "pr-ea-short-query",
      },
      events: [
        timelineEvent(
          "pr-ea-preview",
          "preview",
          "previewed",
          "internal",
          "ticket preview evidence bound",
          "req-pr-ea-run",
        ),
        timelineEvent(
          "pr-ea-long-ack",
          "submit",
          "submitting_second_leg",
          "adapter_ack",
          "long venue acknowledged order",
          "req-pr-ea-submit",
          {
            legRole: "long",
            ledgerEventType: "order_state",
            finalityConfidence: "adapter_ack",
            orderIdentity: {
              internalOrderId: "internal-long-1",
              publicClientOrderId: "public-long-1",
              exchangeOrderId: "venue-long-1",
            },
          },
        ),
        timelineEvent(
          "pr-ea-short-query",
          "reconcile",
          "second_leg_submitted",
          "order_query",
          "short order remains accepted; fill finality pending",
          "req-pr-ea-reconcile",
          {
            legRole: "short",
            ledgerEventType: "order_state",
            finalityConfidence: "order_query",
            orderIdentity: {
              internalOrderId: "internal-short-1",
              publicClientOrderId: "public-short-1",
              exchangeOrderId: "venue-short-1",
            },
          },
        ),
      ],
    },
  } as ExecutionRun;
}

async function useExecutionFinalityScenario(page: Page) {
  let replayRun: ExecutionRun | undefined;
  let compilePlans: { long?: CompilePlan; short?: CompilePlan } = {};
  let replayRequests = 0;

  await page.addInitScript(
    ({ apiBase, scenario }) => {
      window.localStorage.setItem("api_base", JSON.stringify(`${apiBase}/${scenario}`));
      window.localStorage.setItem("api_auth_token", JSON.stringify("e2e-token"));
    },
    { apiBase: API_BASE, scenario: SCENARIO },
  );
  await page.routeWebSocket("**/ws", (socket) => socket.close());
  await page.route("**/api/arbitrage/opportunities/mock-mu-perp/preview", async (route) => {
    const upstream = await route.fetch();
    const response = await upstream.json();
    const longPlan = response.ticketOrderPlans.long.compilePlan as CompilePlan;
    const shortPlan = response.ticketOrderPlans.short.compilePlan as CompilePlan;
    attachExecutionEvidence(longPlan, "long");
    attachExecutionEvidence(shortPlan, "short");
    compilePlans = { long: longPlan, short: shortPlan };
    await route.fulfill({ json: response });
  });
  await page.route("**/api/arbitrage/opportunities/mock-mu-perp/confirm", async (route) => {
    const upstream = await route.fetch();
    const response = await upstream.json();
    if (!compilePlans.long || !compilePlans.short || !response.executionRun) {
      throw new Error("PR-EA fixture requires preview compile plans and an execution run");
    }
    replayRun = enrichRun(response.executionRun, compilePlans.long, compilePlans.short);
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

  return { replayRequests: () => replayRequests };
}

async function submitExecution(page: Page) {
  await page.goto("/#futures");
  await expect(page.getByRole("heading", { name: "期货套利" })).toBeVisible();
  await page.getByRole("button", { name: "构建对冲" }).click();
  await expect(page.locator(".confirm-action.primary")).toBeEnabled();
  await page.getByRole("button", { name: "提交 模拟" }).click();
}

test("PR-EA renders ticket evidence and replayable finality timeline", async ({ page }) => {
  const requests = await useExecutionFinalityScenario(page);
  await submitExecution(page);

  const status = page.locator(".execution-status-bar");
  await expect(status).toContainText("第二腿已提交，等待成交确认");
  await expect(status).toContainText("等待私有 WS 或订单回查确认双腿成交");
  await expect(status).not.toContainText("双腿完成");
  await expect(status).toContainText("native MU-PERP-HL");
  await expect(status).toContainText("精度 tick 0.01 / step 1 / contract 0.001");
  await expect(status).toContainText("能力 ticket_bound_capability_registry");

  const timeline = status.locator(".execution-timeline");
  await expect(timeline).toContainText("执行时间线");
  await expect(timeline).toContainText("3 条 · 已归档 2 条");
  await expect(timeline).toContainText("多腿 提交");
  await expect(timeline).toContainText("ACK · 订单状态 · 置信 ACK");
  await expect(timeline).toContainText("订单 venue-long-1");
  await expect(timeline).toContainText("request_id req-pr-ea-submit");
  await expect(timeline).toContainText("空腿 终态回查");
  await expect(timeline).toContainText("查询 · 订单状态 · 置信 订单查询");
  await expect.poll(requests.replayRequests).toBeGreaterThan(0);
});

test("PR-EA restores the same run timeline from scoped REST after reload", async ({ page }) => {
  const requests = await useExecutionFinalityScenario(page);
  await submitExecution(page);
  await expect(page.locator(".execution-timeline")).toContainText("req-pr-ea-reconcile");
  const beforeReload = requests.replayRequests();

  await page.reload();
  await expect(page.getByRole("heading", { name: "对冲执行" })).toBeVisible();
  await page.getByRole("button", { name: "切换到期货套利" }).click();
  await expect(page.getByRole("heading", { name: "期货套利" })).toBeVisible();
  await page.getByRole("button", { name: "构建对冲" }).click();

  await expect.poll(requests.replayRequests).toBeGreaterThan(beforeReload);
  await expect(page.locator(".execution-timeline")).toContainText("req-pr-ea-reconcile");
  await expect(page.locator(".execution-status-bar")).toContainText(
    "第二腿已提交，等待成交确认",
  );
  await expect(page.locator(".execution-status-bar")).not.toContainText("双腿完成");
});
