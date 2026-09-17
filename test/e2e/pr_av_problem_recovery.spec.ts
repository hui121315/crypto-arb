import { expect, test, type Page } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";
const WEB_BASE = process.env.CROSSLINE_E2E_WEB_BASE ?? "http://127.0.0.1:18080";
const ROUTE_ALLOW_HEADERS = "content-type,authorization,accept,x-request-id,idempotency-key";

type RecoveryProblem = {
  code: string;
  message: string;
  source: string;
  status: number;
  recoveryAction: string;
  details: Record<string, unknown>;
};

function routeHeaders(methods: string, requestId: string, retryAfterSeconds: number) {
  return {
    "access-control-allow-origin": WEB_BASE,
    "access-control-allow-methods": methods,
    "access-control-allow-headers": ROUTE_ALLOW_HEADERS,
    "access-control-expose-headers": "retry-after,x-request-id",
    "content-type": "application/json; charset=utf-8",
    "retry-after": String(retryAfterSeconds),
    "x-request-id": requestId,
    vary: "origin",
  };
}

async function useApiBase(page: Page) {
  await page.addInitScript((apiBase) => {
    window.localStorage.setItem("api_base", JSON.stringify(apiBase));
  }, API_BASE);
}

async function routeProblem(
  page: Page,
  path: string,
  methods: string,
  requestId: string,
  retryAfterSeconds: number,
  error: RecoveryProblem,
) {
  await page.route(path, async (route) => {
    const headers = routeHeaders(methods, requestId, retryAfterSeconds);
    if (route.request().method() === "OPTIONS") {
      await route.fulfill({ status: 204, headers, body: "" });
      return;
    }
    await route.fulfill({ status: error.status, headers, body: JSON.stringify({ error }) });
  });
}

test("PR-AV opportunity failure exposes structured recovery without fake rows", async ({ page }) => {
  await useApiBase(page);
  await routeProblem(
    page,
    "**/api/v3/arbitrage/opportunities/list**",
    "GET,OPTIONS",
    "req-av-opportunity",
    2,
    {
      code: "MARKET_DATA_RATE_LIMITED",
      message: "opportunity list rate limited",
      source: "arbitrage.scan",
      status: 429,
      recoveryAction: "retry_after_delay",
      details: {
        venue: "binance",
        operation: "rest_orderbook",
        method: "GET",
        path: "/fapi/v1/depth",
        symbol: "BTCUSDT",
        upstreamCode: "-1003",
        latencyMs: 39,
      },
    },
  );

  await page.goto("/#opportunities");
  await expect(page.getByRole("heading", { name: "机会扫描" })).toBeVisible();

  const failure = page
    .locator(".settings-message.is-error")
    .filter({ hasText: "机会快照冷启动失败" });
  await expect(failure).toContainText("venue binance");
  await expect(failure).toContainText("operation rest_orderbook");
  await expect(failure).toContainText("GET /fapi/v1/depth");
  await expect(failure).toContainText("request_id req-av-opportunity");
  await expect(failure).toContainText("下一步 等待后重试");
  await expect(page.locator(".clean-table tbody tr").filter({ hasText: "MU" })).toHaveCount(0);
});

test("PR-AV settings failure exposes operation context and operator recovery", async ({ page }) => {
  await useApiBase(page);
  await routeProblem(page, "**/api/exchanges/credentials", "GET,OPTIONS", "req-av-settings", 3, {
    code: "CREDENTIAL_STORE_UNAVAILABLE",
    message: "credential store unavailable",
    source: "settings.credentials",
    status: 503,
    recoveryAction: "check_runtime_health",
    details: {
      venue: "okx",
      operation: "credentials_read",
      method: "GET",
      path: "/api/exchanges/credentials",
      upstreamCode: "KEYCHAIN_UNAVAILABLE",
    },
  });

  await page.goto("/#settings");
  await expect(page.locator("h1", { hasText: "设置" })).toBeVisible();

  const editor = page.locator(".credential-editor");
  await expect(editor.locator("select")).toContainText("operation credentials_read");
  await expect(editor.locator("select")).toContainText("GET /api/exchanges/credentials");
  await expect(editor.locator("select")).toContainText("upstream_code KEYCHAIN_UNAVAILABLE");
  await expect(editor.locator("select")).toContainText("request_id req-av-settings");
  await expect(editor.locator("select")).toContainText("下一步 检查运行状态");
  await expect(editor.getByRole("button", { name: "保存字段" })).toBeDisabled();
});

test("PR-AV execution preview exposes scoped recovery and remains blocked", async ({ page }) => {
  await useApiBase(page);
  await routeProblem(
    page,
    "**/api/arbitrage/opportunities/*/preview",
    "POST,OPTIONS",
    "req-av-preview",
    2,
    {
      code: "PREVIEW_UPSTREAM_TIMEOUT",
      message: "preview venue timed out",
      source: "execution.preview",
      status: 504,
      recoveryAction: "check_runtime_health",
      details: {
        venue: "bybit",
        operation: "hedge_preview",
        method: "POST",
        path: "/api/arbitrage/opportunities/opp-basis-1/preview",
        symbol: "BTCUSDT",
      },
    },
  );

  await page.goto("/#futures");
  await expect(page.getByRole("heading", { name: "期货套利" })).toBeVisible();
  await page.getByRole("button", { name: "构建对冲" }).click();

  const failure = page.locator(".execution-risk-section .risk-empty.stale-note");
  await expect(failure).toContainText("venue bybit");
  await expect(failure).toContainText("operation hedge_preview");
  await expect(failure).toContainText("symbol BTCUSDT");
  await expect(failure).toContainText("request_id req-av-preview");
  await expect(failure).toContainText("下一步 检查运行状态");
  await expect(page.getByRole("button", { name: /提交/ })).toBeDisabled();
});

test("PR-AV review failure exposes ledger context without fake execution", async ({ page }) => {
  await useApiBase(page);
  await routeProblem(page, "**/api/review/executed**", "GET,OPTIONS", "req-av-review", 4, {
    code: "REVIEW_LEDGER_UNAVAILABLE",
    message: "review ledger unavailable",
    source: "review.ledger",
    status: 500,
    recoveryAction: "contact_operator",
    details: {
      operation: "executed_trade_list",
      method: "GET",
      path: "/api/review/executed",
      upstreamCode: "TRADING_SQL_LEDGER_UNAVAILABLE",
    },
  });

  await page.goto("/#review");
  await expect(page.locator("h1", { hasText: "复盘" })).toBeVisible();

  const reviewSurface = page.locator(".surface").filter({ hasText: "交易复盘" });
  const stateLine = reviewSurface.locator(".reason-pill").first();
  await expect(stateLine).toContainText("operation executed_trade_list");
  await expect(stateLine).toContainText("GET /api/review/executed");
  await expect(stateLine).toContainText("request_id req-av-review");
  await expect(stateLine).toContainText("下一步 联系操作员");
  await expect(reviewSurface.locator("tbody tr").filter({ hasText: "MU" })).toHaveCount(0);
  await expect(page.getByText("暂无已执行记录")).toHaveCount(0);
});
