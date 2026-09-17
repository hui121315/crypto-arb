import { expect, test, type Page } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";
const WEB_BASE = process.env.CROSSLINE_E2E_WEB_BASE ?? "http://127.0.0.1:18080";
const SETTINGS_SCENARIO = "e2e-settings-credential-static-adapter-boundary";
const EXECUTION_SCENARIO = "e2e-order-cancel-denied";

type RequestEvidence = {
  requestId: string;
  idempotencyKey: string;
};

async function useScenario(page: Page, scenario: string) {
  await page.addInitScript(
    ({ apiBase, scenario }) => {
      window.localStorage.setItem("api_base", JSON.stringify(`${apiBase}/${scenario}`));
      window.localStorage.setItem("api_auth_token", JSON.stringify("e2e-token"));
    },
    { apiBase: API_BASE, scenario },
  );
  await page.routeWebSocket("**/ws", (socket) => socket.close());
}

function responseHeaders(requestId: string) {
  return {
    "access-control-allow-origin": WEB_BASE,
    "access-control-allow-methods": "GET,POST,PATCH,OPTIONS",
    "access-control-allow-headers":
      "content-type,authorization,accept,x-request-id,idempotency-key",
    "access-control-expose-headers": "x-request-id",
    "content-type": "application/json; charset=utf-8",
    "x-request-id": requestId,
    vary: "origin",
  };
}

function credentialResponse(requestId: string) {
  return {
    venue: "okx",
    label: "OKX",
    configuredCount: 1,
    fieldCount: 6,
    message: "PR-CK credential evidence persisted",
    secretStorage: {
      mode: "runtime_only",
      persistent: false,
      encrypted: false,
      atomicWrite: false,
      path: null,
      label: "仅进程内缓存",
      message: "fixture runtime store",
      warning: "fixture only",
    },
    actionRunId: "action-pr-ck-save",
    requestId,
  };
}

function actionRunsEnvelope() {
  return {
    data: [{
      id: "action-pr-ck-restored",
      kind: "venue_credentials_update",
      status: "succeeded",
      actor: "e2e-operator",
      target: "okx",
      requestId: "req-pr-ck-restored",
      idempotencyKey: "idem-pr-ck-restored",
      message: "PR-CK restored credential action",
      startedAtMs: 1_789_000_000_000,
      updatedAtMs: 1_789_000_000_100,
    }],
    status: "ready",
    source: "action-run-registry",
    observedAtMs: 1_789_000_000_100,
    coverage: {
      expected: 1,
      observed: 1,
      coveragePct: 1,
      truncated: false,
    },
    problems: [],
  };
}

test("PR-CK pending evidence uses the exact mutation request headers", async ({ page }) => {
  await useScenario(page, SETTINGS_SCENARIO);
  let releaseResponse: (() => void) | undefined;
  const responseGate = new Promise<void>((resolve) => {
    releaseResponse = resolve;
  });
  let captured: RequestEvidence | undefined;
  await page.route(`**/${SETTINGS_SCENARIO}/api/trading/action-runs`, async (route) => {
    await route.fulfill({ json: { ...actionRunsEnvelope(), data: [] } });
  });
  await page.route(`**/${SETTINGS_SCENARIO}/api/exchanges/credentials`, async (route) => {
    if (route.request().method() !== "POST") {
      await route.continue();
      return;
    }
    const headers = route.request().headers();
    captured = {
      requestId: headers["x-request-id"] ?? "",
      idempotencyKey: headers["idempotency-key"] ?? "",
    };
    await responseGate;
    await route.fulfill({
      status: 200,
      headers: responseHeaders(captured.requestId),
      body: JSON.stringify(credentialResponse(captured.requestId)),
    });
  });

  await page.goto("/#settings");
  await page.getByLabel("API Secret", { exact: true }).fill("pr-ck-secret");
  await page.getByRole("button", { name: "保存字段" }).click();
  await expect.poll(() => captured?.requestId ?? "").not.toBe("");
  expect(captured?.idempotencyKey).not.toBe("");

  const message = page.locator(".credential-editor-actions em");
  await expect(message).toContainText(`request_id ${captured?.requestId}`);
  await expect(message).toContainText(`idempotency ${captured?.idempotencyKey}`);

  releaseResponse?.();
  await expect(message).toContainText("PR-CK credential evidence persisted");
  await expect(message).toContainText(`request_id ${captured?.requestId}`);
  await expect(page.locator("body")).not.toContainText("pr-ck-secret");
});

