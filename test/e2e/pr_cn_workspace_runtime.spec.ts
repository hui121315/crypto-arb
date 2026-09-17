import { expect, test, type Page } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";
const WEB_BASE = process.env.CROSSLINE_E2E_WEB_BASE ?? "http://127.0.0.1:18080";

async function useApiBase(page: Page, scenario?: string) {
  await page.addInitScript(
    ({ apiBase, scenario }) => {
      const base = scenario ? `${apiBase}/${scenario}` : apiBase;
      window.localStorage.setItem("api_base", JSON.stringify(base));
      window.localStorage.setItem("api_auth_token", JSON.stringify("e2e-token"));
    },
    { apiBase: API_BASE, scenario },
  );
  await page.routeWebSocket("**/ws**", (socket) => socket.close());
}

function moduleTab(page: Page, title: string) {
  return page.locator(".module-tabs button").filter({ hasText: title });
}

test("PR-CN restores symbol strategy and cursor deep link across module switches", async ({
  page,
}) => {
  await useApiBase(page);
  let scopedRequest = "";
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (
      url.pathname.endsWith("/api/v3/arbitrage/opportunities/list") &&
      url.searchParams.get("symbol") === "MU"
    ) {
      scopedRequest = request.url();
    }
  });

  await page.goto("/#futures?symbol=MU&strategy=spot_perp&page=cursor-pr-cn");
  await expect(page.getByRole("heading", { name: "期货套利" })).toBeVisible();
  await expect(page.getByPlaceholder("品种 / 交易所")).toHaveValue("MU");
  await expect(page.locator(".strategy-chips button.active")).toContainText("现货-永续");
  await expect.poll(() => scopedRequest).not.toBe("");
  expect(new URL(scopedRequest).searchParams.get("cursor")).toBe("cursor-pr-cn");

  await moduleTab(page, "设置").click();
  await expect(page.locator("h1", { hasText: "设置" })).toBeVisible();
  await moduleTab(page, "期货套利").click();
  await expect(page.getByPlaceholder("品种 / 交易所")).toHaveValue("MU");
  await expect(page.locator(".strategy-chips button.active")).toContainText("现货-永续");
});

test("PR-CN routes explicit opportunity and run context into the execution query", async ({
  page,
}) => {
  await useApiBase(page, "e2e-order-cancel-denied");
  let runRequest = "";
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (url.pathname.endsWith("/api/trading/execution-runs")) {
      runRequest = request.url();
    }
  });

  await page.goto(
    "/#execution?opp=mock-mu-perp&run=e2e-run-cancel-denied",
  );
  await expect(page.getByRole("heading", { name: "对冲执行" })).toBeVisible();
  await expect.poll(() => runRequest).not.toBe("");
  const query = new URL(runRequest).searchParams;
  expect(query.get("runId")).toBe("e2e-run-cancel-denied");
  expect(query.has("ticketId")).toBe(false);
  expect(query.has("opportunityId")).toBe(false);
  await expect
    .poll(() =>
      page.evaluate(() => ({
        opportunity: JSON.parse(
          window.localStorage.getItem(
            "crossline.execution.runContext.opportunityId",
          ) ?? "null",
        ),
        run: JSON.parse(
          window.localStorage.getItem("crossline.execution.runContext.runId") ??
            "null",
        ),
      })),
    )
    .toEqual({ opportunity: "mock-mu-perp", run: "e2e-run-cancel-denied" });
});

