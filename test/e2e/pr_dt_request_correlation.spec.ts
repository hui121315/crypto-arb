import { expect, test, type Page } from "@playwright/test";
import {
  LOCAL_FIXTURE_TOKEN,
  startRouteRuntimeFixture,
} from "./fixtures/route_runtime_policy.mjs";

let runtimeFixture: Awaited<ReturnType<typeof startRouteRuntimeFixture>>;

async function browserFetch(
  page: Page,
  baseUrl: string,
  path: string,
  init: RequestInit = {},
) {
  return await page.evaluate(async ({ baseUrl, init, path }) => {
    const response = await fetch(`${baseUrl}${path}`, init);
    return {
      body: await response.json(),
      headers: Object.fromEntries(response.headers.entries()),
      status: response.status,
    };
  }, { baseUrl, init, path });
}

test.describe("PR-DT request and audit correlation", () => {
  test.beforeAll(async () => {
    runtimeFixture = await startRouteRuntimeFixture();
  });

  test.afterAll(async () => {
    await runtimeFixture.close();
  });

  test.beforeEach(async () => {
    runtimeFixture.resetAuditEvents();
  });

  test("PR-DT auth 401 is typed, recoverable, and request-correlated", async ({ page }) => {
    await page.goto("/");
    const requestId = "pr-dt-auth-request";
    const response = await browserFetch(
      page,
      runtimeFixture.baseUrl,
      "/api/e2e/route-runtime-policy/metadata",
      { headers: { "x-request-id": requestId } },
    );

    expect(response.status).toBe(401);
    expect(response.headers["x-request-id"]).toBe(requestId);
    expect(response.headers["www-authenticate"]).toBe('Bearer realm="crossline-api"');
    expect(response.body.error).toEqual({
      code: "UNAUTHORIZED",
      details: { recoveryAction: "provide_valid_bearer_token" },
      message: "authentication required; provide a valid Bearer token and retry",
      requestId,
      source: "api.auth",
      status: 401,
    });
  });

  test("PR-BE auth-denied high-risk mutations keep typed audit correlation", async ({ page }) => {
    await page.goto("/");
    const attempts = [
      {
        action: "trading.kill_switch.set",
        actionKind: "trading_kill_switch",
        path: "/api/trading/kill-switch",
        requestId: "pr-be-kill-denied",
        resourceKind: "kill_switch",
      },
      {
        action: "venue_credentials.update",
        actionKind: "venue_credentials_update",
        path: "/api/exchanges/credentials",
        requestId: "pr-be-secret-denied",
        resourceKind: "venue_credentials",
      },
      {
        action: "hedge.confirm",
        actionKind: "hedge_confirm",
        path: "/api/arbitrage/opportunities/pr-be-denied/confirm",
        requestId: "pr-be-hedge-denied",
        resourceKind: "hedge_ticket",
      },
    ];

    for (const attempt of attempts) {
      const response = await browserFetch(page, runtimeFixture.baseUrl, attempt.path, {
        body: JSON.stringify({ fixture: true }),
        headers: {
          Authorization: "Bearer wrong-bearer-secret",
          "content-type": "application/json",
          "x-request-id": attempt.requestId,
        },
        method: "POST",
      });
      expect(response.status).toBe(401);
      expect(response.body.error).toMatchObject({
        code: "UNAUTHORIZED",
        requestId: attempt.requestId,
        status: 401,
      });
    }

    const audit = await browserFetch(
      page,
      runtimeFixture.baseUrl,
      "/api/e2e/route-runtime-policy/audit-events",
      { headers: { Authorization: `Bearer ${LOCAL_FIXTURE_TOKEN}` } },
    );

    expect(audit.status).toBe(200);
    expect(audit.body.events).toHaveLength(3);
    for (const [index, attempt] of attempts.entries()) {
      expect(audit.body.events[index]).toMatchObject({
        action: attempt.action,
        actionKind: attempt.actionKind,
        actor: "unknown",
        actorKind: "unknown",
        method: "POST",
        outcome: "denied",
        problemCode: "UNAUTHORIZED",
        requestId: attempt.requestId,
        resourceKind: attempt.resourceKind,
        status: 401,
      });
      expect(audit.body.events[index]).not.toHaveProperty("actionRunId");
    }
    expect(JSON.stringify(audit.body)).not.toContain("wrong-bearer-secret");
  });

  test("PR-DT audit pairs expose action, order, and execution run identities", async ({ page }) => {
    await page.goto("/");
    const requestId = "pr-dt-hedge-request";
    const idempotencyKey = "pr-dt-hedge-idempotency";
    const response = await browserFetch(
      page,
      runtimeFixture.baseUrl,
      "/api/arbitrage/opportunities/pr-dt-opportunity/confirm",
      {
        body: JSON.stringify({ previewId: "pr-dt-preview" }),
        headers: {
          Authorization: `Bearer ${LOCAL_FIXTURE_TOKEN}`,
          "content-type": "application/json",
          "idempotency-key": idempotencyKey,
          "x-request-id": requestId,
        },
        method: "POST",
      },
    );

    expect(response.status).toBe(200);
    expect(response.body).toMatchObject({
      actionRun: { id: "action-run-hedge-confirm", status: "succeeded" },
      executionRun: {
        longLeg: { orderIds: ["long-order-1"] },
        runId: "execution-run-1",
        shortLeg: { orderIds: ["short-order-1"] },
      },
    });

    const audit = await browserFetch(
      page,
      runtimeFixture.baseUrl,
      "/api/e2e/route-runtime-policy/audit-events",
      { headers: { Authorization: `Bearer ${LOCAL_FIXTURE_TOKEN}` } },
    );

    expect(audit.status).toBe(200);
    expect(audit.body.events).toHaveLength(2);
    for (const event of audit.body.events) {
      expect(event).toMatchObject({
        action: "hedge.confirm",
        actionRunId: "action-run-hedge-confirm",
        idempotencyKey,
        orderIds: ["long-order-1", "short-order-1"],
        requestId,
        runIds: ["execution-run-1"],
      });
    }
    expect(audit.body.events.map((event: { outcome: string }) => event.outcome)).toEqual([
      "accepted",
      "success",
    ]);
  });

  test("PR-DT LoadState keeps auth failure visible without executable fallback rows", async ({
    page,
  }) => {
    await page.route("**/api/v3/arbitrage/opportunities/list**", async (route) => {
      await route.fulfill({
        body: JSON.stringify({
          error: {
            code: "UNAUTHORIZED",
            details: { recoveryAction: "provide_valid_bearer_token" },
            message: "authentication required; provide a valid Bearer token and retry",
            requestId: "e2e-auth-401",
            source: "api.auth",
            status: 401,
          },
        }),
        headers: {
          "content-type": "application/json; charset=utf-8",
          "www-authenticate": 'Bearer realm="crossline-api"',
          "x-request-id": "e2e-auth-401",
        },
        status: 401,
      });
    });
    await page.goto("/#opportunities");

    const error = page
      .locator(".settings-message.is-error")
      .filter({ hasText: "机会快照冷启动失败" });
    await expect(error).toHaveCount(1);
    await expect(error).toContainText("provide a valid Bearer token");
    await expect(error).toContainText("HTTP 401");
    await expect(error).toContainText("request_id e2e-auth-401");
    await expect(page.locator(".empty-cell")).toContainText("机会快照错误");
    await expect(page.getByRole("button", { name: "构建对冲" })).toHaveCount(0);
  });
});
