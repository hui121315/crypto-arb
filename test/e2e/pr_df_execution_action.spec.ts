import { expect, test, type Page } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";

async function useExecutionActionScenario(page: Page) {
  await page.addInitScript((apiBase) => {
    window.localStorage.setItem(
      "api_base",
      JSON.stringify(`${apiBase}/e2e-order-cancel-denied`),
    );
    window.localStorage.setItem("api_auth_token", JSON.stringify("e2e-token"));
  }, API_BASE);
  await page.routeWebSocket("**/ws", (socket) => socket.close());
}

test("PR-DF confirm preserves scoped runtime context and refreshes execution feeds", async ({
  page,
}) => {
  await useExecutionActionScenario(page);

  const requests = { orders: 0, preview: 0, runs: 0 };
  page.on("request", (request) => {
    if (request.method() !== "GET") return;
    const path = new URL(request.url()).pathname;
    if (path.endsWith("/api/trading/orders")) requests.orders += 1;
    if (path.endsWith("/api/trading/execution-runs")) requests.runs += 1;
    if (path.endsWith("/api/arbitrage/opportunities/mock-mu-perp/preview")) {
      requests.preview += 1;
    }
  });

  await page.route(
    "**/api/arbitrage/opportunities/mock-mu-perp/confirm",
    async (route) => {
      const upstream = await route.fetch();
      const response = await upstream.json();
      const context = {
        opportunityId: "mock-mu-perp",
        idempotencyKey: "preview-mu-001",
        ticketId: "ticket-mu-001",
        runId: "e2e-run-cancel-denied",
        environment: "paper",
        longVenue: "hyperliquid:km",
        shortVenue: "kucoin",
        longProblem: {
          code: "HEDGE_UNWIND_SUBMIT_FAILED",
          message: "long unwind route failed",
        },
        shortProblem: {
          code: "RATE_LIMITED",
          message: "short venue rate limited",
          status: 429,
        },
      };
      response.status = "hedge_broken_unwind_failed";
      response.context = context;
      response.executionRun = {
        ...response.executionRun,
        state: "unwind_required",
        netExposureUsd: 750,
        recoveryAction: "manual_review",
        statusReason: "short venue failed and long unwind submit failed",
        shortLeg: { ...response.executionRun.shortLeg, state: "failed" },
      };
      response.problem = {
        code: "HEDGE_UNWIND_SUBMIT_FAILED",
        message: "long unwind route failed",
        status: 409,
        source: "pr_df_execution_action",
        details: { confirmContext: context },
      };
      response.partialOutcome = {
        context,
        cause: "hedge_broken",
        originalStatus: "hedge_broken_unwind_failed",
        runId: "e2e-run-cancel-denied",
        runState: "unwind_required",
        netExposureUsd: 750,
        recoveryAction: "manual_review",
        primaryMessage: "short venue rate limited",
        primaryProblem: context.shortProblem,
        unwindStatus: "submit_failed",
        unwindTargetLeg: "long",
        unwindQuantity: 1.13,
        unwindProblem: context.longProblem,
        manualReviewRequired: true,
      };
      response.error = "long unwind route failed";
      await route.fulfill({ json: response });
    },
  );

  await page.goto("/#futures");
  await expect(page.getByRole("heading", { name: "期货套利" })).toBeVisible();
  await page.getByRole("button", { name: "构建对冲" }).click();
  await expect(page.locator(".confirm-action.primary")).toBeEnabled();

  const beforeConfirm = { ...requests };
  await page.getByRole("button", { name: "提交 模拟" }).click();

  const actionBar = page.locator(".execution-actionbar");
  await expect(actionBar.locator(".confirm-context-detail")).toContainText("模拟");
  await expect(actionBar.locator(".confirm-context-detail")).toContainText(
    "Ticket ticket-mu-001",
  );
  await expect(actionBar.locator(".confirm-context-detail")).toContainText(
    "Run e2e-run-cancel-denied",
  );
  await expect(actionBar.locator(".confirm-context-detail")).toContainText(
    "Idempotency preview-mu-001",
  );
  await expect(actionBar.locator(".confirm-context-detail")).toContainText(
    "long hyperliquid:km [HEDGE_UNWIND_SUBMIT_FAILED]",
  );
  await expect(actionBar.locator(".confirm-context-detail")).toContainText(
    "short kucoin [RATE_LIMITED]",
  );
  await expect(actionBar.locator(".confirm-outcome-detail")).toContainText("事故 第二腿失败");
  await expect(actionBar.locator(".confirm-outcome-detail")).toContainText("unwind 提交失败");
  await expect(actionBar.locator(".confirm-outcome-detail")).toContainText("需要人工复核");

  await expect.poll(() => requests.orders).toBeGreaterThan(beforeConfirm.orders);
  await expect.poll(() => requests.runs).toBeGreaterThan(beforeConfirm.runs);
  await page.waitForTimeout(250);
  expect(requests.preview).toBe(beforeConfirm.preview);
});