test("PR-CN preserves execution draft pending evidence and outcome across unmount", async ({
  page,
}) => {
  await useApiBase(page, "e2e-order-cancel-denied");
  let releaseResponse: (() => void) | undefined;
  const responseGate = new Promise<void>((resolve) => {
    releaseResponse = resolve;
  });
  let confirmStarted = false;
  let confirmCompleted = false;
  await page.route(
    "**/api/arbitrage/opportunities/mock-mu-perp/confirm",
    async (route) => {
      if (route.request().method() !== "POST") {
        await route.continue();
        return;
      }
      confirmStarted = true;
      const upstream = await route.fetch();
      await responseGate;
      await route.fulfill({ response: upstream });
      confirmCompleted = true;
    },
  );

  await page.goto("/#futures");
  await page.getByRole("button", { name: "构建对冲" }).click();
  const capital = page.getByLabel("本金 USD");
  await capital.fill("733");
  await expect(page.locator(".confirm-action.primary")).toBeEnabled();
  await page.getByRole("button", { name: "提交 模拟" }).click();
  await expect.poll(() => confirmStarted).toBe(true);
  await expect(moduleTab(page, "对冲执行")).toHaveAttribute(
    "data-runtime-state",
    "pending",
  );

  await moduleTab(page, "设置").click();
  await expect(page.locator("h1", { hasText: "设置" })).toBeVisible();
  await expect(moduleTab(page, "对冲执行")).toHaveAttribute(
    "data-runtime-state",
    "pending",
  );
  await moduleTab(page, "对冲执行").click();
  await expect(page.getByLabel("本金 USD")).toHaveValue("733");
  await expect(page.locator(".execution-actionbar .run-state em").first()).toContainText(
    "request_id",
  );

  releaseResponse?.();
  await expect.poll(() => confirmCompleted).toBe(true);
  await expect(page.locator(".confirm-outcome-detail")).toBeVisible();
  await expect(moduleTab(page, "对冲执行")).toHaveAttribute(
    "data-runtime-state",
    "pending",
  );
  await expect(moduleTab(page, "对冲执行")).toHaveAttribute(
    "title",
    /finality/,
  );
});

test("PR-CN exposes one typed problem inline in navigation and global toast", async ({
  page,
}) => {
  await useApiBase(page);
  const headers = {
    "access-control-allow-origin": WEB_BASE,
    "access-control-allow-methods": "GET,OPTIONS",
    "access-control-allow-headers":
      "authorization,accept,content-type,x-request-id",
    "access-control-expose-headers": "retry-after,x-request-id",
    "content-type": "application/json; charset=utf-8",
    "retry-after": "2",
    "x-request-id": "req-pr-cn-workspace",
    vary: "origin",
  };
  await page.route("**/api/v3/arbitrage/opportunities/list**", async (route) => {
    if (route.request().method() === "OPTIONS") {
      await route.fulfill({ status: 204, headers, body: "" });
      return;
    }
    await route.fulfill({
      status: 429,
      headers,
      body: JSON.stringify({
        error: {
          code: "WORKSPACE_RATE_LIMITED",
          message: "workspace opportunity source rate limited",
          status: 429,
          requestId: "req-pr-cn-workspace",
          retryAfterMs: 2_000,
          source: "pr-cn-workspace",
        },
      }),
    });
  });

  await page.goto("/#opportunities");
  const tab = moduleTab(page, "机会扫描");
  await expect(tab).toHaveAttribute("data-runtime-state", "error");
  await expect(
    page.locator(".settings-message.is-error").filter({
      hasText: "机会快照冷启动失败",
    }),
  ).toContainText("request_id req-pr-cn-workspace");
  await expect(
    page.locator(".toast-item").filter({ hasText: "机会扫描" }),
  ).toContainText("code=WORKSPACE_RATE_LIMITED · status=429 · source=pr-cn-workspace");
});

test("PR-GT and PR-GU expose dedicated first-level product modules", async ({
  page,
}) => {
  await useApiBase(page);
  await page.goto("/#onchain");

  await expect(page.getByRole("heading", { name: "链上套利" })).toBeVisible();
  await expect(moduleTab(page, "链上套利")).toHaveAttribute(
    "aria-current",
    "page",
  );
  await expect(page.locator(".onchain-workbench")).toBeVisible();
  await expect(page.locator(".opportunity-layout")).toHaveCount(0);

  await moduleTab(page, "自动化").click();
  await expect(page.getByRole("heading", { name: "自动化" })).toBeVisible();
  await expect(moduleTab(page, "自动化")).toHaveAttribute(
    "aria-current",
    "page",
  );
  await expect(page.locator(".automation-workbench")).toBeVisible();
  await expect(page.getByText("退出保护", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "先配置退出保护" })).toBeDisabled();
  await expect(page.locator(".execution-grid")).toHaveCount(0);
});

