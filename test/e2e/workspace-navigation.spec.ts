import { expect, test } from "@playwright/test";
import { API, NOW, setup } from "./fixtures/opportunity-workbench";

const modules = [
  ["positions", "持仓/风控"], ["futures", "期货套利"],
  ["opportunities", "机会扫描"], ["crossex", "CrossEx"],
  ["onchain", "链上套利"], ["stocks", "股票套利"],
  ["automation", "自动化"], ["execution", "对冲执行"],
  ["review", "复盘"], ["settings", "设置"],
] as const;

test("all ten modules keep one shell and usable navigation without writes", async ({ page }, info) => {
  const fixture = await setup(page);
  // Isolate page-state priority from the generic fixture's unrelated account failures.
  await page.route(API + "/api/system/venue-operation-health", route => route.fulfill({ json: {
    rows: ["ws_ticker_snapshot", "balances", "positions", "private_ws_account_stream"].map(operation => ({
      venue: "fixture", operation, configured: true, supported: true, status: "ok",
      source: "isolated_fixture", message: "fixture healthy", observedAtMs: NOW,
    })), generatedAtMs: NOW, rowCount: 4, attentionCount: 0,
  } }));
  await page.route(API + "/api/stocks/peer/plans", route => route.fulfill({ json: {
    security: null, tokens: [], books: [], connected: false,
    reference: null, problem: null, observedAtMs: NOW,
  } }));
  const requests: string[] = [];
  page.on("request", request => {
    if (request.resourceType() === "document") requests.push(request.url());
  });
  await page.goto("/#positions");
  const nav = page.getByRole("navigation", { name: "功能模块", exact: true });
  const main = page.locator("main.mod-content");
  const environment = page.getByRole("group", { name: "执行环境", exact: true });
  await expect(nav.getByRole("button")).toHaveCount(10);
  await expect(environment).toContainText("模拟");
  const summary = page.locator(".status-summary");
  const shellHeight = new Map<number, number>();
  // Keep a shell node across routes: replacing it would recreate shared state.
  await nav.evaluate(node => { node.setAttribute("data-navigation-probe", "original"); });
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    for (const [id, label] of modules) {
      await test.step(`${width}: ${label}`, async () => {
        const button = nav.locator(`button[data-module="${id}"]`);
        await button.click();
        await expect(page).toHaveURL(new RegExp(`#${id}$`));
        await expect(nav.locator('[aria-current="page"]')).toHaveCount(1);
        await expect(button).toHaveAttribute("aria-current", "page");
        await expect(nav).toHaveAttribute("data-navigation-probe", "original");
        await expect.soft(page.getByRole("main")).toHaveCount(1);
        await expect(main).toBeVisible();
        await expect(environment).toContainText("模拟");
        await expect.poll(() => main.innerText()).not.toBe("");
        const primaryCopy = await main.locator('h1, h2, h3, button, [role="tab"]').allTextContents();
        expect.soft(primaryCopy.join("\n")).not.toMatch(/工件|门禁|终态|闭环|净敞口|运行态/);
        const height = await page.locator(".mod-topbar").evaluate(node => node.getBoundingClientRect().height);
        if (!shellHeight.has(width)) shellHeight.set(width, height);
        expect.soft(height).toBe(shellHeight.get(width));
        if (["error", "stale"].includes(await button.getAttribute("data-runtime-state") ?? "")) {
          await expect(summary).not.toHaveAttribute("data-state", "healthy");
        }
        const rect = await button.boundingBox();
        expect(rect).not.toBeNull();
        expect.soft(rect!.x).toBeGreaterThanOrEqual(-1);
        expect.soft(rect!.x + rect!.width).toBeLessThanOrEqual(width + 1);
        const geometry = await page.evaluate(() => ({
          overflow: document.documentElement.scrollWidth - innerWidth,
          top: document.querySelector("main")!.getBoundingClientRect().top,
          bottom: document.querySelector(".mod-topbar")!.getBoundingClientRect().bottom,
        }));
        expect.soft(geometry.overflow).toBeLessThanOrEqual(1);
        expect.soft(geometry.top).toBeGreaterThanOrEqual(geometry.bottom - 1);
        await page.screenshot({ path: info.outputPath(`${id}-${width}.png`) });
      });
    }
  }
  await nav.locator('button[data-module="stocks"]').click();
  await page.getByRole("button", { name: "选择股票", exact: true }).click();
  const catalog = page.getByRole("complementary", { name: "Backpack 股票目录", exact: true });
  await expect(catalog.locator(".stock-catalog-count")).toHaveText("证券数量待确认");
  await expect(nav.locator('[aria-current="page"]')).toHaveAttribute("data-runtime-state", "error");
  await expect(summary).toContainText("当前模块异常");
  let catalogFailed = false;
  await page.route(API + "/api/stocks/catalog", route => catalogFailed
    ? route.fulfill({ status: 503, json: { code: "CATALOG_UNAVAILABLE", message: "fixture: catalog offline" } })
    : route.fulfill({ json: { rows: [], observedAtMs: NOW } }));
  await catalog.getByRole("button", { name: "刷新股票目录", exact: true }).click();
  await expect(catalog.locator(".stock-catalog-count")).toHaveText("0 个证券");
  await expect(summary).not.toContainText("当前模块异常");
  catalogFailed = true;
  await catalog.getByRole("button", { name: "刷新股票目录", exact: true }).click();
  await expect(catalog.locator(".stock-catalog-count")).toHaveText("0 个证券 · 上次目录");
  await expect(summary).toContainText("当前数据已过期");
  await nav.locator('button[data-module="onchain"]').click();
  await expect(nav.locator('[aria-current="page"]')).toHaveAttribute("data-runtime-state", "error");
  await expect(summary).toContainText("当前模块异常");
  await summary.click();
  const moduleEvidence = page.getByRole("group", { name: "当前模块状态", exact: true });
  await expect(moduleEvidence).toContainText("链上套利");
  await expect(moduleEvidence).toContainText("NOT_FOUND");
  const { data: health } = await (await page.request.get(API + "/api/system/health")).json();
  const systemSockets = fixture.channelSockets.get("system")!;
  expect(systemSockets.size).toBeGreaterThan(0);
  for (const socket of systemSockets) {
    socket.send(JSON.stringify({ type: "message", channel: "system", payload: { ...health, risk: "block", updatedAtMs: NOW + 1 } }));
  }
  await expect(summary).toContainText("风险已阻断");
  await expect(moduleEvidence).toContainText("链上套利");
  await page.screenshot({ path: info.outputPath("risk-priority-mobile.png") });
  for (const socket of systemSockets) {
    socket.send(JSON.stringify({ type: "message", channel: "system", payload: { ...health, updatedAtMs: NOW + 2 } }));
  }
  await expect(summary).toContainText("当前模块异常");
  await summary.click();
  await nav.locator('button[data-module="futures"]').click();
  fixture.tick();
  await expect(summary).toContainText("运行正常");
  expect(requests).toHaveLength(1);
  expect(fixture.writes).toEqual([]);
  expect(fixture.errors).toEqual([]);
});
