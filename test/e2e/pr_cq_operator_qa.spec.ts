import { expect, test, type Page } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";
const WEB_BASE = process.env.CROSSLINE_E2E_WEB_BASE ?? "http://127.0.0.1:18080";
const SETTINGS_SCENARIO = "e2e-settings-credential-static-adapter-boundary";
const EXECUTION_SCENARIO = "e2e-order-cancel-denied";

function responseHeaders(requestId: string, retryAfterSeconds: number) {
  return {
    "access-control-allow-origin": WEB_BASE,
    "access-control-allow-methods": "GET,POST,PATCH,OPTIONS",
    "access-control-allow-headers":
      "content-type,authorization,accept,x-request-id,idempotency-key",
    "access-control-expose-headers": "retry-after,x-request-id",
    "content-type": "application/json; charset=utf-8",
    "retry-after": String(retryAfterSeconds),
    "x-request-id": requestId,
    vary: "origin",
  };
}

async function useScenario(page: Page, scenario: string) {
  await page.addInitScript(
    ({ apiBase, scenarioName }) => {
      window.localStorage.setItem("api_base", JSON.stringify(`${apiBase}/${scenarioName}`));
      window.localStorage.setItem("api_auth_token", JSON.stringify("e2e-token"));
      Date.now = () => 1_770_000_000_000;
      Math.random = () => 0.25;
    },
    { apiBase: API_BASE, scenarioName: scenario },
  );
  await page.routeWebSocket("**/ws", (socket) => socket.close());
}

function responseGate() {
  let release: (() => void) | undefined;
  const wait = new Promise<void>((resolve) => {
    release = resolve;
  });
  return { wait, release: () => release?.() };
}

test("PR-CQ credential save failure exits pending and records operator-visible evidence", async ({
  page,
}) => {
  await useScenario(page, SETTINGS_SCENARIO);
  const gate = responseGate();
  let requestSeen = false;
  await page.route(`**/${SETTINGS_SCENARIO}/api/exchanges/credentials`, async (route) => {
    if (route.request().method() !== "POST") {
      await route.continue();
      return;
    }
    requestSeen = true;
    await gate.wait;
    await route.fulfill({
      status: 503,
      headers: responseHeaders("req-cq-secret-backend", 2),
      body: JSON.stringify({
        error: {
          code: "SECRET_BACKEND_UNAVAILABLE",
          message: "operator keychain is locked",
          source: "pr-cq.operator-qa",
          status: 503,
        },
      }),
    });
  });

  await page.goto("/#settings");
  await page.getByLabel("API Secret", { exact: true }).fill("never-render-this-secret");
  await page.getByRole("button", { name: "保存字段" }).click();
  await expect.poll(() => requestSeen).toBe(true);
  await expect(page.getByRole("button", { name: "保存中" })).toBeDisabled();

  gate.release();
  await expect(page.getByRole("button", { name: "保存字段" })).toBeEnabled();
  const editor = page.locator(".credential-editor").first();
  const message = editor.locator(".credential-editor-actions em");
  await expect(message).toContainText("SECRET_BACKEND_UNAVAILABLE");
  await expect(message).toContainText("request_id req-cq-secret-backend");
  await expect(message).toContainText("retry 2000ms");
  await expect(page.locator("body")).not.toContainText("never-render-this-secret");
  await expect(editor).toHaveScreenshot("pr-cq-credential-save-failure.png", {
    animations: "disabled",
    caret: "hide",
    mask: [editor.locator("input")],
    scale: "css",
  });
});

test("PR-CQ execution submit failure exits pending and remains retryable", async ({ page }) => {
  await useScenario(page, EXECUTION_SCENARIO);
  const gate = responseGate();
  let requestSeen = false;
  await page.route("**/api/arbitrage/opportunities/mock-mu-perp/confirm", async (route) => {
    requestSeen = true;
    await gate.wait;
    await route.fulfill({
      status: 502,
      headers: responseHeaders("req-cq-execution-submit", 3),
      body: JSON.stringify({
        error: {
          code: "EXECUTION_UPSTREAM_UNAVAILABLE",
          message: "execution coordinator unavailable",
          source: "pr-cq.operator-qa",
          status: 502,
        },
      }),
    });
  });

  await page.goto("/#futures");
  await page.getByRole("button", { name: "构建对冲" }).click();
  const submit = page.getByRole("button", { name: "提交 模拟" });
  await expect(submit).toBeEnabled();
  await submit.click();
  await expect.poll(() => requestSeen).toBe(true);
  await expect(page.locator(".execution-actionbar .run-state > span")).toHaveText("模拟提交中");
  await expect(submit).toBeDisabled();

  gate.release();
  const actionBar = page.locator(".execution-actionbar");
  await expect(actionBar.locator(".run-state > span")).toHaveText("模拟提交失败");
  await expect(actionBar.locator(".run-state em").first()).toContainText(
    "EXECUTION_UPSTREAM_UNAVAILABLE",
  );
  await expect(actionBar.locator(".run-state em").first()).toContainText(
    "request_id req-cq-execution-submit",
  );
  await expect(actionBar.locator(".run-state em").first()).toContainText("retry 3000ms");
  await expect(submit).toBeEnabled();
  await expect(actionBar).toHaveScreenshot("pr-cq-execution-submit-failure.png", {
    animations: "disabled",
    caret: "hide",
    scale: "css",
  });
});
