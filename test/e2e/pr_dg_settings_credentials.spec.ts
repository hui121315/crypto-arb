import { expect, test, type Page } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";
const WEB_BASE = process.env.CROSSLINE_E2E_WEB_BASE ?? "http://127.0.0.1:18080";
const SCENARIO = "settings-credential-static-adapter-boundary";
const CREDENTIALS_ROUTE = `**/${SCENARIO}/api/exchanges/credentials`;
const ADAPTERS_ROUTE = `**/${SCENARIO}/api/trading/adapters`;
const FIXTURE_TIME_MS = 1_789_000_000_000;

type JsonRecord = Record<string, unknown>;

function routeHeaders(requestId = "req-dg-credentials", retryAfterSeconds?: number) {
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

async function useScenarioApiBase(page: Page) {
  await page.addInitScript(
    ({ apiBase, scenario }) => {
      window.localStorage.setItem("api_base", JSON.stringify(`${apiBase}/${scenario}`));
      window.localStorage.setItem("api_auth_token", JSON.stringify("e2e-token"));
    },
    { apiBase: API_BASE, scenario: SCENARIO },
  );
}

function credentialField(
  key: string,
  label: string,
  envKey: string,
  secret: boolean,
  configured: boolean,
  required = true,
) {
  return {
    key,
    label,
    envKey,
    configured,
    secret,
    required,
    source: configured ? "keychain" : "missing",
  };
}

function credentialProbe(kind: string, status: "ok" | "unknown", source: string) {
  return {
    kind,
    status,
    scope: `hyperliquid_${kind}`,
    source,
    message: status === "ok" ? `${kind} verified` : `${kind} remains unproven`,
    checkedAtMs: FIXTURE_TIME_MS,
    requestId: `req-dg-${kind}`,
  };
}

function validationEvidence() {
  return {
    status: "read_only_ok",
    checkedAtMs: FIXTURE_TIME_MS,
    probes: [
      credentialProbe("balance_read", "ok", "hyperliquid.info clearinghouseState"),
      credentialProbe("positions_read", "ok", "hyperliquid.info clearinghouseState"),
      credentialProbe("open_orders_read", "ok", "hyperliquid.info frontendOpenOrders"),
      credentialProbe(
        "order_permission",
        "unknown",
        "hyperliquid.POST /exchange action=noop",
      ),
      credentialProbe(
        "account_mode_read",
        "ok",
        "userRole(account)+userRole(signer)+vaultDetails",
      ),
    ],
  };
}

function secretStorage() {
  return {
    mode: "keychain",
    persistent: true,
    encrypted: true,
    atomicWrite: true,
    path: "service:com.crossline.crypto-arb.venue-credentials",
    label: "macOS Keychain",
    message: "系统 Keychain 已启用，凭证通过登录会话加密持久化",
    warning: null,
  };
}

function credentialsResponse(configured: boolean) {
  return {
    venues: [{
      venue: "hyperliquid",
      label: "Hyperliquid",
      fields: [
        credentialField(
          "account_address",
          "余额读取地址（主账户 / 子账户）",
          "HYPERLIQUID_ACCOUNT_ADDRESS",
          false,
          configured,
        ),
        credentialField(
          "private_key",
          "已授权 API / Agent 钱包私钥",
          "HYPERLIQUID_PRIVATE_KEY",
          true,
          configured,
        ),
        credentialField(
          "vault_address",
          "Vault 执行地址（可选）",
          "HYPERLIQUID_VAULT_ADDRESS",
          false,
          configured,
          false,
        ),
      ],
      publicMarket: true,
      privateRead: true,
      testnetWrite: false,
      liveWrite: true,
      note: "vault/subaccount actions require master signer plus vaultAddress",
      missingFields: configured ? [] : ["account_address", "private_key"],
      validationEvidence: configured ? validationEvidence() : null,
    }],
    secretStorage: secretStorage(),
  };
}

function adapterCapabilities() {
  return {
    spot: false,
    perp: true,
    limitOrders: true,
    marketOrders: false,
    postOnly: true,
    reduceOnly: true,
  };
}

async function routeTradingAdapters(page: Page) {
  await page.route(ADAPTERS_ROUTE, async (route) => {
    const headers = routeHeaders("req-dg-adapters");
    if (route.request().method() === "OPTIONS") {
      await route.fulfill({ status: 204, headers, body: "" });
      return;
    }
    await route.fulfill({
      status: 200,
      headers,
      body: JSON.stringify({
        current: "mock",
        currentEnvironment: "paper",
        options: [
          {
            id: "mock",
            label: "Paper",
            environment: "paper",
            enabled: true,
            credentialsAvailable: true,
            capabilities: adapterCapabilities(),
            disabledReason: null,
          },
          {
            id: "live_router",
            label: "实盘",
            environment: "live",
            enabled: true,
            credentialsAvailable: true,
            capabilities: adapterCapabilities(),
            disabledReason: null,
          },
        ],
        venues: [],
      }),
    });
  });
}

async function routeCredentialLifecycle(page: Page) {
  let configured = false;
  let savedRequest: JsonRecord | undefined;
  await page.route(CREDENTIALS_ROUTE, async (route) => {
    const method = route.request().method();
    const headers = routeHeaders();
    if (method === "OPTIONS") {
      await route.fulfill({ status: 204, headers, body: "" });
      return;
    }
    if (method === "GET") {
      await route.fulfill({ status: 200, headers, body: JSON.stringify(credentialsResponse(configured)) });
      return;
    }
    savedRequest = route.request().postDataJSON() as JsonRecord;
    configured = true;
    await route.fulfill({
      status: 200,
      headers,
      body: JSON.stringify({
        venue: "hyperliquid",
        label: "Hyperliquid",
        configuredCount: 3,
        fieldCount: 3,
        message: "vault-scoped credentials persisted",
        secretStorage: secretStorage(),
        validationEvidence: validationEvidence(),
        actionRunId: "action-dg-credential-save",
        requestId: "req-dg-credential-save",
      }),
    });
  });
  return () => savedRequest;
}

test("PR-DG persists the dynamic Hyperliquid vault field and keeps readiness fail-closed", async ({
  page,
}) => {
  await useScenarioApiBase(page);
  const savedRequest = await routeCredentialLifecycle(page);
  await routeTradingAdapters(page);

  await page.goto("/#settings");
  await expect(page.locator("h1", { hasText: "设置" })).toBeVisible();
  await expect(page.getByLabel("交易所")).toHaveValue("hyperliquid");

  await page.getByLabel("余额读取地址（主账户 / 子账户）").fill(
    "0x1111111111111111111111111111111111111111",
  );
  await page.getByLabel("已授权 API / Agent 钱包私钥").fill("agent-private-key");
  await page.getByLabel("Vault 执行地址（可选）").fill(
    "0x2222222222222222222222222222222222222222",
  );
  await page.getByRole("button", { name: "保存字段" }).click();

  await expect.poll(savedRequest).toMatchObject({
    venue: "hyperliquid",
    fields: expect.arrayContaining([
      { key: "account_address", value: "0x1111111111111111111111111111111111111111" },
      { key: "private_key", value: "agent-private-key" },
      { key: "vault_address", value: "0x2222222222222222222222222222222222222222" },
    ]),
  });

  const storage = page.locator(".runtime-health-panel").filter({ hasText: "Secret 存储" });
  await expect(storage).toContainText("macOS Keychain");
  await expect(storage).toContainText("已加密");
  await expect(storage).toContainText("原子写入");

  const summary = page.locator(".credential-summary strong");
  await expect(summary).toContainText(/2\/2 必填字段已填写.*1\/1 可选字段已填写/);
  await expect(summary).toContainText(/未验证.*订单权限/);
  await expect(summary).not.toContainText(/可下单|实盘就绪|权限验证完整/);

  const validation = page.locator(".runtime-health-panel").filter({ hasText: "保存期验证" });
  await expect(validation).toContainText("权限未完整");
  await expect(validation).toContainText("hyperliquid.POST /exchange action=noop");
  await expect(validation).not.toContainText("权限验证完整");

  await page.getByRole("tab", { name: "诊断" }).click();
  const liveAdapter = page
    .locator('[data-settings-table="execution-environment"] tbody tr')
    .filter({ hasText: "live_router" });
  await expect(liveAdapter).toContainText("字段组已补齐");
  await expect(liveAdapter).toContainText("下单仍需票据级权限与运行状态数据依据");
  await expect(liveAdapter).not.toContainText("可下单");
});

test("PR-DG credential cold error exposes typed request and retry context", async ({ page }) => {
  await useScenarioApiBase(page);
  await page.route(CREDENTIALS_ROUTE, async (route) => {
    const headers = routeHeaders("req-dg-credentials-502", 4);
    if (route.request().method() === "OPTIONS") {
      await route.fulfill({ status: 204, headers, body: "" });
      return;
    }
    await route.fulfill({
      status: 502,
      headers,
      body: JSON.stringify({
        error: {
          code: "CREDENTIAL_REGISTRY_UNAVAILABLE",
          message: "credential registry unavailable",
          source: "pr-dg-settings",
          status: 502,
        },
      }),
    });
  });

  await page.goto("/#settings");
  const context =
    "credential registry unavailable · code CREDENTIAL_REGISTRY_UNAVAILABLE · source pr-dg-settings · HTTP 502 · request_id req-dg-credentials-502 · retry 4000ms";
  await expect(page.getByLabel("交易所")).toContainText(`读取交易所列表失败：${context}`);
  await expect(page.locator(".empty-cell").filter({ hasText: "读取凭证状态失败" }))
    .toContainText(`读取凭证状态失败：${context}`);
  await expect(page.getByText("Secret 存储", { exact: true })).toHaveCount(0);
  await expect(page.getByText("保存期验证", { exact: true })).toHaveCount(0);
});