test("PR-CK restores structured ActionRun evidence after settings unmount", async ({ page }) => {
  await useScenario(page, SETTINGS_SCENARIO);
  await page.route(`**/${SETTINGS_SCENARIO}/api/trading/action-runs`, async (route) => {
    await route.fulfill({ json: actionRunsEnvelope() });
  });

  await page.goto("/#settings");
  const message = page.locator(".credential-editor-actions em");
  await expect(message).toContainText("PR-CK restored credential action");
  await expect(message).toContainText("request_id req-pr-ck-restored");
  await expect(message).toContainText("action_run_id action-pr-ck-restored");
  await expect(message).toContainText("idempotency idem-pr-ck-restored");

  await page.getByRole("tab", { name: "诊断", exact: true }).click();
  await expect(page.locator('[data-settings-table="execution-environment"]')).toBeVisible();
  await page.getByRole("tab", { name: "凭证", exact: true }).click();

  await expect(message).toContainText("PR-CK restored credential action");
  await expect(message).toContainText("request_id req-pr-ck-restored");
  await expect(page.getByRole("button", { name: /readiness|旧准入/i })).toHaveCount(0);
});

test("PR-CK execution pending state exposes request idempotency and client order ids", async ({
  page,
}) => {
  await useScenario(page, EXECUTION_SCENARIO);
  let releaseResponse: (() => void) | undefined;
  const responseGate = new Promise<void>((resolve) => {
    releaseResponse = resolve;
  });
  let captured: RequestEvidence | undefined;
  await page.route("**/api/arbitrage/opportunities/mock-mu-perp/preview", async (route) => {
    const upstream = await route.fetch();
    const response = await upstream.json();
    const nowMs = await page.evaluate(() => Date.now());
    response.ticket.createdAtMs = nowMs;
    response.ticket.expiresAtMs = nowMs + 60_000;
    await route.fulfill({ json: response });
  });
  await page.route("**/api/trading/execution-runs**", async (route) => {
    const upstream = await route.fetch();
    const response = await upstream.json();
    response.rows = [];
    response.page = {
      ...response.page,
      returnedCount: 0,
      totalRows: 0,
      hasMore: false,
      nextCursor: null,
    };
    await route.fulfill({ json: response });
  });
  await page.route("**/api/arbitrage/opportunities/mock-mu-perp/confirm", async (route) => {
    const headers = route.request().headers();
    captured = {
      requestId: headers["x-request-id"] ?? "",
      idempotencyKey: headers["idempotency-key"] ?? "",
    };
    const upstream = await route.fetch();
    await responseGate;
    await route.fulfill({ response: upstream });
  });

  await page.goto("/#futures");
  await page.getByRole("button", { name: "构建对冲" }).click();
  await expect(page.locator(".confirm-action.primary")).toBeEnabled();
  await page.getByRole("button", { name: "提交 模拟" }).click();
  await expect.poll(() => captured?.requestId ?? "").not.toBe("");
  expect(captured?.idempotencyKey).not.toBe("");

  const detail = page.locator(".execution-actionbar .run-state em").first();
  await expect(detail).toContainText(`request_id ${captured?.requestId}`);
  await expect(detail).toContainText(`idempotency ${captured?.idempotencyKey}`);
  await expect(detail).toContainText("client_order_id");

  releaseResponse?.();
  await expect(detail).toContainText("run_id");
});
