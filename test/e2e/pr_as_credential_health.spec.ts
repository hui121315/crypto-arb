import { expect, test, type Page } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";
const WEB_BASE = process.env.CROSSLINE_E2E_WEB_BASE ?? "http://127.0.0.1:18080";
const SCENARIO = "e2e-settings-selected-venue-trading-runtime-all-ok-local-capture-shaped";
const CREDENTIALS_ROUTE = `**/${SCENARIO}/api/exchanges/credentials`;
const CHECKED_AT_MS = 1_789_000_000_000;

type SecretHealth = "ready" | "unavailable";

async function useScenario(page: Page) {
  await page.addInitScript(
    ({ apiBase, scenario }) => {
      window.localStorage.setItem("api_base", JSON.stringify(`${apiBase}/${scenario}`));
      window.localStorage.setItem("api_auth_token", JSON.stringify("e2e-token"));
    },
    { apiBase: API_BASE, scenario: SCENARIO },
  );
}

function responseHeaders() {
  return {
    "access-control-allow-origin": WEB_BASE,
    "access-control-allow-methods": "GET,POST,OPTIONS",
    "access-control-allow-headers":
      "content-type,authorization,accept,x-request-id,idempotency-key",
    "access-control-expose-headers": "retry-after,x-request-id",
    "content-type": "application/json; charset=utf-8",
    "x-request-id": "req-pr-as-credentials",
    vary: "origin",
  };
}

function probe(kind: string, status: "ok" | "unknown") {
  return {
    kind,
    status,
    scope: `okx_${kind}`,
    source: `credential_validation.${kind}`,
    message: status === "ok" ? `${kind} verified` : `${kind} remains unproven`,
    checkedAtMs: CHECKED_AT_MS,
    requestId: `req-pr-as-${kind}`,
  };
}

function credentialsResponse(health: SecretHealth) {
  const unavailable = health === "unavailable";
  return {
    venues: [{
      venue: "okx",
      label: "OKX",
      fields: [
        ["api_key", "API Key", "OKX_API_KEY"],
        ["api_secret", "API Secret", "OKX_API_SECRET"],
        ["passphrase", "Passphrase", "OKX_PASSPHRASE"],
      ].map(([key, label, envKey]) => ({
        key,
        label,
        envKey,
        configured: true,
        secret: true,
        required: true,
        source: "keychain",
      })),
      publicMarket: true,
      privateRead: true,
      testnetWrite: false,
      liveWrite: true,
      note: "static adapter declaration; runtime evidence remains independent",
      missingFields: [],
      validationEvidence: {
        status: "read_only_ok",
        checkedAtMs: CHECKED_AT_MS,
        probes: [
          probe("balance_read", "ok"),
          probe("positions_read", "ok"),
          probe("open_orders_read", "ok"),
          probe("order_permission", "unknown"),
          probe("account_mode_read", "ok"),
        ],
      },
    }],
    secretStorage: {
      mode: "keychain",
      health,
      persistent: !unavailable,
      encrypted: !unavailable,
      atomicWrite: !unavailable,
      path: "service:com.crossline.crypto-arb.venue-credentials",
      label: "macOS Keychain",
      message: unavailable
        ? "Secret backend 读取异常，凭证字段已按未配置处理。"
        : "系统 Keychain 已启用，凭证通过登录会话加密持久化",
      warning: unavailable ? "Secret backend error: keychain session locked" : null,
      lastError: unavailable ? "keychain session locked" : null,
    },
  };
}

async function routeCredentials(page: Page, health: SecretHealth) {
  await page.route(CREDENTIALS_ROUTE, async (route) => {
    if (route.request().method() === "OPTIONS") {
      await route.fulfill({ status: 204, headers: responseHeaders(), body: "" });
      return;
    }
    await route.fulfill({
      status: 200,
      headers: responseHeaders(),
      body: JSON.stringify(credentialsResponse(health)),
    });
  });
}

test("PR-AS keeps secret backend health separate from save-time and live runtime evidence", async ({
  page,
}) => {
  await useScenario(page);
  await routeCredentials(page, "ready");
  await page.goto("/#settings");

  const storage = page.locator('[data-secret-storage-health="可用"]');
  await expect(storage).toContainText("macOS Keychain");
  await expect(storage).toContainText("已加密");
  await expect(storage.locator(".status-pill")).toHaveText("可用");
  await expect(storage.locator(".status-pill")).toHaveClass(/ready/);

  const summary = page.locator(".credential-summary strong");
  await expect(summary).toContainText(/只读验证.*未验证.*订单权限/);
  await expect(summary).not.toContainText(/实盘就绪|权限验证完整/);

  const runtime = page.locator(
    '.runtime-health-panel:has(> .runtime-health-head strong:text-is("交易运行数据依据"))',
  );
  await expect(runtime).toContainText("当前可用");
  await expect(runtime).toContainText("写单运行状态");
  await expect(runtime).toContainText("私有订单流");
  await expect(runtime).toContainText("订单最终结果");
});

test("PR-AS blocks unavailable secret storage without erasing credential evidence", async ({
  page,
}) => {
  await useScenario(page);
  await routeCredentials(page, "unavailable");
  await page.goto("/#settings");

  const storage = page.locator('[data-secret-storage-health="不可用"]');
  await expect(storage.locator(".status-pill")).toHaveText("不可用");
  await expect(storage.locator(".status-pill")).toHaveClass(/blocked/);
  await expect(storage).toContainText("后端错误：keychain session locked");
  await expect(storage.locator(".status-pill.ready")).toHaveCount(0);

  const validation = page.locator(".runtime-health-panel").filter({ hasText: "保存期验证" });
  await expect(validation).toContainText("只读验证");
  await expect(validation).toContainText("权限未完整");
  await expect(page.locator("body")).not.toContainText("secret-value-must-not-render");
});
