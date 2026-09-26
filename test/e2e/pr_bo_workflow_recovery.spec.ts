import { expect, test, type Page, type Route } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";
const WEB_BASE = process.env.CROSSLINE_E2E_WEB_BASE ?? "http://127.0.0.1:18080";
const SCENARIO = "order-cancel-denied";

const ids = {
  opportunity: "mock-mu-perp",
  ticket: "ticket-mu-001",
  run: "e2e-run-cancel-denied",
};

const corsHeaders = {
  "access-control-allow-origin": WEB_BASE,
  "access-control-allow-methods": "GET,POST,OPTIONS",
  "access-control-allow-headers": "authorization,accept,content-type,x-request-id",
  "content-type": "application/json; charset=utf-8",
  vary: "origin",
};

function health(kind: string, venue: string) {
  return {
    status: "ready",
    source: `pr_bo_${kind}_runtime`,
    evidenceId: `${kind}:${venue}:1784100000000`,
    observedAtMs: 1_784_100_000_000,
  };
}

function leg(role: "long" | "short", venue: string) {
  return {
    role,
    venue,
    symbol: "MU",
    market: health("market", venue),
    fee: health("fee", venue),
    balance: health("balance", venue),
    capability: health("capability", venue),
  };
}

function workflowView(phase?: "working" | "settled") {
  return {
    ticketId: ids.ticket,
    opportunityId: ids.opportunity,
    strategy: "perp_cross",
    symbol: "MU",
    expiresAtMs: Date.now() + 60_000,
    blockers: [],
    longLeg: leg("long", "hyperliquid:km"),
    shortLeg: leg("short", "kucoin"),
    ...(phase
      ? {
          executionRun: {
            key: {
              ticketId: ids.ticket,
              runId: ids.run,
              orderId: "e2e-run-order-1",
            },
            phase,
          },
        }
      : {}),
  };
}

function run(state: "second_leg_submitted" | "hedged", updatedAtMs: number) {
  const phase = state === "hedged" ? "settled" : "working";
  return {
    runId: ids.run,
    ticketId: ids.ticket,
    opportunityId: ids.opportunity,
    state,
    longLeg: runLeg("long", "hyperliquid:km", "e2e-run-order-1", state),
    shortLeg: runLeg("short", "kucoin", "e2e-run-order-2", state),
    netExposureUsd: 0,
    evidence: {
      schemaVersion: 2,
      events: [],
      droppedEventCount: 0,
      longLeg: { role: "long", finalityConfidence: "unknown" },
      shortLeg: { role: "short", finalityConfidence: "unknown" },
      hedgeTicketView: workflowView(phase),
    },
    recoveryAction: null,
    statusReason: state === "hedged" ? "双腿成交已确认" : "等待双腿最终结果确认",
    createdAtMs: updatedAtMs - 2_000,
    updatedAtMs,
  };
}

function runLeg(
  role: "long" | "short",
  exchange: string,
  orderId: string,
  state: "second_leg_submitted" | "hedged",
) {
  return {
    role,
    exchange,
    symbol: "MU",
    orderIds: [orderId],
    state: state === "hedged" ? "filled" : "accepted",
    targetQuantity: 1.13,
    filledQuantity: state === "hedged" ? 1.13 : null,
    targetNotionalUsd: 750,
    filledNotionalUsd: state === "hedged" ? 750 : null,
    filledFee: state === "hedged" ? 0.35 : null,
  };
}

function listEnvelope(rows: unknown[]) {
  return {
    rows,
    page: {
      limit: 32,
      maxLimit: 100,
      startOffset: 0,
      returnedCount: rows.length,
      totalRows: rows.length,
      hasMore: false,
      nextCursor: null,
    },
    status: "fresh",
    source: "pr_bo_execution_run_rest",
    observedAtMs: 1_784_100_000_000,
    problems: [],
  };
}

async function fulfillJson(route: Route, body: unknown) {
  if (route.request().method() === "OPTIONS") {
    await route.fulfill({ status: 204, headers: corsHeaders, body: "" });
    return;
  }
  await route.fulfill({ status: 200, headers: corsHeaders, body: JSON.stringify(body) });
}

async function useWorkflowRecoveryScenario(page: Page) {
  const restUpdatedAt = Date.now();
  await page.addInitScript(
    ({ apiBase, localView, context }) => {
      const setString = (key: string, value: string) =>
        window.localStorage.setItem(key, JSON.stringify(value));
      setString("api_base", apiBase);
      setString("api_auth_token", "e2e-token");
      setString("crossline.execution.runContext.opportunityId", context.opportunity);
      setString("crossline.execution.runContext.ticketId", context.ticket);
      setString("crossline.execution.runContext.runId", context.run);
      setString("crossline.execution.runContext.idempotencyKey", "pr-bo-idempotency");
      setString("crossline.execution.hedgeTicketView", JSON.stringify(localView));
    },
    {
      apiBase: `${API_BASE}/e2e-${SCENARIO}`,
      localView: workflowView(),
      context: ids,
    },
  );
  await page.route("**/api/trading/execution-runs**", async (route) => {
    await new Promise((resolve) => setTimeout(resolve, 1_500));
    await fulfillJson(route, listEnvelope([run("second_leg_submitted", restUpdatedAt)]));
  });
  let executionDeltaSent = false;
  await page.routeWebSocket("**/ws**", (socket) => {
    socket.onMessage((raw) => {
      const message = JSON.parse(raw.toString());
      if (message.type !== "subscribe") return;
      socket.send(
        JSON.stringify({
          type: "ack",
          subscribed: message.channels ?? [],
          requestId: message.requestId,
        }),
      );
      if (executionDeltaSent || !message.channels?.includes("execution")) return;
      executionDeltaSent = true;
      setTimeout(() => {
        socket.send(
          JSON.stringify({
            type: "message",
            channel: "execution",
            payload: {
              event: "execution_run_updated",
              executionRun: run("hedged", restUpdatedAt + 1_000),
              timestampMs: restUpdatedAt + 1_000,
            },
          }),
        );
      }, 3_500);
    });
  });
}

test("PR-BO restores ticket health, seeds REST, and applies execution WS delta", async ({ page }) => {
  await useWorkflowRecoveryScenario(page);
  await page.goto("/#execution");

  const workflow = page.getByTestId("hedge-workflow-status");
  const source = workflow.locator("[data-workflow-source]");
  await expect(workflow).toBeVisible();
  await expect(source).toHaveAttribute("data-workflow-source", "本地票据快照");
  await expect(workflow).toContainText(`${ids.ticket} · 预览`);

  for (const role of ["long", "short"] as const) {
    const row = workflow.locator(`[data-leg-role="${role}"]`);
    await expect(row).toBeVisible();
    for (const kind of ["market", "fee", "balance", "capability"] as const) {
      await expect(row.locator(`[data-health="${kind}"]`)).toHaveAttribute(
        "data-health-status",
        "ready",
      );
    }
  }

  await expect(source).toHaveAttribute("data-workflow-source", "REST 运行单快照");
  await expect(workflow).toContainText(`${ids.ticket} · ${ids.run} · 工作中`);

  await expect(source).toHaveAttribute("data-workflow-source", "WS 运行单增量", {
    timeout: 8_000,
  });
  await expect(workflow).toContainText(`${ids.ticket} · ${ids.run} · 已结算`);
});