test("PR-GM keeps the workstation topbar readable without page overflow", async ({
  page,
}) => {
  await useApiBase(page);
  await page.setViewportSize({ width: 1280, height: 720 });
  await page.goto("/#positions");

  const navigation = page.locator(".module-tabs");
  const status = page.getByTestId("top-status-bar");
  const primary = page.locator(".topbar-primary");
  const statusFrame = page.locator(".topbar-status-frame");
  const modulePage = page.locator(".module-page");
  await expect(navigation.locator("button")).toHaveCount(8);
  await expect(status.locator(".status-cluster")).toHaveCount(4);
  await expect(status.locator(".slot")).toHaveCount(10);
  await expect(page.getByTestId("status-order-elapsed")).toContainText("订单终态");

  const desktop = await page.evaluate(() => {
    const primary = document.querySelector<HTMLElement>(".topbar-primary");
    const navigation = document.querySelector<HTMLElement>(".module-tabs");
    const status = document.querySelector<HTMLElement>('[data-testid="top-status-bar"]');
    const statusFrame = document.querySelector<HTMLElement>(".topbar-status-frame");
    const modulePage = document.querySelector<HTMLElement>(".module-page");
    const slots = [...document.querySelectorAll<HTMLElement>(".top-slots .slot")];
    const bounds = (element: HTMLElement | null) => {
      const rect = element?.getBoundingClientRect();
      return rect
        ? { left: rect.left, right: rect.right, width: rect.width }
        : { left: 0, right: 0, width: 0 };
    };
    return {
      pageOverflow: document.documentElement.scrollWidth > window.innerWidth + 1,
      navigationOverflow:
        (navigation?.scrollWidth ?? 0) > (navigation?.clientWidth ?? 0) + 1,
      statusOverflow: (status?.scrollWidth ?? 0) > (status?.clientWidth ?? 0) + 1,
      minimumSlotHeight: Math.min(...slots.map((slot) => slot.getBoundingClientRect().height)),
      primary: bounds(primary),
      navigation: bounds(navigation),
      statusFrame: bounds(statusFrame),
      modulePage: bounds(modulePage),
    };
  });
  expect(desktop.pageOverflow).toBe(false);
  expect(desktop.navigationOverflow).toBe(false);
  expect(desktop.statusOverflow).toBe(false);
  expect(desktop.minimumSlotHeight).toBeGreaterThanOrEqual(29);
  expect(Math.abs(desktop.primary.left - desktop.modulePage.left)).toBeLessThanOrEqual(1);
  expect(Math.abs(desktop.primary.width - desktop.modulePage.width)).toBeLessThanOrEqual(1);
  expect(Math.abs(desktop.statusFrame.left - desktop.modulePage.left)).toBeLessThanOrEqual(1);
  expect(Math.abs(desktop.statusFrame.width - desktop.modulePage.width)).toBeLessThanOrEqual(1);
  expect(desktop.navigation.width).toBeLessThanOrEqual(1080);
  expect(desktop.navigation.right).toBeLessThanOrEqual(desktop.primary.right + 1);

  await expect(primary).toBeVisible();
  await expect(statusFrame).toBeVisible();
  await expect(modulePage).toBeVisible();

  await page.setViewportSize({ width: 390, height: 844 });
  const mobile = await page.evaluate(() => {
    const navigation = document.querySelector<HTMLElement>(".module-tabs");
    const status = document.querySelector<HTMLElement>('[data-testid="top-status-bar"]');
    if (navigation) {
      navigation.scrollLeft = navigation.scrollWidth;
    }
    if (status) {
      status.scrollLeft = status.scrollWidth;
    }
    return {
      pageOverflow: document.documentElement.scrollWidth > window.innerWidth + 1,
      navigationOverflow:
        (navigation?.scrollWidth ?? 0) > (navigation?.clientWidth ?? 0) + 1,
      statusOverflow: (status?.scrollWidth ?? 0) > (status?.clientWidth ?? 0) + 1,
      pageScrollX: window.scrollX,
    };
  });
  expect(mobile.pageOverflow).toBe(false);
  expect(mobile.navigationOverflow).toBe(true);
  expect(mobile.statusOverflow).toBe(true);
  expect(mobile.pageScrollX).toBe(0);
});
