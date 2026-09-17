import { expect, test, type Page } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";
const WEB_BASE = process.env.CROSSLINE_E2E_WEB_BASE ?? "http://127.0.0.1:18080";

async function useScenario(page: Page, scenario: string) {
  await page.addInitScript(
    ({ apiBase, scenario }) => {
      window.localStorage.setItem("api_base", JSON.stringify(`${apiBase}/${scenario}`));
      window.localStorage.setItem("api_auth_token", JSON.stringify("e2e-token"));
    },
    { apiBase: API_BASE, scenario },
  );
}

function responseHeaders(requestId: string, retryAfterSeconds?: number) {
  const headers: Record<string, string> = {
    "access-control-allow-origin": WEB_BASE,
    "access-control-allow-methods": "GET,POST,OPTIONS",
    "access-control-allow-headers":
      "content-type,authorization,accept,x-request-id,idempotency-key",
    "access-control-expose-headers": "retry-after,x-request-id",
    "content-type": "application/json; charset=utf-8",
    "x-request-id": requestId,
    vary: "origin",
  };
  if (retryAfterSeconds !== undefined) {
    headers["retry-after"] = String(retryAfterSeconds);
  }
  return headers;
}

function successfulCredentialUpdate() {
  return {
    venue: "okx",
    label: "OKX",
    configuredCount: 1,
    fieldCount: 6,
    message: "credential evidence persisted",
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
    validationEvidence: {
      status: "unknown",
      checkedAtMs: 1_789_000_000_000,
      probes: [{
        kind: "order_permission",
        status: "unknown",
        scope: "place_cancel_order_stream",
        source: "safe_noop_probe",
        message: "live write remains unproven",
        checkedAtMs: 1_789_000_000_000,
        requestId: "req-du-probe",
      }],
    },
    actionRunId: "action-du-credential-save",
    requestId: "req-du-credential-save",
  };
}

test("PR-DU refreshes the selected venue credential and runtime evidence together", async ({
  page,
}) => {
  await useScenario(
    page,
    "e2e-settings-selected-venue-trading-runtime-all-ok-local-capture-shaped",
  );
  let credentialReads = 0;
  let operationReads = 0;
  page.on("request", (request) => {
    if (request.method() !== "GET") return;
    const path = new URL(request.url()).pathname;
    if (path.endsWith("/api/exchanges/credentials")) credentialReads += 1;
    if (path.endsWith("/api/system/venue-operation-health")) operationReads += 1;
  });

  await page.goto("/#settings");
  await expect(page.getByLabel("交易所")).toHaveValue("okx");
  await expect.poll(() => credentialReads).toBeGreaterThan(0);
  await expect.poll(() => operationReads).toBeGreaterThan(0);
  const initialCredentialReads = credentialReads;
  const initialOperationReads = operationReads;

  const runtime = page.locator(
    '.runtime-health-panel:has(> .runtime-health-head strong:text-is("交易运行证据"))',
  );
  await expect(runtime).toContainText("当前可用");
  await expect(runtime).toContainText("request_id req-settings-runtime-all-ok-order-permission");

  await page.getByRole("button", { name: "刷新当前证据" }).click();

  await expect.poll(() => credentialReads).toBeGreaterThan(initialCredentialReads);
  await expect.poll(() => operationReads).toBeGreaterThan(initialOperationReads);
  await expect(page.locator(".credential-editor-actions em"))
    .toContainText("已请求刷新当前交易所证据");
  await expect(runtime).toContainText("写单运行态");
  await expect(runtime).toContainText("私有订单流");
  await expect(runtime).toContainText("订单终态");
});

test("PR-DU keeps request context and reuses idempotency after an in-flight save", async ({
  page,
}) => {
  const scenario = "e2e-settings-credential-static-adapter-boundary";
  await useScenario(page, scenario);
  const idempotencyKeys: string[] = [];
  let attempt = 0;
  await page.route(`**/${scenario}/api/exchanges/credentials`, async (route) => {
    if (route.request().method() !== "POST") {
      await route.continue();
      return;
    }
    attempt += 1;
    idempotencyKeys.push(route.request().headers()["idempotency-key"] ?? "");
    if (attempt === 1) {
      await route.fulfill({
        status: 409,
        headers: responseHeaders("req-du-in-flight", 2),
        body: JSON.stringify({
          error: {
            code: "ACTION_RUN_IN_FLIGHT",
            message: "credential mutation is still in flight",
            status: 409,
            source: "action_runs",
            requestId: "req-du-in-flight",
            retryAfterMs: 2_000,
            details: { recoveryAction: "retry_same_idempotency_key" },
          },
        }),
      });
      return;
    }
    await route.fulfill({
      status: 200,
      headers: responseHeaders("req-du-credential-save"),
      body: JSON.stringify(successfulCredentialUpdate()),
    });
  });

  await page.goto("/#settings");
  const secret = "du-secret-must-not-render";
  await page.getByLabel("API Secret", { exact: true }).fill(secret);
  await page.getByRole("button", { name: "保存字段" }).click();

  const message = page.locator(".credential-editor-actions em");
  await expect(message).toContainText("credential mutation is still in flight");
  await expect(message).toContainText("code ACTION_RUN_IN_FLIGHT");
  await expect(message).toContainText("request_id req-du-in-flight");
  await expect(message).toContainText("retry 2000ms");
  await expect(message).not.toContainText("credential evidence persisted");

  await page.getByRole("button", { name: "保存字段" }).click();

  await expect(message).toContainText("credential evidence persisted");
  await expect(message).toContainText("Action action-du-credential-save");
  await expect(message).toContainText("Request req-du-credential-save");
  await expect.poll(() => idempotencyKeys).toHaveLength(2);
  expect(idempotencyKeys[0]).not.toBe("");
  expect(idempotencyKeys[1]).toBe(idempotencyKeys[0]);
  await expect(message).toContainText(`Idempotency ${idempotencyKeys[0]}`);
  await expect(page.locator("body")).not.toContainText(secret);
});

