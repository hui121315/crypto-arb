import { expect, test } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";
const WEB_BASE = process.env.CROSSLINE_E2E_WEB_BASE ?? "http://127.0.0.1:18080";
const STRATEGY_QUERY = "perp_cross,spot_perp,cross_spot_perp";
const ROUTE_ALLOW_HEADERS = "content-type,authorization,accept,x-request-id,idempotency-key";
const ROUTE_EXPOSE_HEADERS = "retry-after,x-request-id";
const PORTFOLIO_FIRST_FRAME_BUDGET_MS = process.env.CI ? 500 : 200;
const OPPORTUNITY_SWITCH_RENDER_BUDGET_MS = process.env.CI ? 5_000 : 2_000;
const OPPORTUNITY_LIST_BROWSER_CLONE_BUDGET_MS = process.env.CI ? 1_000 : 500;
const OPPORTUNITY_LIST_BROWSER_PARSE_BUDGET_MS = process.env.CI ? 1_000 : 500;
const OPPORTUNITY_LIST_WASM_DECODE_BUDGET_MS = process.env.CI ? 1_500 : 750;
const OPPORTUNITY_TABLE_RAF_COMMIT_BUDGET_MS = process.env.CI ? 2_000 : 1_000;
const OPPORTUNITY_SWITCH_FRAME_GAP_BUDGET_MS = process.env.CI ? 1_500 : 800;
const OPPORTUNITY_SWITCH_LONG_TASK_BUDGET_MS = process.env.CI ? 1_500 : 800;
const OPPORTUNITY_SWITCH_HEAP_DELTA_BUDGET_BYTES = 64 * 1024 * 1024;
const OPPORTUNITY_LIST_PAYLOAD_BUDGET_BYTES = 184_320;
const OPPORTUNITY_SWITCH_TABLE_CELL_BUDGET = 700;
const STATUS_API_SLOT_LABEL = "TradingAPI";
const STATUS_PRIVATE_WS_SLOT_LABEL = "PrivateWS";
const STATUS_ORDER_ELAPSED_SLOT_LABEL = "订单终态";
const TOP_STATUS_BAR_TEST_ID = "top-status-bar";
const STATUS_API_RUNTIME_TEST_ID = "status-api-runtime";
const STATUS_PRIVATE_WS_TEST_ID = "status-private-ws";
const STATUS_ORDER_ELAPSED_TEST_ID = "status-order-elapsed";
const VISUAL_SMOKE_VIEWPORTS = [
  { name: "desktop", width: 1440, height: 900 },
  { name: "mobile", width: 390, height: 844 },
];

function routeJsonHeaders(
  methods: string,
  extra: Record<string, string> = {},
): Record<string, string> {
  return {
    "access-control-allow-origin": WEB_BASE,
    "vary": "origin",
    "access-control-allow-methods": methods,
    "access-control-allow-headers": ROUTE_ALLOW_HEADERS,
    "access-control-expose-headers": ROUTE_EXPOSE_HEADERS,
    "content-type": "application/json; charset=utf-8",
    ...extra,
  };
}

async function useScenarioApiBase(page, scenario) {
  await page.addInitScript(
    ({ apiBase, scenario }) => {
      window.localStorage.setItem("api_base", JSON.stringify(`${apiBase}/${scenario}`));
    },
    { apiBase: API_BASE, scenario },
  );
}

async function useScenarioApiBaseWithAuth(page, scenario, token) {
  await page.addInitScript(
    ({ apiBase, scenario, token }) => {
      window.localStorage.setItem("api_base", JSON.stringify(`${apiBase}/${scenario}`));
      window.localStorage.setItem("api_auth_token", JSON.stringify(token));
    },
    { apiBase: API_BASE, scenario, token },
  );
}

async function expectFuturesReady(page) {
  await expect(page.getByRole("heading", { name: "期货套利" })).toBeVisible();
}

async function switchModule(page, name) {
  await page.getByRole("button", { name: new RegExp(name) }).click();
}

async function installOpportunityFetchProbe(page) {
  await page.addInitScript(() => {
    const runtime = window as any;
    if (runtime.__crosslineOpportunityHotPath) return;
    const originalFetch = window.fetch.bind(window);
    const state = {
      enabled: false,
      listResponses: [],
    };
    window.fetch = async (input, init) => {
      const response = await originalFetch(input, init);
      const href = typeof input === "string"
        ? input
        : input instanceof Request
          ? input.url
          : String(input);
      const url = new URL(href, window.location.href);
      if (state.enabled && url.pathname.endsWith("/api/v3/arbitrage/opportunities/list")) {
        const cloneStarted = performance.now();
        const text = await response.clone().text();
        const cloneReadMs = performance.now() - cloneStarted;
        const parseStarted = performance.now();
        JSON.parse(text);
        const jsonParseMs = performance.now() - parseStarted;
        state.listResponses.push({
          bytes: new TextEncoder().encode(text).byteLength,
          cloneReadMs,
          jsonParseMs,
          path: url.pathname,
        });
      }
      return response;
    };
    runtime.__crosslineOpportunityHotPath = state;
  });
}

async function setOpportunityFetchProbe(page, enabled) {
  return await page.evaluate((nextEnabled) => {
    const runtime = window as any;
    const state = runtime.__crosslineOpportunityHotPath;
    if (!state) return { missing: true, listResponses: [] };
    state.enabled = nextEnabled;
    if (nextEnabled) state.listResponses = [];
    return { missing: false, listResponses: state.listResponses };
  }, enabled);
}

async function startOpportunityHotPathProbe(page, tableSelector) {
  await page.evaluate((selector) => {
    const runtime = window as any;
    const performanceWithMemory = performance as any;
    const probe = {
      selector,
      startedAt: performance.now(),
      frameCount: 0,
      heapStartBytes: performanceWithMemory.memory?.usedJSHeapSize ?? null,
      longTaskCount: 0,
      longTaskMs: 0,
      maxCells: 0,
      maxFrameGapMs: 0,
      maxLongTaskMs: 0,
      maxRows: 0,
      running: true,
    };
    const sampleTable = () => {
      const rowCount = document.querySelectorAll(`${selector} tbody tr`).length;
      const cellCount = document.querySelectorAll(`${selector} tbody td`).length;
      probe.maxRows = Math.max(probe.maxRows, rowCount);
      probe.maxCells = Math.max(probe.maxCells, cellCount);
    };
    const observerCtor = runtime.PerformanceObserver;
    let observer = null;
    if (observerCtor) {
      try {
        observer = new observerCtor((list) => {
          for (const entry of list.getEntries()) {
            probe.longTaskCount += 1;
            probe.longTaskMs += entry.duration;
            probe.maxLongTaskMs = Math.max(probe.maxLongTaskMs, entry.duration);
          }
        });
        observer.observe({ entryTypes: ["longtask"] });
      } catch {
        observer = null;
      }
    }
    let lastFrame = performance.now();
    const tick = (now) => {
      if (!probe.running) return;
      probe.frameCount += 1;
      probe.maxFrameGapMs = Math.max(probe.maxFrameGapMs, now - lastFrame);
      lastFrame = now;
      sampleTable();
      requestAnimationFrame(tick);
    };
    probe.stop = () => {
      probe.running = false;
      observer?.disconnect();
      sampleTable();
      const heapEndBytes = performanceWithMemory.memory?.usedJSHeapSize ?? null;
      return {
        durationMs: performance.now() - probe.startedAt,
        frameCount: probe.frameCount,
        heapDeltaBytes: probe.heapStartBytes == null || heapEndBytes == null
          ? null
          : Math.max(0, heapEndBytes - probe.heapStartBytes),
        longTaskCount: probe.longTaskCount,
        longTaskMs: probe.longTaskMs,
        maxCells: probe.maxCells,
        maxFrameGapMs: probe.maxFrameGapMs,
        maxLongTaskMs: probe.maxLongTaskMs,
        maxRows: probe.maxRows,
      };
    };
    runtime.__crosslineOpportunityHotPathProbe = probe;
    requestAnimationFrame(tick);
  }, tableSelector);
}

async function stopOpportunityHotPathProbe(page) {
  return await page.evaluate(() => {
    const runtime = window as any;
    const probe = runtime.__crosslineOpportunityHotPathProbe;
    if (!probe?.stop) return { missing: true };
    const report = probe.stop();
    delete runtime.__crosslineOpportunityHotPathProbe;
    return { missing: false, ...report };
  });
}

async function expectVisualShellStable(page, label) {
  const report = await page.evaluate(() => {
    const body = document.body;
    const rootOverflow = Math.max(
      document.documentElement.scrollWidth,
      body?.scrollWidth ?? 0,
    ) - window.innerWidth;
    const emptySurfaces = Array.from(document.querySelectorAll(".surface"))
      .filter((element) => {
        const rect = element.getBoundingClientRect();
        return rect.width > 0 && rect.height > 0;
      })
      .filter((element) => (element.textContent ?? "").trim().length === 0)
      .map((element) => element.className);
    return { emptySurfaces, rootOverflow };
  });
  expect(report.rootOverflow, `${label} document horizontal overflow`).toBeLessThanOrEqual(2);
  expect(report.emptySurfaces, `${label} blank visible surfaces`).toEqual([]);
}

async function expectHeaderVisibleAfterContentScroll(page, tableSelector, label) {
  await page.evaluate((selector) => {
    const table = document.querySelector(selector);
    const scroller = table?.closest(".table-wrap");
    if (scroller) scroller.scrollTop = 220;
  }, tableSelector);
  await page.waitForTimeout(50);
  const report = await page.evaluate((selector) => {
    const table = document.querySelector(selector);
    const content = table?.closest(".table-wrap");
    const header = document.querySelector(`${selector} thead th`);
    if (!content || !header) return { ok: false, reason: "missing content or header" };
    const contentRect = content.getBoundingClientRect();
    const headerRect = header.getBoundingClientRect();
    return {
      ok: headerRect.bottom > contentRect.top
        && headerRect.top < contentRect.bottom
        && headerRect.right > contentRect.left
        && headerRect.left < contentRect.right,
      reason: `header=${headerRect.top}:${headerRect.bottom} content=${contentRect.top}:${contentRect.bottom}`,
    };
  }, tableSelector);
  expect(report.ok, `${label} sticky header visibility ${report.reason}`).toBeTruthy();
}

