import { expect, test } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";
const WEB_BASE = process.env.CROSSLINE_E2E_WEB_BASE ?? "http://127.0.0.1:18080";
const ROUTE_ALLOW_HEADERS = "content-type,authorization,accept,x-request-id,idempotency-key";

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

async function useScenarioApiBase(page, scenario: string) {
  await page.addInitScript(
    ({ apiBase, scenario }) => {
      window.localStorage.setItem("api_base", JSON.stringify(`${apiBase}/${scenario}`));
    },
    { apiBase: API_BASE, scenario },
  );
}

async function useApiBase(page) {
  await page.addInitScript((apiBase) => {
    window.localStorage.setItem("api_base", JSON.stringify(apiBase));
  }, API_BASE);
}

async function routeProblem(
  page,
  path: string,
  methods: string,
  requestId: string,
  retryAfterSeconds: number,
  error: { code: string; message: string; source: string; status: number },
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

test("PR-FT opportunities cold error remains typed and leaves no fake opportunity", async ({
  page,
}) => {
  await useApiBase(page);
  await routeProblem(page, "**/api/v3/arbitrage/opportunities/list**", "GET,OPTIONS", "req-ft-opportunities", 2, {
    code: "MARKET_DATA_RATE_LIMITED",
    message: "opportunity list rate limited",
    source: "pr-ft-opportunities",
    status: 429,
  });

  await page.goto("/#opportunities");
  await expect(page.getByRole("heading", { name: "机会扫描" })).toBeVisible();

  const context = "opportunity list rate limited · source pr-ft-opportunities · HTTP 429 · request_id req-ft-opportunities · retry 2000ms";
  await expect(
    page.locator(".settings-message.is-error").filter({ hasText: "机会快照冷启动失败" }),
  ).toContainText(`机会快照冷启动失败 · ${context}`);
  await expect(page.locator(".empty-cell")).toContainText(`机会快照错误 · ${context}`);
  await expect(page.locator(".clean-table tbody tr").filter({ hasText: "MU" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "构建对冲" })).toHaveCount(0);
});

test("PR-FT review cold error remains typed and leaves no fake execution record", async ({ page }) => {
  await useScenarioApiBase(page, "e2e-review-502");
  await page.goto("/#review");
  await expect(page.locator("h1", { hasText: "复盘" })).toBeVisible();

  const reviewSurface = page.locator(".surface").filter({ hasText: "交易复盘" });
  const stateLine = reviewSurface.locator(".reason-pill").first();
  const table = reviewSurface.locator(".review-table").first();

  await expect(stateLine).toContainText("读取失败");
  await expect(stateLine).toContainText("mock review executed failed");
  await expect(stateLine).toContainText("HTTP 502");
  await expect(stateLine).toContainText("request_id e2e-review-executed-502");
  await expect(stateLine).toContainText("retry 3000ms");
  await expect(table.locator(".empty-cell")).toContainText("读取失败：mock review executed failed");
  await expect(table.locator("tbody tr").filter({ hasText: "MU" })).toHaveCount(0);
  await expect(page.getByText("暂无已执行记录")).toHaveCount(0);
});

test("PR-FT settings credentials cold error remains typed and hides static success panels", async ({
  page,
}) => {
  await useApiBase(page);
  await routeProblem(page, "**/api/exchanges/credentials", "GET,OPTIONS", "req-ft-credentials", 4, {
    code: "CREDENTIALS_UNAVAILABLE",
    message: "credential registry unavailable",
    source: "pr-ft-settings",
    status: 502,
  });

  await page.goto("/#settings");
  await expect(page.locator("h1", { hasText: "设置" })).toBeVisible();

  const context = "credential registry unavailable · code CREDENTIALS_UNAVAILABLE · source pr-ft-settings · HTTP 502 · request_id req-ft-credentials · retry 4000ms";
  const editor = page.locator(".credential-editor");
  await expect(editor.locator("select")).toContainText(`读取交易所列表失败：${context}`);
  await expect(editor.locator('[data-credential-spec-state="error"]')).toContainText(
    `读取凭证规格失败：${context}`,
  );
  await expect(editor.getByRole("button", { name: "保存字段" })).toBeDisabled();
  await expect(editor).not.toContainText("等待交易所凭证规格");
  await expect(page.locator(".empty-cell").filter({ hasText: "读取凭证状态失败" })).toContainText(
    `读取凭证状态失败：${context}`,
  );
  await expect(page.getByText("Secret 存储", { exact: true })).toHaveCount(0);
  await expect(page.getByText("静态能力证据", { exact: true })).toHaveCount(0);
  await expect(page.getByText("保存期验证", { exact: true })).toHaveCount(0);
});

test("PR-FT execution preview cold error remains typed and blocks submit", async ({ page }) => {
  await useApiBase(page);
  await routeProblem(page, "**/api/arbitrage/opportunities/*/preview", "POST,OPTIONS", "req-ft-preview", 2, {
    code: "MARKET_DATA_RATE_LIMITED",
    message: "preview rate limited",
    source: "pr-ft-execution",
    status: 429,
  });

  await page.goto("/#futures");
  await expect(page.getByRole("heading", { name: "期货套利" })).toBeVisible();
  await page.getByRole("button", { name: "构建对冲" }).click();

  const riskSection = page.locator(".execution-risk-section");
  await expect(riskSection.locator(".execution-section-head strong")).toHaveText("失效");
  await expect(riskSection.locator(".execution-section-head em")).toHaveText("预检失效 · 等待");
  await expect(riskSection.locator(".risk-empty.stale-note")).toContainText(
    "预览已失效：preview rate limited · code MARKET_DATA_RATE_LIMITED · source pr-ft-execution · HTTP 429 · request_id req-ft-preview · retry 2000ms",
  );
  await expect(riskSection.locator(".risk-notes p")).toContainText(
    "上次预览已失效：preview rate limited · code MARKET_DATA_RATE_LIMITED · source pr-ft-execution · HTTP 429 · request_id req-ft-preview · retry 2000ms",
  );
  await expect(page.getByRole("button", { name: /提交/ })).toBeDisabled();
});