test("PR-DU keeps execution environment read-only and legacy readiness absent", async ({ page }) => {
  await useScenario(page, "e2e-pr-fu-settings-environment");
  await page.goto("/#settings");

  await expect(page.locator(".settings-tabs button")).toHaveCount(4);
  await expect(page.getByRole("tab", { name: "凭证", exact: true })).toBeVisible();
  await expect(page.getByRole("tab", { name: "风控", exact: true })).toBeVisible();
  await expect(page.getByRole("tab", { name: "动作账本", exact: true })).toBeVisible();
  await expect(page.getByRole("tab", { name: "诊断", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: /readiness|准入|切换执行环境/i })).toHaveCount(0);

  await page.getByRole("tab", { name: "诊断", exact: true }).click();
  const environment = page.locator('[data-settings-table="execution-environment"]');
  await expect(environment).toBeVisible();
  await expect(environment).toContainText("模拟环境");
  await expect(environment).toContainText("实盘环境");
  await expect(environment.locator("select, button")).toHaveCount(0);
  await expect(page.getByText(/只读：.*HedgeTicket.*双腿预检/)).toBeVisible();
  await expect(page.getByText(/Dry-run|Testnet|确认短语|旧准入/)).toHaveCount(0);
});

test("PR-BN separates runtime facts, editable risk, and kill-switch policy", async ({ page }) => {
  const scenario = "e2e-pr-fu-settings-environment";
  await useScenario(page, scenario);
  let requestBody: Record<string, unknown> | undefined;
  let requestId = "";
  let idempotencyKey = "";
  await page.route(`**/${scenario}/api/trading/kill-switch`, async (route) => {
    if (route.request().method() === "POST") {
      requestBody = route.request().postDataJSON() as Record<string, unknown>;
      requestId = route.request().headers()["x-request-id"] ?? "";
      idempotencyKey = route.request().headers()["idempotency-key"] ?? "";
      const upstream = await route.fetch();
      const response = await upstream.json();
      response.requestId = requestId;
      response.idempotencyKey = idempotencyKey;
      await route.fulfill({
        response: upstream,
        headers: responseHeaders(requestId),
        body: JSON.stringify(response),
      });
      return;
    }
    await route.continue();
  });

  await page.goto("/#settings");
  await page.getByRole("tab", { name: "风控", exact: true }).click();

  const runtime = page.locator('[data-settings-risk-scope="runtime-readonly"]');
  await expect(runtime).toContainText("运行态事实");
  await expect(runtime).toContainText("只读");
  await expect(runtime).toContainText("后端模拟环境");
  await expect(runtime).toContainText("实盘写入停用");
  await expect(runtime).toContainText("Kill Switch 关闭");
  await expect(runtime).toContainText(
    "阻止非 reduce-only 新订单；保留 reduce-only 平仓与撤单；不会自动撤销现有挂单",
  );
  await expect(runtime.locator("input, select, button")).toHaveCount(0);

  const editable = page.locator('[data-settings-risk-scope="editable-thresholds"]');
  // c8c0f949 双边退出保护重写风控 tab 后，可编辑区首个标题为"订单约束"。
  await expect(editable).toContainText("订单约束");
  await expect(editable).toContainText("可编辑");
  await expect(editable.getByLabel("单笔名义上限 USD")).toHaveValue("5000");
  await expect(editable.getByLabel("最大挂单数")).toHaveValue("10");
  await expect(editable.getByRole("button", { name: "保存风控" })).toBeEnabled();

  await page.getByRole("button", { name: "切换 Kill Switch" }).click();
  await expect.poll(() => requestBody).toEqual({
    active: true,
    expectedActive: false,
    expectedOpenOrderCount: 0,
    reason: "settings.kill_switch.enable",
  });
  expect(requestId).not.toBe("");
  expect(idempotencyKey).not.toBe("");

  const message = page.locator(
    '[data-settings-risk-scope="kill-switch-action"] .settings-message',
  );
  await expect(message).toContainText("Action e2e-kill-switch-action");
  await expect(message).toContainText(`Request ${requestId}`);
  await expect(message).toContainText(`request_id ${requestId}`);
  await expect(message).toContainText(`idempotency ${idempotencyKey}`);
});