test.describe("exchange data pipeline", () => {
  test.use({
    storageState: {
      cookies: [],
      origins: [
        {
          origin: WEB_BASE,
          localStorage: [
            { name: "api_base", value: JSON.stringify(API_BASE) },
            { name: "api_auth_token", value: JSON.stringify("e2e-token") },
          ],
        },
      ],
    },
  });

  test("portfolio first frame stays within budget after module switch", async ({ page }) => {
    await page.goto("/#futures");
    await expectFuturesReady(page);

    await page.evaluate(() => {
      window.performance.mark("portfolio-switch-start");
    });
    await page.getByRole("button", { name: "持仓/风控" }).click();
    await expect(page.getByText("交易所可用保证金")).toBeVisible();
    await expect(page.locator(".balance-row").filter({ hasText: "mock" })).toHaveCount(1);

    const duration = await page.evaluate(() => {
      const entries = window.performance.getEntriesByName("portfolio-switch-start");
      const start = entries[entries.length - 1];
      return start ? window.performance.now() - start.startTime : Number.POSITIVE_INFINITY;
    });
    expect(duration).toBeLessThan(PORTFOLIO_FIRST_FRAME_BUDGET_MS);
  });

  test("positions page surfaces single venue position failure without dropping balances", async ({
    page,
  }) => {
    await page.goto("/#positions");
    await expect(page.locator("h1", { hasText: "持仓/风控" })).toBeVisible();

    const banner = page.locator(".runtime-problems");
    await expect(banner).toBeVisible();
    await expect(banner).toContainText("数据降级");
    await expect(banner).toContainText("gate · portfolio/positions");
    await expect(banner).toContainText("gate · positions · WARN");
    await expect(banner).toHaveAttribute(
      "title",
      /Gate positions read failed; other venues retained/,
    );
    await expect(page.locator(".balance-row").filter({ hasText: "mock" })).toHaveCount(1);
  });

  test("positions page surfaces portfolio snapshot LoadState failure", async ({ page }) => {
    await useScenarioApiBase(page, "e2e-portfolio-snapshot-502");
    const snapshotFailure = page.waitForResponse((response) =>
      response.url().includes("/e2e-portfolio-snapshot-502/api/trading/portfolio/snapshot")
      && response.status() === 502,
    );

    await page.goto("/#positions");
    await expect(page.locator("h1", { hasText: "持仓/风控" })).toBeVisible();
    await snapshotFailure;

    const banner = page.locator(".runtime-problems");
    await expect(banner).toBeVisible();
    await expect(banner).toContainText("数据降级");
    await expect(banner).toContainText("请求失败 · PORTFOLIO_SNAPSHOT_UNAVAILABLE");
    await expect(banner).toHaveAttribute("title", /mock portfolio snapshot unavailable/);
    await expect(banner).toHaveAttribute("title", /HTTP 502/);
    await expect(banner).toHaveAttribute("title", /request_id e2e-portfolio-snapshot-502/);
    await expect(banner).toHaveAttribute("title", /retry 6000ms/);

    await expect(page.locator(".summary-state-card")).toContainText(
      "账户概览读取失败：mock portfolio snapshot unavailable",
    );
    await expect(page.locator(".summary-state-card")).toContainText("HTTP 502");
    await expect(page.locator(".positions-table .empty-cell")).toContainText(
      "持仓读取失败：mock portfolio snapshot unavailable",
    );
    await expect(page.locator(".positions-table .empty-cell")).toContainText(
      "request_id e2e-portfolio-snapshot-502",
    );
    await expect(page.locator(".risk-empty").filter({ hasText: "风险快照读取失败" }))
      .toContainText("HTTP 502");
    await expect(page.locator(".risk-empty").filter({ hasText: "余额读取失败" }))
      .toContainText("retry 6000ms");
    await expect(page.getByText("暂无持仓")).toHaveCount(0);
    await expect(page.getByText("暂无可用余额")).toHaveCount(0);
    await expect(page.locator(".balance-row")).toHaveCount(0);
  });

  test("positions close denial keeps the position and typed retry context", async ({ page }) => {
    await useScenarioApiBase(page, "e2e-position-close-denied");
    await page.goto("/#positions");
    await expect(page.locator("h1", { hasText: "持仓/风控" })).toBeVisible();

    const positionRow = page
      .locator(".positions-table tbody tr")
      .filter({ hasText: "mock" })
      .filter({ hasText: "MU" });
    await expect(positionRow).toHaveCount(1);
    const closeFailure = page.waitForResponse((response) =>
      response.url().includes(
        "/e2e-position-close-denied/api/trading/portfolio/positions/mock/MU/close",
      )
      && response.request().method() === "POST"
      && response.status() === 409,
    );
    await positionRow.getByRole("button", { name: "平仓", exact: true }).click();
    await closeFailure;

    const message = page.locator(".positions-main .positions-action-message");
    await expect(message).toContainText("平仓失败：position close denied by snapshot policy");
    await expect(message).toContainText("code POSITION_CLOSE_DENIED");
    await expect(message).toContainText("HTTP 409");
    await expect(message).toContainText("request_id req-position-close-denied");
    await expect(message).toContainText("retry 10000ms");
    await expect(positionRow).toHaveCount(1);
    await expect(positionRow.getByRole("button", { name: "平仓", exact: true })).toBeEnabled();
    await expect(message).not.toContainText("已完成");
  });

  test("positions compensation cancel denial keeps CloseRun accepted and retryable", async ({
    page,
  }) => {
    await useScenarioApiBase(page, "e2e-position-cancel-denied");
    await page.goto("/#positions");
    await expect(page.locator("h1", { hasText: "持仓/风控" })).toBeVisible();

    const closeRunRow = page
      .locator(".close-runs-table tbody tr")
      .filter({ hasText: "e2e-close-run-cancel-denied" });
    await expect(closeRunRow).toHaveCount(1);
    await expect(closeRunRow).toContainText("补偿中");
    await expect(closeRunRow).toContainText("positions-cancel-v1");
    const cancelFailure = page.waitForResponse((response) =>
      response.url().includes(
        "/e2e-position-cancel-denied/api/trading/orders/e2e-comp-order-1/cancel",
      )
      && response.request().method() === "POST"
      && response.status() === 409,
    );
    await closeRunRow.getByRole("button", { name: "撤补买" }).click();
    await cancelFailure;

    const message = page.locator(".close-runs-panel .positions-action-message");
    await expect(message).toContainText(
      "补偿撤单失败：compensation cancel denied while order remains accepted",
    );
    await expect(message).toContainText("code COMPENSATION_CANCEL_DENIED");
    await expect(message).toContainText("HTTP 409");
    await expect(message).toContainText("request_id req-position-cancel-denied");
    await expect(message).toContainText("retry 9000ms");
    await expect(closeRunRow).toContainText("补偿中");
    await expect(closeRunRow.getByRole("button", { name: "撤补买" })).toBeEnabled();
    await expect(message).not.toContainText("已确认取消");
  });

  test("hedge preview uses fresh ticket data without rate-limit blocker noise", async ({ page }) => {
    await page.goto("/#futures");
    await expectFuturesReady(page);
    await expect(page.locator(".futures-table tbody tr").filter({ hasText: "MU" })).toHaveCount(1);

    const build = page.getByRole("button", { name: "构建对冲" });
    await expect(build).toHaveCount(1);
    const previewResponse = page.waitForResponse((response) =>
      response.url().includes("/api/arbitrage/opportunities/")
      && response.url().endsWith("/preview"),
    );
    await build.click();
    expect((await previewResponse).ok()).toBeTruthy();

    await expect(page.getByText("执行草案")).toBeVisible();
    const riskSection = page.locator(".execution-risk-section");
    await riskSection.locator(".execution-evidence-details summary").click();
    await expect(
      riskSection.locator(".risk-notes").getByText("后端预检", { exact: true }),
    ).toBeVisible();
    await expect(riskSection.getByText("RiskDecision", { exact: true })).toBeVisible();
    await expect(
      riskSection.locator(".execution-section-head").getByText("通过", { exact: true }),
    ).toBeVisible();
    await expect(page.getByText("HedgeTicket 阻断 4 条")).toHaveCount(0);
    await expect(page.getByText("rate limited")).toHaveCount(0);
  });

  test("hedge preview rate-limit surfaces typed preview error", async ({ page }) => {
    await page.route("**/api/arbitrage/opportunities/*/preview", async (route) => {
      const headers = routeJsonHeaders("POST,OPTIONS", {
        "retry-after": "2",
        "x-request-id": "req-preview",
      });
      if (route.request().method() === "OPTIONS") {
        await route.fulfill({ status: 204, headers, body: "" });
        return;
      }
      await route.fulfill({
        status: 429,
        headers,
        body: JSON.stringify({
          error: {
            code: "MARKET_DATA_RATE_LIMITED",
            message: "preview rate limited",
            status: 429,
            source: "e2e-route",
          },
        }),
      });
    });

    await page.goto("/#futures");
    await expectFuturesReady(page);
    await page.getByRole("button", { name: "构建对冲" }).click();

    const riskSection = page.locator(".execution-risk-section");
    await expect(riskSection.locator(".risk-empty.stale-note")).toContainText(
      "预览已失效：preview rate limited · code MARKET_DATA_RATE_LIMITED · source e2e-route · HTTP 429 · request_id req-preview · retry 2000ms",
    );
    await expect(
      riskSection.locator(".execution-section-head").getByText("失效", { exact: true }),
    ).toBeVisible();
  });

  test("hedge confirm rate-limit surfaces typed submit error", async ({ page }) => {
    await useScenarioApiBase(page, "e2e-confirm-429");
    await page.goto("/#futures");
    await expectFuturesReady(page);
    await page.getByRole("button", { name: "构建对冲" }).click();

    const riskSection = page.locator(".execution-risk-section");
    await expect(
      riskSection.locator(".execution-section-head").getByText("通过", { exact: true }),
    ).toBeVisible();
    const confirmResponse = page.waitForResponse((response) =>
      response.url().includes("/e2e-confirm-429/api/arbitrage/opportunities/mock-mu-perp/confirm")
      && response.status() === 429,
    );
    await page.getByRole("button", { name: "提交 模拟" }).click();
    await confirmResponse;

    const actionBar = page.locator(".execution-actionbar");
    await expect(actionBar.locator(".run-state")).toContainText("提交失败");
    await expect(actionBar.locator(".run-state")).toContainText("hedge confirm rate limited");
    await expect(actionBar.locator(".run-state")).toContainText("code HEDGE_CONFIRM_RATE_LIMITED");
    await expect(actionBar.locator(".run-state")).toContainText("HTTP 429");
    await expect(actionBar.locator(".run-state")).toContainText("request_id req-confirm-429");
    await expect(actionBar.locator(".run-state")).toContainText("retry 7000ms");
  });

  test("hedge confirm HTTP 200 failed outcome surfaces typed body context", async ({ page }) => {
    await useScenarioApiBase(page, "e2e-confirm-200-failed");
    await page.goto("/#futures");
    await expectFuturesReady(page);
    await page.getByRole("button", { name: "构建对冲" }).click();

    const riskSection = page.locator(".execution-risk-section");
    await expect(
      riskSection.locator(".execution-section-head").getByText("通过", { exact: true }),
    ).toBeVisible();
    const confirmResponse = page.waitForResponse((response) =>
      response
        .url()
        .includes("/e2e-confirm-200-failed/api/arbitrage/opportunities/mock-mu-perp/confirm")
      && response.status() === 200,
    );
    await page.getByRole("button", { name: "提交 模拟" }).click();
    await confirmResponse;

    const actionBar = page.locator(".execution-actionbar");
    await expect(actionBar.locator(".run-state")).toContainText("多腿提交失败");
    await expect(actionBar.locator(".run-state")).toContainText("Idempotency preview-mu-001");
    await expect(actionBar.locator(".run-state")).toContainText(
      "hedge confirm accepted transport but failed",
    );
    await expect(actionBar.locator(".run-state")).toContainText(
      "code HEDGE_CONFIRM_HTTP_200_FAILED",
    );
    await expect(actionBar.locator(".run-state")).toContainText("HTTP 200");
    await expect(actionBar.locator(".run-state")).toContainText(
      "request_id req-confirm-200-failed",
    );
    await expect(actionBar.locator(".run-state")).toContainText("retry 7000ms");
  });

  test("hedge confirm policy denial preserves typed error without a fake run state", async ({
    page,
  }) => {
    await useScenarioApiBase(page, "e2e-confirm-denied");
    await page.goto("/#futures");
    await expectFuturesReady(page);
    await page.getByRole("button", { name: "构建对冲" }).click();

    const confirmFailure = page.waitForResponse((response) =>
      response.url().includes(
        "/e2e-confirm-denied/api/arbitrage/opportunities/mock-mu-perp/confirm",
      )
      && response.request().method() === "POST"
      && response.status() === 403,
    );
    await page.getByRole("button", { name: "提交 模拟" }).click();
    await confirmFailure;

    const actionBar = page.locator(".execution-actionbar");
    await expect(actionBar.locator(".run-state")).toContainText("提交失败");
    await expect(actionBar.locator(".run-state")).toContainText(
      "hedge confirm denied by execution policy",
    );
    await expect(actionBar.locator(".run-state")).toContainText("code HEDGE_CONFIRM_DENIED");
    await expect(actionBar.locator(".run-state")).toContainText("source execution_policy");
    await expect(actionBar.locator(".run-state")).toContainText("HTTP 403");
    await expect(actionBar.locator(".run-state")).toContainText("request_id req-confirm-denied");
    await expect(actionBar.locator(".run-state")).toContainText("retry 12000ms");
    await expect(actionBar).not.toContainText("等待成交确认");
    await expect(actionBar).not.toContainText("双腿完成");
    await expect(actionBar.getByRole("button", { name: "撤单", exact: true })).toBeDisabled();
  });

  test("execution order cancel denial keeps the pending run and typed retry context", async ({
    page,
  }) => {
    await useScenarioApiBase(page, "e2e-order-cancel-denied");
    await page.goto("/#futures");
    await expectFuturesReady(page);
    await page.getByRole("button", { name: "构建对冲" }).click();

    const confirm = page.waitForResponse((response) =>
      response.url().includes(
        "/e2e-order-cancel-denied/api/arbitrage/opportunities/mock-mu-perp/confirm",
      )
      && response.request().method() === "POST"
      && response.status() === 200,
    );
    await page.getByRole("button", { name: "提交 模拟" }).click();
    await confirm;

    const actionBar = page.locator(".execution-actionbar");
    await expect(actionBar.locator(".run-state > span")).toContainText("等待成交确认");
    const cancel = actionBar.getByRole("button", { name: "撤单", exact: true });
    await expect(cancel).toBeEnabled();
    const cancelFailure = page.waitForResponse((response) =>
      response.url().includes(
        "/e2e-order-cancel-denied/api/trading/orders/e2e-run-order-1/cancel",
      )
      && response.request().method() === "POST"
      && response.status() === 409,
    );
    await cancel.click();
    await cancelFailure;

    const remedy = actionBar.locator(".remedy-state");
    await expect(remedy).toContainText(
      "撤单失败：execution order cancel denied while venue finality is pending",
    );
    await expect(remedy).toContainText("code ORDER_CANCEL_DENIED");
    await expect(remedy).toContainText("source execution.cancel_order:e2e-run-order-1");
    await expect(remedy).toContainText("HTTP 409");
    await expect(remedy).toContainText("request_id req-execution-cancel-denied");
    await expect(remedy).toContainText("retry 8000ms");
    await expect(actionBar.locator(".run-state > span")).toContainText("等待成交确认");
    await expect(actionBar).not.toContainText("双腿完成");
    await expect(remedy).not.toContainText("已提交撤单");
  });

  test("opportunities list rate-limit surfaces LoadState error", async ({ page }) => {
    await page.route("**/api/v3/arbitrage/opportunities/list**", async (route) => {
      const headers = routeJsonHeaders("GET,OPTIONS", {
        "retry-after": "2",
        "x-request-id": "req-opp-list",
      });
      if (route.request().method() === "OPTIONS") {
        await route.fulfill({ status: 204, headers, body: "" });
        return;
      }
      await route.fulfill({
        status: 429,
        headers,
        body: JSON.stringify({
          error: {
            code: "MARKET_DATA_RATE_LIMITED",
            message: "opportunity list rate limited",
            status: 429,
            source: "e2e-route",
          },
        }),
      });
    });

    await page.goto("/#opportunities");
    await expect(page.getByRole("heading", { name: "机会扫描" })).toBeVisible();
    await expect(page.getByRole("heading", { name: "候选机会", exact: true })).toBeVisible();

    const expected =
      "opportunity list rate limited · source e2e-route · HTTP 429 · request_id req-opp-list · retry 2000ms";
    await expect(page.locator(".settings-message.is-error")).toContainText(
      `机会快照冷启动失败 · ${expected}`,
    );
    await expect(page.locator(".empty-cell")).toContainText(`机会快照错误 · ${expected}`);
    await expect(page.locator(".opportunity-table-tools")).toContainText("错误");
    await expect(page.locator(".clean-table tbody tr").filter({ hasText: "MU" })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "构建对冲" })).toHaveCount(0);
  });

  test("scanner fast route and metrics respond inside the data-pipeline budget", async ({
    request,
  }) => {
    const start = performance.now();
    const opportunities = await request.get(
      `${API_BASE}/api/v3/arbitrage/opportunities/list?pageSize=50&fast=true&sortKey=score&strategy=${STRATEGY_QUERY}`,
      { timeout: 8_000 },
    );
    expect(opportunities.ok()).toBeTruthy();
    expect(performance.now() - start).toBeLessThan(8_000);

    const metrics = await request.get(`${API_BASE}/metrics`, { timeout: 5_000 });
    expect(metrics.ok()).toBeTruthy();
    expect(await metrics.text()).toContain("crypto_arb_market_cache_hit_ratio");
    expect(await metrics.text()).toContain("crypto_arb_http_requests_total");
  });

  test("scanner rate-limit fixture returns typed ApiProblem and retry-after", async ({
    request,
  }) => {
    const response = await request.get(
      `${API_BASE}/api/v3/arbitrage/opportunities/list?pageSize=50&fast=true&sortKey=score&strategy=${STRATEGY_QUERY}&e2eScenario=typed-rate-limit`,
      { timeout: 8_000 },
    );
    expect(response.status()).toBe(429);
    expect(response.headers()["retry-after"]).toBe("2");
    expect(response.headers()["x-request-id"]).toBe("e2e-rate-limit-1");
    expect(response.headers()["access-control-expose-headers"]).toContain("retry-after");
    expect(response.headers()["access-control-expose-headers"]).toContain("x-request-id");

    const body = await response.json();
    expect(body.error).toMatchObject({
      code: "MARKET_DATA_RATE_LIMITED",
      message: "mock opportunity list rate limited",
      status: 429,
      requestId: "e2e-rate-limit-1",
      source: "e2e-fixture",
    });
    expect(body.error.retryAfterMs).toBe(2000);
    expect(body.error.details).toMatchObject({
      venue: "mock",
      operation: "opportunity_list",
      path: "/api/v3/arbitrage/opportunities/list",
      status: 429,
      source: "e2e-fixture",
      scenario: "typed-rate-limit",
    });
    expect(body.rows).toBeUndefined();
    expect(body.opportunities).toBeUndefined();
  });

  test("mock runtime contract smoke uses backend envelopes", async ({ request }) => {
    const portfolio = await request.get(`${API_BASE}/api/trading/portfolio/snapshot`, {
      timeout: 5_000,
    });
    expect(portfolio.ok()).toBeTruthy();
    const portfolioBody = await portfolio.json();
    expect(portfolioBody).toMatchObject({
      status: "fresh",
      source: "e2e-fixture",
      observedAtMs: 1_770_000_000_000,
    });
    expect(portfolioBody.snapshot.summary.totalNavUsd).toBe(10_000);
    expect(portfolioBody.operationHealth).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ venue: "mock", operation: "balance", status: "ok" }),
        expect.objectContaining({ venue: "gate", operation: "positions", status: "warn" }),
      ]),
    );
    expect(portfolioBody.summary).toBeUndefined();

    const accountState = await request.get(`${API_BASE}/api/trading/account-state`, {
      timeout: 5_000,
    });
    expect(accountState.ok()).toBeTruthy();
    const accountStateBody = await accountState.json();
    expect(accountStateBody).toMatchObject({
      status: "degraded",
      source: "account_state_runtime",
      observedAtMs: 1_770_000_000_000,
      balances: {
        rowCount: 1,
        rows: [expect.objectContaining({ venue: "mock", currency: "USDC" })],
      },
    });
    expect(accountStateBody.snapshot).toBeUndefined();

    const detail = await request.get(
      `${API_BASE}/api/v3/arbitrage/opportunities/mock-mu-perp/detail`,
      { timeout: 5_000 },
    );
    expect(detail.ok()).toBeTruthy();
    const detailBody = await detail.json();
    expect(detailBody).toMatchObject({
      status: "fresh",
      source: "e2e-fixture",
      requestId: "e2e-detail-1",
    });
    expect(detailBody.opportunity.id).toBe("mock-mu-perp");
    expect(detailBody.longOrderbook.health.quality).toBe("fresh");
    expect(detailBody.shortOrderbook.health.quality).toBe("fresh");
    expect(detailBody.history.rows[0].id).toBe("mock-mu-perp");
    expect(detailBody.partialFailures).toEqual([]);
    expect(detailBody.id).toBeUndefined();
  });

  test("scanner upstream failure fixture returns typed ApiProblem without fake rows", async ({
    request,
  }) => {
    const response = await request.get(
      `${API_BASE}/api/v3/arbitrage/opportunities/list?pageSize=50&fast=true&sortKey=score&strategy=${STRATEGY_QUERY}&e2eScenario=typed-upstream-502`,
      { timeout: 8_000 },
    );
    expect(response.status()).toBe(502);
    expect(response.headers()["x-request-id"]).toBe("e2e-upstream-502");
    expect(response.headers()["retry-after"]).toBeUndefined();

    const body = await response.json();
    expect(body.error).toMatchObject({
      code: "UPSTREAM_HTTP",
      message: "mock upstream gateway failed",
      status: 502,
      requestId: "e2e-upstream-502",
      source: "e2e-fixture",
    });
    expect(body.error.retryAfterMs).toBeUndefined();
    expect(body.error.details).toMatchObject({
      route: "/api/v3/arbitrage/opportunities/list",
      scenario: "typed-upstream-502",
    });
    expect(body.rows).toBeUndefined();
    expect(body.opportunities).toBeUndefined();
  });

  test("scanner auth failure fixture returns typed ApiProblem without fake rows", async ({
    request,
  }) => {
    const response = await request.get(
      `${API_BASE}/e2e-auth-401/api/v3/arbitrage/opportunities/list?pageSize=50&fast=true&sortKey=score&strategy=${STRATEGY_QUERY}`,
      { timeout: 8_000 },
    );
    expect(response.status()).toBe(401);
    expect(response.headers()["x-request-id"]).toBe("e2e-auth-401");

    const body = await response.json();
    expect(body.error).toMatchObject({
      code: "UNAUTHORIZED",
      message: "mock auth token missing",
      status: 401,
      requestId: "e2e-auth-401",
      source: "e2e-fixture",
    });
    expect(body.error.details).toMatchObject({
      route: "/api/v3/arbitrage/opportunities/list",
      scenario: "typed-auth-401",
    });
    expect(body.rows).toBeUndefined();
    expect(body.opportunities).toBeUndefined();
  });

  test("opportunities page surfaces auth failure without fake executable rows", async ({ page }) => {
    await useScenarioApiBase(page, "e2e-auth-401");
    await page.goto("/#opportunities");
    await expect(page.getByRole("heading", { name: "机会扫描" })).toBeVisible();

    await expect(page.locator(".settings-message.is-error")).toContainText(
      "机会快照冷启动失败 · mock auth token missing",
    );
    await expect(page.locator(".settings-message.is-error")).toContainText("HTTP 401");
    await expect(page.locator(".settings-message.is-error")).toContainText(
      "request_id e2e-auth-401",
    );
    await expect(page.locator(".empty-cell")).toContainText("机会快照错误");
    await expect(page.locator(".clean-table tbody tr").filter({ hasText: "MU" })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "构建对冲" })).toHaveCount(0);
  });

  test("opportunities page surfaces websocket decode failure and keeps REST fallback rows", async ({
    page,
  }) => {
    await useScenarioApiBase(page, "e2e-ws-decode");
    await page.goto("/#opportunities");
    await expect(page.getByRole("heading", { name: "机会扫描" })).toBeVisible();

    await expect(page.locator(".settings-message.is-error")).toContainText(
      "套利WS异常 · ws[arbitrage]",
    );
    await expect(page.locator(".settings-message.is-error")).toContainText("source frontend-ws");
    await expect(page.locator(".settings-message.is-error")).toContainText("帧 0 · 错误 1");
    await expect(page.locator(".clean-table tbody tr").filter({ hasText: "MU" })).toHaveCount(1);
  });

  test("opportunities page surfaces websocket server error and keeps REST fallback rows", async ({
    page,
  }) => {
    await useScenarioApiBase(page, "e2e-ws-error");
    await page.goto("/#opportunities");
    await expect(page.getByRole("heading", { name: "机会扫描" })).toBeVisible();

    await expect(page.locator(".settings-message.is-error")).toContainText(
      "套利WS异常 · ws[arbitrage] mock ws channel rejected",
    );
    await expect(page.locator(".settings-message.is-error")).toContainText(
      "request_id e2e-ws-error",
    );
    await expect(page.locator(".settings-message.is-error")).toContainText("retry 2000ms");
    await expect(page.locator(".settings-message.is-error")).toContainText("帧 0 · 错误 1");
    await expect(page.locator(".clean-table tbody tr").filter({ hasText: "MU" })).toHaveCount(1);
  });

  test("opportunities websocket authenticates with ticket before subscribe", async ({
    page,
    request,
  }) => {
    await request.delete(`${API_BASE}/api/e2e/ws-auth-events`);
    await useScenarioApiBaseWithAuth(page, "e2e-ws-auth", "e2e-token");

    await page.goto("/#opportunities");
    await expect(page.getByRole("heading", { name: "机会扫描" })).toBeVisible();
    await expect(page.locator(".clean-table tbody tr").filter({ hasText: "MU" })).toHaveCount(1);

    await expect
      .poll(async () => {
        const response = await request.get(`${API_BASE}/api/e2e/ws-auth-events`);
        const body = await response.json();
        return body.events
          .filter((event) => event.scenario === "ws-auth")
          .map((event) => event.type)
          .join(",");
      })
      .toContain("ws_ticket,auth,subscribe");

    const response = await request.get(`${API_BASE}/api/e2e/ws-auth-events`);
    const body = await response.json();
    const authEvents = body.events.filter((event) => event.scenario === "ws-auth");
    expect(authEvents[0]).toMatchObject({ type: "ws_ticket", authorized: true });
    expect(authEvents[1]).toMatchObject({ type: "auth", ticket: "e2e-ws-ticket" });
    expect(authEvents[2]).toMatchObject({
      type: "subscribe",
      authenticated: true,
    });
    expect(authEvents[2].channels).toContain("arbitrage");
  });

  test("settings diagnostics surfaces private order stream runtime warning", async ({ page }) => {
    await page.goto("/#settings");
    await expect(page.locator("h1", { hasText: "设置" })).toBeVisible();

    await page.getByRole("tab", { name: "诊断" }).click();
    await expect(page.getByText("运行态矩阵")).toBeVisible();

    await page.getByLabel("搜索状态").fill("private_ws_order_stream");
    const row = page
      .locator("tr")
      .filter({ hasText: "hyperliquid:km" })
      .filter({ hasText: "private_ws_order_stream" })
      .filter({ hasText: "存在 2 笔未决实盘订单" })
      .filter({ hasText: "0/2" })
      .first();

    await expect(row).toBeVisible();
    await expect(row).toContainText("观察");
    await expect(row).toContainText("private_ws_runtime");
    await expect(row).toContainText("0/2");
    await expect(row).toContainText("存在 2 笔未决实盘订单");
    await expect(row).toContainText("retry 60000ms");
    await expect(row).toContainText("官方 order_state_stream / private_ws_runtime/order_stream");
    await expect(page.locator(".settings-summary-line").filter({ hasText: "运行态矩阵" }))
      .toContainText("需关注 4");
  });

  test("settings diagnostics surfaces order finality runtime warning", async ({ page }) => {
    await page.goto("/#settings");
    await expect(page.locator("h1", { hasText: "设置" })).toBeVisible();

    await page.getByRole("tab", { name: "诊断" }).click();
    await expect(page.getByText("运行态矩阵")).toBeVisible();

    await page.getByLabel("搜索状态").fill("order_finality");
    const row = page
      .locator("tr")
      .filter({ hasText: "mock" })
      .filter({ hasText: "order_finality" });

    await expect(row).toBeVisible();
    await expect(row).toContainText("观察");
    await expect(row).toContainText("run_finality_runtime");
    await expect(row).toContainText("0/1");
    await expect(row).toContainText("订单终态回查完成");
    await expect(row).toContainText("HEDGE_ORDER_FINALITY_FAILED");
    await expect(row).toContainText("request_id req-order-finality-retry");
    await expect(row).toContainText("retry 90000ms");
    await expect(row)
      .toContainText("官方 order_state/execution_run_finality/close_run_finality / order_finality");
    await expect(page.locator(".settings-summary-line").filter({ hasText: "运行态矩阵" }))
      .toContainText("匹配 1 / 6 条");
    await expect(page.locator(".settings-summary-line").filter({ hasText: "运行态矩阵" }))
      .toContainText("需关注 4");
  });

  test("settings diagnostics surfaces private ws auth failure", async ({ page }) => {
    await page.goto("/#settings");
    await expect(page.locator("h1", { hasText: "设置" })).toBeVisible();

    await page.getByRole("tab", { name: "诊断" }).click();
    await expect(page.getByText("运行态矩阵")).toBeVisible();

    await page.getByLabel("搜索状态").fill("auth_status=failed");
    const row = page
      .locator("tr")
      .filter({ hasText: "bybit" })
      .filter({ hasText: "private_ws_subscribe" })
      .filter({ hasText: "Bybit private WS auth failed" })
      .filter({ hasText: "0/1" })
      .first();

    await expect(row).toBeVisible();
    await expect(row).toContainText("阻断");
    await expect(row).toContainText("private_ws_runtime");
    await expect(row).toContainText("0/1");
    await expect(row).toContainText("Bybit private WS auth failed");
    await expect(row).toContainText("retry 30000ms");
    await expect(row).toContainText("官方 orders / private_ws_auth/order_stream");
    await expect(page.locator(".settings-summary-line").filter({ hasText: "运行态矩阵" }))
      .toContainText("匹配 1 / 6 条");
    await expect(page.locator(".settings-summary-line").filter({ hasText: "运行态矩阵" }))
      .toContainText("需关注 4");
  });

  test("settings diagnostics surfaces credential validation failure row", async ({ page }) => {
    await page.goto("/#settings");
    await expect(page.locator("h1", { hasText: "设置" })).toBeVisible();

    await page.getByRole("tab", { name: "诊断" }).click();
    await expect(page.getByText("运行态矩阵")).toBeVisible();

    await page.getByLabel("搜索状态").fill("CREDENTIAL_PERMISSION_DENIED");
    const row = page
      .locator("tr")
      .filter({ hasText: "okx" })
      .filter({ hasText: "credential_probe:balance_read" });

    await expect(row).toBeVisible();
    await expect(row).toContainText("阻断");
    await expect(row).toContainText("credential_validation");
    await expect(row).toContainText("静态字段完整");
    await expect(row).toContainText("OKX balance read permission denied");
    await expect(row).toContainText("官方 account / credential_validation");
    await expect(page.locator(".settings-summary-line").filter({ hasText: "运行态矩阵" }))
      .toContainText("匹配 1 / 6 条");
    await expect(page.locator(".settings-summary-line").filter({ hasText: "运行态矩阵" }))
      .toContainText("需关注 4");
  });

  test("settings credentials keep static adapter copy separate from runtime readiness", async ({
    page,
  }) => {
    await useScenarioApiBase(page, "e2e-pr-fu-settings-environment");
    const credentials = page.waitForResponse((response) =>
      response.url().includes("/e2e-pr-fu-settings-environment/api/exchanges/credentials")
      && response.status() === 200,
    );
    const operationHealth = page.waitForResponse((response) =>
      response.url().includes("/e2e-pr-fu-settings-environment/api/system/venue-operation-health")
      && response.status() === 200,
    );

    await page.goto("/#settings");
    await expect(page.locator("h1", { hasText: "设置" })).toBeVisible();
    await credentials;
    await operationHealth;

    const summary = page.locator(".credential-summary").filter({ hasText: "当前能力" });
    await expect(summary).toContainText(/6\/6 字段已填写/);
    await expect(summary).toContainText(/未验证（缺少探针）: 订单权限\/账户模式/);
    await expect(summary).toContainText(/当前状态待运行态证据/);
    await expect(summary).toContainText(/静态写侧声明/);
    await expect(summary).not.toContainText(/权限验证完整|可下单|实盘就绪/);

    const staticPanel = page.locator(".runtime-health-panel").filter({ hasText: "静态能力证据" });
    await expect(staticPanel).toContainText("静态写单声明");
    await expect(staticPanel).toContainText("保存期探针与运行态证据决定是否可提交");
    await expect(staticPanel).not.toContainText("权限验证完整");
    await expect(staticPanel).not.toContainText("可下单");

    const validationPanel = page.locator(".runtime-health-panel").filter({ hasText: "保存期验证" });
    await expect(validationPanel).toContainText("权限未完整");
    await expect(validationPanel).toContainText("safe/noop probe 未授予 live_write");
    await expect(validationPanel).not.toContainText("权限验证完整");

    const permissionRow = page
      .locator("tr")
      .filter({ hasText: "下单/撤单权限" })
      .filter({ hasText: "credential_probe:order_permission" });
    await expect(permissionRow).toContainText("待验证");
    await expect(permissionRow).toContainText("credential_validation");
    await expect(permissionRow).toContainText("safe/noop probe 未授予 live_write");
    await expect(permissionRow).toContainText("does_not_grant_live_write=true");

    const writeRow = page.locator("tr").filter({ hasText: "order_write" });
    await expect(writeRow).toContainText("写单运行态");
    await expect(writeRow).toContainText("待证据");
    await expect(writeRow).toContainText("live place/cancel/finality 证据");
    await expect(writeRow).not.toContainText("可下单");
    await expect(writeRow).not.toContainText("权限验证完整");

    await expect(page.getByText("okx · 当前运行态 0/4 正常 · 4/4 待处理"))
      .toBeVisible();

    const privateOrderStreamRow = page
      .locator("tr")
      .filter({ hasText: "私有订单流" })
      .filter({ hasText: "private_ws_order_stream" });
    await expect(privateOrderStreamRow).toContainText("待证据");
    await expect(privateOrderStreamRow).toContainText("private_ws_runtime");
    await expect(privateOrderStreamRow).toContainText("request_id -");
    await expect(privateOrderStreamRow).toContainText("okx 暂无私有订单流运行态记录");
    await expect(privateOrderStreamRow).toContainText("私有订单事件流需要运行态样本");
    await expect(privateOrderStreamRow).not.toContainText("可下单");
    await expect(privateOrderStreamRow).not.toContainText("权限验证完整");

    const orderFinalityRow = page
      .locator("tr")
      .filter({ hasText: "订单终态" })
      .filter({ hasText: "order_finality" });
    await expect(orderFinalityRow).toContainText("待证据");
    await expect(orderFinalityRow).toContainText("run_finality");
    await expect(orderFinalityRow).toContainText("request_id -");
    await expect(orderFinalityRow).toContainText("okx 暂无订单终态回查运行态记录");
    await expect(orderFinalityRow).toContainText("未决订单产生后由 REST/WS 终态回查写入");
    await expect(orderFinalityRow).not.toContainText("可下单");
    await expect(orderFinalityRow).not.toContainText("权限验证完整");

    const adapters = page.waitForResponse((response) =>
      response.url().includes("/e2e-pr-fu-settings-environment/api/trading/adapters")
      && response.status() === 200,
    );
    await expect(page.getByRole("button", { name: "执行模式" })).toHaveCount(0);
    await page.getByRole("tab", { name: "诊断" }).click();
    await adapters;

    await expect(page.getByText("执行环境诊断", { exact: true })).toBeVisible();
    const environmentTable = page.locator('[data-settings-table="execution-environment"]');
    await expect(environmentTable).toBeVisible();
    await expect(environmentTable).not.toContainText("Paper");

    const liveAdapterRow = environmentTable
      .locator("tr")
      .filter({ hasText: "live_router" })
      .filter({ hasText: "实盘" });
    await expect(liveAdapterRow).toContainText("字段组已补齐");
    await expect(liveAdapterRow).toContainText("可选路由；下单仍需票据级权限与运行态证据");
    await expect(liveAdapterRow).not.toContainText("可下单");
    await expect(liveAdapterRow).not.toContainText("权限验证完整");

    const venueTable = page.locator('[data-settings-table="venue-capabilities"]');
    const venueRow = venueTable
      .locator("tr")
      .filter({ hasText: "okx" })
      .filter({ hasText: "live_adapter.capabilities" });
    await expect(venueRow).toContainText("永续");
    await expect(venueRow).not.toContainText("可下单");
    await expect(venueRow).not.toContainText("权限验证完整");
  });

  test("settings credentials surface private order stream capture readiness", async ({
    page,
  }) => {
    await useScenarioApiBase(page, "e2e-settings-private-order-stream-ok-capture-readiness");
    const credentials = page.waitForResponse((response) =>
      response
        .url()
        .includes(
          "/e2e-settings-private-order-stream-ok-capture-readiness/api/exchanges/credentials",
        )
      && response.status() === 200,
    );
    const operationHealth = page.waitForResponse((response) =>
      response
        .url()
        .includes(
          "/e2e-settings-private-order-stream-ok-capture-readiness/api/system/venue-operation-health",
        )
      && response.status() === 200,
    );

    await page.goto("/#settings");
    await expect(page.locator("h1", { hasText: "设置" })).toBeVisible();
    await credentials;
    await operationHealth;

    await expect(page.getByText("okx · 当前运行态 1/4 正常 · 3/4 待处理"))
      .toBeVisible();

    const tradingRuntimePanel = page
      .locator(".runtime-health-head")
      .filter({ hasText: "交易运行证据" })
      .locator("xpath=..");
    const privateOrderStreamRow = tradingRuntimePanel
      .locator("tr")
      .filter({ hasText: "私有订单流" })
      .filter({ hasText: "private_ws_order_stream" });
    await expect(privateOrderStreamRow).toContainText("正常");
    await expect(privateOrderStreamRow).toContainText("private_ws_runtime");
    await expect(privateOrderStreamRow).toContainText("freshness 800ms");
    await expect(privateOrderStreamRow).toContainText("3/3");
    await expect(privateOrderStreamRow).toContainText(
      "request_id req-settings-private-order-stream-ok",
    );
    await expect(privateOrderStreamRow).toContainText("官方 order_state_stream 样本");
    await expect(privateOrderStreamRow).toContainText(
      "WS wss://ws.okx.com:8443/ws/v5/private#orders",
    );
    await expect(privateOrderStreamRow.locator("td").last()).toHaveAttribute(
      "title",
      /official_evidence=order_state_stream/,
    );
    await expect(privateOrderStreamRow).not.toContainText("可下单");
    await expect(privateOrderStreamRow).not.toContainText("权限验证完整");

    const writeRow = tradingRuntimePanel.locator("tr").filter({ hasText: "order_write" });
    await expect(writeRow).toContainText("待证据");
    await expect(writeRow).toContainText("live place/cancel/finality 证据");
    await expect(writeRow).not.toContainText("可下单");

    const orderFinalityRow = tradingRuntimePanel
      .locator("tr")
      .filter({ hasText: "订单终态" })
      .filter({ hasText: "order_finality" });
    await expect(orderFinalityRow).toContainText("待证据");
    await expect(orderFinalityRow).toContainText("run_finality");
    await expect(orderFinalityRow).toContainText("request_id -");
    await expect(orderFinalityRow).not.toContainText("权限验证完整");
  });

  test("settings credentials surface local capture-shaped trading runtime 4/4 ok gate", async ({
    page,
  }) => {
    await useScenarioApiBase(
      page,
      "e2e-settings-selected-venue-trading-runtime-all-ok-local-capture-shaped",
    );
    const credentials = page.waitForResponse((response) =>
      response
        .url()
        .includes(
          "/e2e-settings-selected-venue-trading-runtime-all-ok-local-capture-shaped/api/exchanges/credentials",
        )
      && response.status() === 200,
    );
    const operationHealth = page.waitForResponse((response) =>
      response
        .url()
        .includes(
          "/e2e-settings-selected-venue-trading-runtime-all-ok-local-capture-shaped/api/system/venue-operation-health",
        )
      && response.status() === 200,
    );

    await page.goto("/#settings");
    await expect(page.locator("h1", { hasText: "设置" })).toBeVisible();
    await credentials;
    await operationHealth;

    await expect(page.getByText("okx · 当前运行态 4/4 正常 · 0/4 待处理"))
      .toBeVisible();

    const tradingRuntimePanel = page
      .locator(".runtime-health-head")
      .filter({ hasText: "交易运行证据" })
      .locator("xpath=..");
    await expect(tradingRuntimePanel).toContainText("当前可用");

    const permissionRow = tradingRuntimePanel
      .locator("tr")
      .filter({ hasText: "下单/撤单权限" })
      .filter({ hasText: "credential_probe:order_permission" });
    await expect(permissionRow).toContainText("正常");
    await expect(permissionRow).toContainText("credential_validation");
    await expect(permissionRow).toContainText("freshness 700ms");
    await expect(permissionRow).toContainText("1/1");
    await expect(permissionRow).toContainText(
      "request_id req-settings-runtime-all-ok-order-permission",
    );
    await expect(permissionRow).toContainText("非真实交易所 live 样本");
    await expect(permissionRow.locator("td").last()).toHaveAttribute(
      "title",
      /local_ui_ci_gate=true/,
    );
    await expect(permissionRow.locator("td").last()).toHaveAttribute(
      "title",
      /not_real_exchange_live_sample=true/,
    );

    const writeRow = tradingRuntimePanel
      .locator("tr")
      .filter({ hasText: "写单运行态" })
      .filter({ hasText: "order_write" });
    await expect(writeRow).toContainText("正常");
    await expect(writeRow).toContainText("live_order_proof_runtime");
    await expect(writeRow).toContainText("freshness 600ms");
    await expect(writeRow).toContainText("2/2");
    await expect(writeRow).toContainText(
      "request_id req-settings-runtime-all-ok-order-write",
    );
    await expect(writeRow).toContainText("非真实交易所 live 样本");
    await expect(writeRow.locator("td").last()).toHaveAttribute(
      "title",
      /sample_place_request_id=req-settings-runtime-all-ok-place/,
    );
    await expect(writeRow.locator("td").last()).toHaveAttribute(
      "title",
      /not_real_exchange_live_sample=true/,
    );

    const privateOrderStreamRow = tradingRuntimePanel
      .locator("tr")
      .filter({ hasText: "私有订单流" })
      .filter({ hasText: "private_ws_order_stream" });
    await expect(privateOrderStreamRow).toContainText("正常");
    await expect(privateOrderStreamRow).toContainText("private_ws_runtime");
    await expect(privateOrderStreamRow).toContainText("freshness 500ms");
    await expect(privateOrderStreamRow).toContainText("3/3");
    await expect(privateOrderStreamRow).toContainText(
      "request_id req-settings-runtime-all-ok-private-stream",
    );
    await expect(privateOrderStreamRow).toContainText("非真实交易所 live 样本");
    await expect(privateOrderStreamRow.locator("td").last()).toHaveAttribute(
      "title",
      /official_evidence_shape=order_state_stream/,
    );
    await expect(privateOrderStreamRow.locator("td").last()).toHaveAttribute(
      "title",
      /not_real_exchange_live_sample=true/,
    );

    const orderFinalityRow = tradingRuntimePanel
      .locator("tr")
      .filter({ hasText: "订单终态" })
      .filter({ hasText: "order_finality" });
    await expect(orderFinalityRow).toContainText("正常");
    await expect(orderFinalityRow).toContainText("run_finality_runtime");
    await expect(orderFinalityRow).toContainText("freshness 900ms");
    await expect(orderFinalityRow).toContainText("1/1");
    await expect(orderFinalityRow).toContainText(
      "request_id req-settings-runtime-all-ok-order-finality",
    );
    await expect(orderFinalityRow).toContainText("非真实交易所 live 样本");
    await expect(orderFinalityRow.locator("td").last()).toHaveAttribute(
      "title",
      /scanned_order_count=1/,
    );
    await expect(orderFinalityRow.locator("td").last()).toHaveAttribute(
      "title",
      /not_real_exchange_live_sample=true/,
    );

    await expect(tradingRuntimePanel).not.toContainText("待证据");
    await expect(tradingRuntimePanel).not.toContainText("待验证");
    await expect(tradingRuntimePanel).not.toContainText("阻断");
    await expect(tradingRuntimePanel).not.toContainText("retry");
    await expect(tradingRuntimePanel).not.toContainText("可下单");
    await expect(tradingRuntimePanel).not.toContainText("权限验证完整");
  });

  test("top status bar consumes operation-health API and private WS rows", async ({ page }) => {
    const operationHealth = page.waitForResponse((response) =>
      response.url().endsWith("/api/system/venue-operation-health")
      && response.status() === 200,
    );

    await page.goto("/#futures");
    await expectFuturesReady(page);
    await operationHealth;

    const status = page.getByTestId(TOP_STATUS_BAR_TEST_ID);
    await expect(status).toBeVisible();
    await expect(status).toHaveAttribute("aria-label", "系统状态");

    const apiSlot = status.getByTestId(STATUS_API_RUNTIME_TEST_ID);
    await expect(apiSlot).toHaveClass(/degraded/);
    await expect(apiSlot).toContainText(STATUS_API_SLOT_LABEL);
    await expect(apiSlot).toContainText("2可用/4配置");
    await expect(apiSlot).not.toContainText("旧8/8");
    await expect(apiSlot).toHaveAttribute("title", /credential_probe:balance_read/);
    await expect(apiSlot).toHaveAttribute("title", /OKX balance read permission denied/);
    await expect(apiSlot).toHaveAttribute("title", /request_id req-credential-probe/);
    await expect(apiSlot).not.toHaveAttribute("title", /旧 API 汇总/);

    const wsSlot = status.getByTestId(STATUS_PRIVATE_WS_TEST_ID);
    await expect(wsSlot).toHaveClass(/degraded/);
    await expect(wsSlot).toContainText(STATUS_PRIVATE_WS_SLOT_LABEL);
    await expect(wsSlot).toContainText("0可用/2配置");
    await expect(wsSlot).not.toContainText("本地3");
    await expect(wsSlot).toHaveAttribute("title", /private_ws_subscribe/);
    await expect(wsSlot).toHaveAttribute("title", /Bybit private WS auth failed/);
    await expect(wsSlot).toHaveAttribute("title", /request_id req-private-ws-auth/);
    await expect(wsSlot).toHaveAttribute("title", /retry 30000ms/);
    await expect(wsSlot).not.toHaveAttribute("title", /本地 WS hub/);

    const orderSlot = status.getByTestId(STATUS_ORDER_ELAPSED_TEST_ID);
    await expect(orderSlot).toContainText(STATUS_ORDER_ELAPSED_SLOT_LABEL);
    await expect(orderSlot).toContainText("24ms");
    await expect(orderSlot).not.toContainText("RTT");
    await expect(orderSlot).toHaveAttribute("title", /OrderRecord updated_at - created_at/);
    await expect(orderSlot).toHaveAttribute("title", /不代表网络 RTT/);
  });

  test("top status bar surfaces private order stream runtime warning", async ({ page }) => {
    await useScenarioApiBase(page, "e2e-private-order-stream-warning");
    const operationHealth = page.waitForResponse((response) =>
      response.url().includes("/e2e-private-order-stream-warning/api/system/venue-operation-health")
      && response.status() === 200,
    );

    await page.goto("/#futures");
    await expectFuturesReady(page);
    await operationHealth;

    const wsSlot = page
      .getByTestId(TOP_STATUS_BAR_TEST_ID)
      .getByTestId(STATUS_PRIVATE_WS_TEST_ID);

    await expect(wsSlot).toHaveClass(/degraded/);
    await expect(wsSlot).toContainText(STATUS_PRIVATE_WS_SLOT_LABEL);
    await expect(wsSlot).toContainText("0可用/1配置");
    await expect(wsSlot).toHaveAttribute("title", /private_ws_order_stream/);
    await expect(wsSlot).toHaveAttribute("title", /存在 2 笔未决实盘订单/);
    await expect(wsSlot).toHaveAttribute("title", /request_id req-private-order-stream-stale/);
    await expect(wsSlot).toHaveAttribute("title", /retry 60000ms/);
    await expect(wsSlot).toHaveAttribute("title", /官方证据 order_state_stream/);
    await expect(wsSlot).not.toHaveAttribute("title", /private_ws_subscribe/);
  });

  test("top status bar shows API transport drilldown without changing trading count", async ({
    page,
  }) => {
    await useScenarioApiBase(page, "e2e-api-transport");
    const operationHealth = page.waitForResponse((response) =>
      response.url().includes("/e2e-api-transport/api/system/venue-operation-health")
      && response.status() === 200,
    );

    await page.goto("/#futures");
    await expectFuturesReady(page);
    await operationHealth;

    const apiSlot = page
      .getByTestId(TOP_STATUS_BAR_TEST_ID)
      .getByTestId(STATUS_API_RUNTIME_TEST_ID);

    await expect(apiSlot).toContainText("2可用/4配置");
    await expect(apiSlot).toHaveAttribute("title", /Transport：gate/);
    await expect(apiSlot).toHaveAttribute("title", /http_rest:GET \/api\/v4\/orders/);
    await expect(apiSlot).toHaveAttribute("title", /HTTP RTT 41ms \/ p95≤80ms/);
    await expect(apiSlot).toHaveAttribute("title", /request_id req-api-transport/);
    await expect(apiSlot).not.toHaveAttribute("title", /旧 API 汇总/);
  });

  test("top status bar excludes unscoped HTTP fallback from authenticated trading health", async ({
    page,
  }) => {
    await useScenarioApiBase(page, "e2e-api-transport-fallback");
    const operationHealth = page.waitForResponse((response) =>
      response.url().includes("/e2e-api-transport-fallback/api/system/venue-operation-health")
      && response.status() === 200,
    );

    await page.goto("/#futures");
    await expectFuturesReady(page);
    await operationHealth;

    const apiSlot = page
      .getByTestId(TOP_STATUS_BAR_TEST_ID)
      .getByTestId(STATUS_API_RUNTIME_TEST_ID);

    await expect(apiSlot).toContainText("2可用/4配置");
    await expect(apiSlot).toHaveAttribute("title", /credential_probe:balance_read/);
    await expect(apiSlot).not.toHaveAttribute("title", /Transport：gate/);
    await expect(apiSlot).not.toHaveAttribute("title", /req-api-transport-fallback/);
    await expect(apiSlot).not.toHaveAttribute("title", /官方证据 http_outcome/);
    await expect(apiSlot).not.toHaveAttribute("title", /endpoint_evidence=not_recorded/);
    await expect(apiSlot).not.toHaveAttribute("title", /旧 API 汇总/);
  });

  test("top status bar surfaces system health LoadState failure", async ({ page }) => {
    await useScenarioApiBase(page, "e2e-system-502");
    const systemHealthFailure = page.waitForResponse((response) =>
      response.url().includes("/e2e-system-502/api/system/health")
      && response.status() === 502,
    );
    await page.goto("/#futures");
    await expectFuturesReady(page);
    await systemHealthFailure;

    const status = page.getByTestId(TOP_STATUS_BAR_TEST_ID);
    await expect(status).toBeVisible();
    await expect(status).toHaveAttribute("aria-label", "系统状态");

    const orderSlot = status.getByTestId(STATUS_ORDER_ELAPSED_TEST_ID);
    await expect(orderSlot).toHaveClass(/degraded/);
    await expect(orderSlot).toContainText(STATUS_ORDER_ELAPSED_SLOT_LABEL);
    await expect(orderSlot).toContainText("错误");
    await expect(orderSlot).toHaveAttribute("title", /mock system health failed/);
    await expect(orderSlot).toHaveAttribute("title", /HTTP 502/);
    await expect(orderSlot).toHaveAttribute("title", /request_id e2e-system-health-502/);
    await expect(orderSlot).toHaveAttribute("title", /retry 4000ms/);

    const riskSlot = status.locator("button.slot", { hasText: "Risk" });
    await expect(riskSlot).toHaveClass(/degraded/);
    await expect(riskSlot).toContainText("错误");
    await expect(riskSlot).toHaveAttribute("title", /mock system health failed/);

    const deltaSlot = status.locator(".slot", { hasText: "Delta" });
    await expect(deltaSlot).toContainText("错误");
    await expect(deltaSlot).toHaveAttribute("title", /request_id e2e-system-health-502/);

    const fundingSlot = status.locator(".slot", { hasText: "Funding" });
    await expect(fundingSlot).toContainText("错误");
    await expect(fundingSlot).toHaveAttribute("title", /retry 4000ms/);
    await expect(status.locator(".slot", { hasText: "Funding" })).not.toContainText("60m");
  });

  test("settings diagnostics shows typed operation health fetch failure", async ({ page }) => {
    await page.route("**/api/system/venue-operation-health", async (route) => {
      const headers = routeJsonHeaders("GET,OPTIONS", {
        "retry-after": "3",
        "x-request-id": "req-venue-health",
      });
      if (route.request().method() === "OPTIONS") {
        await route.fulfill({ status: 204, headers, body: "" });
        return;
      }
      await route.fulfill({
        status: 502,
        headers,
        body: JSON.stringify({
          error: {
            code: "VENUE_OPERATION_HEALTH_FAILED",
            message: "venue operation health failed",
            status: 502,
            requestId: "req-venue-health",
            retryAfterMs: 3000,
            source: "e2e-route",
          },
        }),
      });
    });

    await page.goto("/#settings");
    await expect(page.locator("h1", { hasText: "设置" })).toBeVisible();

    await page.getByRole("tab", { name: "诊断" }).click();
    const cell = page.locator(".empty-cell").filter({ hasText: "读取运行状态失败" });

    await expect(cell).toContainText("venue operation health failed");
    await expect(cell).toHaveAttribute("title", /HTTP 502/);
    await expect(cell).toHaveAttribute("title", /request_id req-venue-health/);
    await expect(cell).toHaveAttribute("title", /retry 3000ms/);
    await expect(page.getByText("暂无运行态状态")).toHaveCount(0);
  });

  test("settings risk kill switch surfaces extractor 422 typed problem", async ({ page }) => {
    await useScenarioApiBase(page, "e2e-extractor-422");
    const killSwitchFailure = page.waitForResponse((response) =>
      response.url().includes("/e2e-extractor-422/api/trading/kill-switch")
      && response.status() === 422,
    );

    await page.goto("/#settings");
    await expect(page.locator("h1", { hasText: "设置" })).toBeVisible();

    await page.getByRole("tab", { name: "风控", exact: true }).click();
    await expect(page.getByText("Kill Switch 关闭")).toBeVisible();
    await page.getByRole("button", { name: "切换 Kill Switch" }).click();
    await killSwitchFailure;

    const message = page.locator(".settings-message").filter({ hasText: "更新失败" });
    await expect(message).toContainText(
      "更新失败：invalid JSON request body · code REQUEST_BODY_INVALID · source e2e-fixture · HTTP 422 · request_id e2e-extractor-422",
    );
    await expect(message).not.toContainText("Kill Switch 已开启");
  });

  test("settings credential save denial redacts secret and shows no success feedback", async ({
    page,
  }) => {
    const secret = "e2e-secret-must-not-echo-7f3a";
    await useScenarioApiBase(page, "e2e-credential-save-denied");
    await page.goto("/#settings");
    await expect(page.locator("h1", { hasText: "设置" })).toBeVisible();

    const secretInput = page.getByLabel("API Secret", { exact: true });
    await expect(secretInput).toHaveAttribute("type", "password");
    await secretInput.fill(secret);
    const saveFailure = page.waitForResponse((response) =>
      response.url().includes(
        "/e2e-credential-save-denied/api/exchanges/credentials",
      )
      && response.request().method() === "POST"
      && response.status() === 403,
    );
    await page.getByRole("button", { name: "保存字段" }).click();
    const response = await saveFailure;

    const responseText = await response.text();
    expect(responseText).not.toContain(secret);
    expect(responseText).not.toContain("api_secret");
    const message = page.locator(".credential-editor-actions em");
    await expect(message).toContainText("保存失败：credential save denied by secret policy");
    await expect(message).toContainText("code CREDENTIAL_SAVE_DENIED");
    await expect(message).toContainText("HTTP 403");
    await expect(message).toContainText("request_id req-credential-save-denied");
    await expect(message).toContainText("retry 30000ms");
    await expect(page.locator("body")).not.toContainText(secret);
    await expect(message).not.toContainText("已保存");
    await expect(message).not.toContainText("Secret 仅进程内缓存");
  });

  test("review venue quality shows neutral no-sample baseline", async ({ page }) => {
    await page.goto("/#review");
    await expect(page.locator("h1", { hasText: "复盘" })).toBeVisible();

    await expect(page.getByText("无样本基线 · 2 场所 · 0 已采样")).toBeVisible();
    await expect(page.getByText("等待场所执行质量")).toHaveCount(0);

    const row = page.locator("tr").filter({ hasText: "binance" });
    await expect(row).toBeVisible();
    await expect(row).toContainText("未采样");
    await expect(row).not.toContainText("100.0%");
    await expect(row).not.toContainText("0ms");
  });

  test("review executed failure surfaces LoadState error without fake rows", async ({ page }) => {
    await useScenarioApiBase(page, "e2e-review-502");
    await page.goto("/#review");
    await expect(page.locator("h1", { hasText: "复盘" })).toBeVisible();

    const reviewSurface = page.locator(".surface").filter({ hasText: "交易复盘" });
    const stateLine = reviewSurface.locator(".reason-pill").first();
    await expect(stateLine).toContainText("读取失败");
    await expect(stateLine).toContainText("mock review executed failed");
    await expect(stateLine).toContainText("HTTP 502");
    await expect(stateLine).toContainText("request_id e2e-review-executed-502");
    await expect(stateLine).toContainText("retry 3000ms");

    const table = reviewSurface.locator(".review-table").first();
    const cell = table.locator(".empty-cell");
    await expect(cell).toContainText("读取失败：mock review executed failed");
    await expect(cell).toContainText("HTTP 502");
    await expect(cell).toContainText("request_id e2e-review-executed-502");
    await expect(cell).toContainText("retry 3000ms");
    await expect(table.locator("tbody tr")).toHaveCount(1);
    await expect(table.locator("tbody tr").filter({ hasText: "MU" })).toHaveCount(0);
    await expect(page.getByText("暂无已执行记录")).toHaveCount(0);
  });

  test("review missed failure surfaces LoadState error without fake attribution", async ({
    page,
  }) => {
    await useScenarioApiBase(page, "e2e-review-missed-502");
    await page.addInitScript(() => {
      window.localStorage.setItem(
        "crossline.review.activeTab",
        JSON.stringify("missed"),
      );
    });
    const missedFailure = page.waitForResponse((response) =>
      response.url().includes("/e2e-review-missed-502/api/review/missed")
      && response.status() === 502,
    );
    await page.goto("/#review");
    await expect(page.locator("h1", { hasText: "复盘" })).toBeVisible();
    await missedFailure;

    const reviewSurface = page.locator(".surface").filter({ hasText: "交易复盘" });
    const stateLine = reviewSurface.locator(".reason-pill").first();
    await expect(stateLine).toContainText("读取失败");
    await expect(stateLine).toContainText("mock review missed failed");
    await expect(stateLine).toContainText("HTTP 502");
    await expect(stateLine).toContainText("request_id e2e-review-missed-502");
    await expect(stateLine).toContainText("retry 3000ms");

    const table = reviewSurface.locator(".review-table").first();
    const cell = table.locator(".empty-cell");
    await expect(cell).toContainText("读取失败：mock review missed failed");
    await expect(cell).toContainText("HTTP 502");
    await expect(cell).toContainText("request_id e2e-review-missed-502");
    await expect(cell).toContainText("retry 3000ms");
    await expect(table.locator("tbody tr")).toHaveCount(1);
    await expect(table.locator("tbody tr").filter({ hasText: "MU" })).toHaveCount(0);
    await expect(page.getByText("暂无错失机会")).toHaveCount(0);
    await expect(reviewSurface.locator(".review-attribution")).toHaveCount(0);
  });

  test("review scalar resource failures keep typed LoadState context visible", async ({
    page,
  }) => {
    await useScenarioApiBase(page, "e2e-review-resources-502");
    await page.addInitScript(() => {
      window.localStorage.setItem(
        "crossline.review.activeTab",
        JSON.stringify("strategy"),
      );
      window.localStorage.setItem(
        "crossline.review.venueQuality.view",
        JSON.stringify("table"),
      );
    });
    const strategyFailure = page.waitForResponse((response) =>
      response.url().includes("/e2e-review-resources-502/api/review/strategy-performance")
      && response.status() === 502,
    );
    const venueQualityFailure = page.waitForResponse((response) =>
      response.url().includes("/e2e-review-resources-502/api/trading/venues/quality")
      && response.status() === 502,
    );
    await page.goto("/#review");
    await expect(page.locator("h1", { hasText: "复盘" })).toBeVisible();
    await strategyFailure;
    await venueQualityFailure;

    const reviewSurface = page.locator(".surface").filter({ hasText: "交易复盘" });
    const strategyState = reviewSurface.locator(".reason-pill").first();
    await expect(strategyState).toContainText("读取失败");
    await expect(strategyState).toContainText("mock review strategy performance failed");
    await expect(strategyState).toContainText("HTTP 502");
    await expect(strategyState).toContainText("request_id e2e-review-strategy-502");
    await expect(strategyState).toContainText("retry 4000ms");

    const strategyTable = reviewSurface.locator(".review-table").first();
    const strategyCell = strategyTable.locator(".empty-cell");
    await expect(strategyCell).toContainText(
      "读取失败：mock review strategy performance failed",
    );
    await expect(strategyCell).toContainText("HTTP 502");
    await expect(strategyCell).toContainText("request_id e2e-review-strategy-502");
    await expect(strategyCell).toContainText("retry 4000ms");
    await expect(strategyTable.locator("tbody tr")).toHaveCount(1);
    await expect(strategyTable.locator("tbody tr").filter({ hasText: "永续跨所" }))
      .toHaveCount(0);
    await expect(page.getByText("等待策略绩效")).toHaveCount(0);

    const qualitySurface = page.locator(".surface").filter({ hasText: "场所执行质量" });
    const qualityState = qualitySurface.locator(".reason-pill").first();
    await expect(qualityState).toContainText("读取失败");
    await expect(qualityState).toContainText("mock venue quality failed");
    await expect(qualityState).toContainText("HTTP 502");
    await expect(qualityState).toContainText("request_id e2e-venue-quality-502");
    await expect(qualityState).toContainText("retry 5000ms");

    const qualityTable = qualitySurface.locator(".venue-quality-table");
    const qualityCell = qualityTable.locator(".empty-cell");
    await expect(qualityCell).toContainText("读取失败：mock venue quality failed");
    await expect(qualityCell).toContainText("HTTP 502");
    await expect(qualityCell).toContainText("request_id e2e-venue-quality-502");
    await expect(qualityCell).toContainText("retry 5000ms");
    await expect(qualityTable.locator("tbody tr")).toHaveCount(1);
    await expect(qualityTable.locator("tbody tr").filter({ hasText: "binance" })).toHaveCount(0);
    await expect(page.getByText("等待场所执行质量")).toHaveCount(0);
    await expect(qualitySurface.locator(".quality-summary-grid")).toHaveCount(0);
    await expect(qualitySurface.getByText("无样本基线")).toHaveCount(0);
  });

  test("review stale rows respect retry-after backoff", async ({ page, request }) => {
    const statsUrl = `${API_BASE}/e2e-review-stale-retry/api/e2e/request-stats`;
    const reset = await request.delete(statsUrl);
    expect(reset.ok()).toBeTruthy();

    await useScenarioApiBase(page, "e2e-review-stale-retry");
    const firstLoad = page.waitForResponse((response) =>
      response.url().includes("/e2e-review-stale-retry/api/review/executed")
      && response.status() === 200,
    );
    await page.goto("/#review");
    await expect(page.locator("h1", { hasText: "复盘" })).toBeVisible();
    await firstLoad;

    const reviewSurface = page.locator(".surface").filter({ hasText: "交易复盘" });
    const table = reviewSurface.locator(".review-table").first();
    await expect(table.locator("tbody tr").filter({ hasText: "MU-STL" })).toHaveCount(1);

    const pager = reviewSurface.locator(".table-pager").first();
    await expect(pager).toContainText("1-1 / 2");

    const refreshFailure = page.waitForResponse((response) =>
      response.url().includes("/e2e-review-stale-retry/api/review/executed")
      && response.status() === 429,
    );
    await pager.getByRole("button", { name: "下一页" }).click();
    await refreshFailure;

    const stateLine = reviewSurface.locator(".reason-pill").first();
    await expect(stateLine).toContainText("降级");
    await expect(stateLine).toContainText("mock review executed refresh rate limited");
    await expect(stateLine).toContainText("HTTP 429");
    await expect(stateLine).toContainText("request_id e2e-review-stale-retry-2");
    await expect(stateLine).toContainText("retry 15000ms");
    await expect(table.locator("tbody tr").filter({ hasText: "MU-STL" })).toHaveCount(1);
    await expect(table.locator(".empty-cell")).toHaveCount(0);
    await expect(page.getByText("暂无已执行记录")).toHaveCount(0);

    const executedRequestCount = async () => {
      const stats = await request.get(statsUrl);
      expect(stats.ok()).toBeTruthy();
      const body = await stats.json();
      return body.counts["/api/review/executed"] ?? 0;
    };

    await expect.poll(executedRequestCount).toBe(2);
    await page.waitForTimeout(10_500);
    expect(await executedRequestCount()).toBe(2);
  });

  test("settings api base save applies without page reload", async ({
    page,
  }) => {
    await page.goto("/#settings");
    await expect(page.locator("h1", { hasText: "设置" })).toBeVisible();

    await page.getByRole("tab", { name: "诊断" }).click();
    const save = page.getByRole("button", { name: "保存并应用" });
    await expect(save).toBeDisabled();

    const nextBase = `${API_BASE}/alt///`;
    await page.getByLabel("API Base").fill(` ${nextBase} `);
    await expect(save).toBeDisabled();

    await page.evaluate(() => {
      (window as any).__crosslineApiBaseApplyMarker = "kept";
    });
    await page.getByLabel("确认应用").fill("apply");
    await expect(save).toBeEnabled();
    await save.click();

    await expect(page.locator("h1", { hasText: "设置" })).toBeVisible();
    await expect(
      page.locator(".settings-message").filter({ hasText: "API Base 已应用" }),
    ).toContainText("API Base 已应用");
    const stored = await page.evaluate(() => localStorage.getItem("api_base"));
    expect(JSON.parse(stored ?? "null")).toBe(`${API_BASE}/alt`);
    const marker = await page.evaluate(() => (window as any).__crosslineApiBaseApplyMarker);
    expect(marker).toBe("kept");
  });

  test("PR-CO visual smoke keeps core tables stable across viewports", async ({ page }) => {
    const browserErrors = [];
    page.on("pageerror", (error) => browserErrors.push(error.message));
    page.on("console", (message) => {
      if (message.type() === "error") browserErrors.push(message.text());
    });
    await useScenarioApiBase(page, "e2e-large-tables");

    for (const viewport of VISUAL_SMOKE_VIEWPORTS) {
      await page.setViewportSize({ width: viewport.width, height: viewport.height });

      await page.goto("/#opportunities");
      await expect(page.getByRole("heading", { name: "机会扫描" })).toBeVisible();
      await expect(page.getByText("1-50 / 500")).toBeVisible();
      await expect(page.locator(".opportunity-layout table.clean-table tbody tr"))
        .toHaveCount(50);
      await expectVisualShellStable(page, `${viewport.name} opportunities`);
      await expectHeaderVisibleAfterContentScroll(
        page,
        ".opportunity-layout table.clean-table",
        `${viewport.name} opportunities`,
      );

      await page.goto("/#review");
      await expect(page.locator("h1", { hasText: "复盘" })).toBeVisible();
      await expect(page.getByText("1-50 / 1000")).toBeVisible();
      await expect(page.locator(".review-table").first().locator("tbody tr")).toHaveCount(50);
      await expectVisualShellStable(page, `${viewport.name} review`);
      await expectHeaderVisibleAfterContentScroll(
        page,
        ".review-table",
        `${viewport.name} review`,
      );

      await page.goto("/#settings");
      await expect(page.locator("h1", { hasText: "设置" })).toBeVisible();
      await page.getByRole("tab", { name: "诊断" }).click();
      await expect(page.getByText("运行态矩阵")).toBeVisible();
      await expect(page.locator(".settings-summary-line").filter({ hasText: "运行态矩阵" }))
        .toContainText("200 条");
      await expect(page.locator("table.settings-table tbody tr").first()).toBeVisible();
      await expectVisualShellStable(page, `${viewport.name} settings`);
      await expectHeaderVisibleAfterContentScroll(
        page,
        "table.settings-table",
        `${viewport.name} settings`,
      );
    }

    expect(browserErrors).toEqual([]);
  });

  test("large table perf smoke keeps paged modules responsive", async ({ page }) => {
    await useScenarioApiBase(page, "e2e-large-tables");

    const firstPaintStarted = Date.now();
    await page.goto("/#futures");
    await expectFuturesReady(page);
    await expect(page.getByText("1-50 / 500")).toBeVisible();
    await expect(page.getByRole("button", { name: "构建对冲" })).toHaveCount(50);
    expect(Date.now() - firstPaintStarted).toBeLessThan(12_000);

    await page.evaluate(() => {
      const content = document.querySelector(".mod-content");
      if (content) content.scrollTop = 480;
    });
    await page.locator(".table-pager").first().getByRole("button", { name: "下一页" }).click();
    await expect(page.getByText("51-100 / 500")).toBeVisible();
    await expect
      .poll(async () =>
        page.evaluate(() => document.querySelector(".mod-content")?.scrollTop ?? -1),
      )
      .toBe(0);

    const opportunitySwitchStarted = Date.now();
    await switchModule(page, "机会扫描");
    await expect(page.getByRole("heading", { name: "机会扫描" })).toBeVisible();
    await expect(page.getByText("51-100 / 500")).toBeVisible();
    await expect(page.locator(".opportunity-layout table.clean-table tbody tr")).toHaveCount(50);
    expect(Date.now() - opportunitySwitchStarted).toBeLessThan(5_000);

    await switchModule(page, "复盘");
    await expect(page.locator("h1", { hasText: "复盘" })).toBeVisible();
    await expect(page.getByText("1-50 / 1000")).toBeVisible();
    await expect(page.locator(".review-table").first().locator("tbody tr")).toHaveCount(50);
    await page.locator(".table-pager").first().getByRole("button", { name: "下一页" }).click();
    await expect(page.getByText("51-100 / 1000")).toBeVisible();

    await page.getByRole("button", { name: "错失机会" }).click();
    await expect(page.getByText("1-50 / 1000")).toBeVisible();
    await expect(page.locator(".review-table").first().locator("tbody tr")).toHaveCount(50);

    await switchModule(page, "设置");
    await expect(page.locator("h1", { hasText: "设置" })).toBeVisible();
    await page.getByRole("tab", { name: "诊断" }).click();
    await expect(page.getByText("运行态矩阵")).toBeVisible();
    await expect(
      page.locator(".settings-summary-line").filter({ hasText: "运行态矩阵" }),
    ).toContainText("200 条");
    const matrix = page
      .locator("table.settings-table")
      .filter({ hasText: "操作" })
      .filter({ hasText: "样本" })
      .first();
    await expect(matrix.locator("tbody tr")).toHaveCount(20);
    await page.getByLabel("搜索状态").fill("diagnostic_probe_199");
    await expect(page.locator("tr").filter({ hasText: "diagnostic_probe_199" })).toBeVisible();

    await switchModule(page, "机会扫描");
    await expect(page.getByRole("heading", { name: "机会扫描" })).toBeVisible();
    await expect(page.getByText("51-100 / 500")).toBeVisible();
  });

  test("large table perf smoke keeps opportunity switch list-only and bounded", async ({
    page,
  }) => {
    await installOpportunityFetchProbe(page);
    await useScenarioApiBase(page, "e2e-large-tables");

    let collecting = false;
    const listResponseReads = [];
    const legacyWideRequests = [];
    const collectedListRequests = new WeakSet();
    page.on("response", (response) => {
      const url = new URL(response.url());
      if (
        collecting
        && response.request().method() === "GET"
        && url.pathname.endsWith("/api/v3/arbitrage/opportunities/list")
        && collectedListRequests.has(response.request())
      ) {
        listResponseReads.push(response.body().then((body) => body.byteLength));
      }
    });
    page.on("request", (request) => {
      const url = new URL(request.url());
      if (
        collecting
        && request.method() === "GET"
        && url.pathname.endsWith("/api/v3/arbitrage/opportunities/list")
      ) {
        collectedListRequests.add(request);
      }
      if (
        collecting
        && request.method() === "GET"
        && url.pathname.endsWith("/api/v3/arbitrage/opportunities")
      ) {
        legacyWideRequests.push(request.url());
      }
    });

    await page.goto("/#futures");
    await expectFuturesReady(page);
    await expect(page.getByText("1-50 / 500")).toBeVisible();

    collecting = true;
    await setOpportunityFetchProbe(page, true);
    await page.evaluate(() => {
      (window as any).__crosslineWasmSerdeMetrics = [];
    });
    await startOpportunityHotPathProbe(page, ".opportunity-layout table.clean-table");
    const started = Date.now();
    await switchModule(page, "机会扫描");
    await expect(page.getByRole("heading", { name: "机会扫描" })).toBeVisible();
    await expect(page.getByText("1-50 / 500")).toBeVisible();
    await expect(page.locator(".opportunity-layout table.clean-table tbody tr")).toHaveCount(50);
    await page.evaluate(
      () => new Promise((resolve) => requestAnimationFrame(() => resolve(null))),
    );
    const browserBudget = await stopOpportunityHotPathProbe(page);
    const fetchBudget = await setOpportunityFetchProbe(page, false);
    const wasmSerdeMetrics = await page.evaluate(() => {
      const runtime = window as any;
      const metrics = runtime.__crosslineWasmSerdeMetrics ?? [];
      delete runtime.__crosslineWasmSerdeMetrics;
      return metrics;
    });
    collecting = false;

    expect(Date.now() - started).toBeLessThan(OPPORTUNITY_SWITCH_RENDER_BUDGET_MS);
    expect(fetchBudget.missing).toBeFalsy();
    expect(fetchBudget.listResponses).toHaveLength(1);
    const listBrowserMetric = fetchBudget.listResponses[0];
    expect(listBrowserMetric.bytes).toBeLessThanOrEqual(OPPORTUNITY_LIST_PAYLOAD_BUDGET_BYTES);
    expect(listBrowserMetric.cloneReadMs).toBeLessThanOrEqual(
      OPPORTUNITY_LIST_BROWSER_CLONE_BUDGET_MS,
    );
    expect(listBrowserMetric.jsonParseMs).toBeLessThanOrEqual(
      OPPORTUNITY_LIST_BROWSER_PARSE_BUDGET_MS,
    );
    expect(wasmSerdeMetrics).toHaveLength(1);
    expect(wasmSerdeMetrics[0]).toMatchObject({
      path: expect.stringContaining("/api/v3/arbitrage/opportunities/list"),
      success: true,
    });
    expect(wasmSerdeMetrics[0].decodeMs).toBeLessThanOrEqual(
      OPPORTUNITY_LIST_WASM_DECODE_BUDGET_MS,
    );
    expect(browserBudget.missing).toBeFalsy();
    expect(browserBudget.durationMs).toBeLessThanOrEqual(OPPORTUNITY_TABLE_RAF_COMMIT_BUDGET_MS);
    expect(browserBudget.maxRows).toBe(50);
    expect(browserBudget.maxCells).toBeLessThanOrEqual(OPPORTUNITY_SWITCH_TABLE_CELL_BUDGET);
    expect(browserBudget.maxFrameGapMs).toBeLessThanOrEqual(
      OPPORTUNITY_SWITCH_FRAME_GAP_BUDGET_MS,
    );
    expect(browserBudget.longTaskMs).toBeLessThanOrEqual(
      OPPORTUNITY_SWITCH_LONG_TASK_BUDGET_MS,
    );
    if (browserBudget.heapDeltaBytes !== null) {
      expect(browserBudget.heapDeltaBytes).toBeLessThanOrEqual(
        OPPORTUNITY_SWITCH_HEAP_DELTA_BUDGET_BYTES,
      );
    }
    const listPayloadBytes = await Promise.all(listResponseReads);
    expect(listPayloadBytes.length).toBeGreaterThanOrEqual(1);
    expect(listPayloadBytes.length).toBeLessThanOrEqual(1);
    for (const bytes of listPayloadBytes) {
      expect(bytes).toBeLessThanOrEqual(OPPORTUNITY_LIST_PAYLOAD_BUDGET_BYTES);
    }
    expect(legacyWideRequests).toEqual([]);
  });

  test("mock route surface returns typed problem for unhandled paths", async ({
    request,
  }) => {
    const response = await request.get(`${API_BASE}/api/e2e/unhandled-route`, {
      timeout: 5_000,
    });
    expect(response.status()).toBe(404);
    expect(response.headers()["x-request-id"]).toBe("e2e-route-not-found");

    const body = await response.json();
    expect(body.error).toMatchObject({
      code: "NOT_FOUND",
      message: "unhandled e2e fixture route: /api/e2e/unhandled-route",
      status: 404,
      requestId: "e2e-route-not-found",
      source: "e2e-fixture",
    });
    expect(body.error.details).toMatchObject({ route: "/api/e2e/unhandled-route" });
  });
});
