import { test, expect, type Page } from "@playwright/test";
import { API, WEB, NOW, setup as marketSetup } from "./fixtures/opportunity-workbench";

async function setup(page: Page) {
  const base = await marketSetup(page);
  const catalog = ["BINANCE", "OKX", "KRAKEN"].flatMap((venue) => ["BTC", "ETH"].map((asset) => ({
    nativeSymbol: `${venue}_FUTURE_${asset}_USDT`, underlyingVenue: venue.toLowerCase(), product: "future",
    baseAsset: asset, quoteAsset: "USDT", displaySymbol: `${asset}/USDT`, executionSupported: true,
    listingStatus: "trading", sourceUrl: null,
  })));
  let config = { mode: "monitor", selectedRoutes: [catalog[0].nativeSymbol, catalog[2].nativeSymbol], minGrossSpreadPct: 0.1 };
  let reads = 0;
  let failRead = false;
  let failSave = false;
  let holdRead = false;
  let holdSave = false;
  let releaseRead: (() => void) | undefined;
  let releaseSave: (() => void) | undefined;
  let heldSearch: string | undefined;
  let releaseSearch: (() => void) | undefined;
  let failedSearch: string | undefined;
  let snapshotOverride: Record<string, unknown> = {};
  const patches: any[] = [];
  const snapshot = () => ({ config: structuredClone(config), runtimeState: config.mode === "disabled" ? "disabled" : "live",
    catalogCount: catalog.length, selectedCount: config.selectedRoutes.length, liveCount: config.mode === "disabled" ? 0 : config.selectedRoutes.length,
    routes: config.mode === "disabled" ? [] : catalog.filter((row) => config.selectedRoutes.includes(row.nativeSymbol)).map((row, i) => ({ ...row, bid: 100 + i, ask: 100.1 + i, last: 100.05 + i, observedAtMs: NOW })),
    candidates: config.mode === "disabled" || config.selectedRoutes.length < 2 ? [] : [{ product: "future", baseAsset: "BTC", quoteAsset: "USDT",
      longRoute: config.selectedRoutes[0], shortRoute: config.selectedRoutes[1], longAsk: 100.1, shortBid: 101,
      grossSpreadPct: 0.8991, synchronizedAtMs: NOW }], observedAtMs: NOW + reads, problem: null, ...snapshotOverride });
  await page.route(`${API}/api/system/gate-crossex**`, async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname.endsWith("/routes")) {
      const query = url.searchParams.get("search") ?? "";
      if (heldSearch === query) await new Promise<void>((resolve) => { releaseSearch = resolve; });
      if (failedSearch === query) return route.fulfill({ status: 503, json: { code: "CATALOG_OFFLINE", message: "fixture: catalog offline" } });
      const rows = catalog.filter((row) => row.nativeSymbol.includes(query.toUpperCase()));
      return route.fulfill({ json: { routes: rows, total: rows.length, generatedAtMs: NOW } });
    }
    if (route.request().method() === "PATCH") {
      const patch = route.request().postDataJSON();
      patches.push(patch);
      if (holdSave) await new Promise<void>((resolve) => { releaseSave = resolve; });
      if (failSave) return route.fulfill({ status: 503, json: { code: "SAVE_FAILED", message: "fixture: save failed" } });
      config = { ...config, ...Object.fromEntries(Object.entries(patch).filter(([, value]) => value != null)) };
      return route.fulfill({ json: snapshot() });
    }
    reads++;
    const value = snapshot();
    if (holdRead) { holdRead = false; await new Promise<void>((resolve) => { releaseRead = resolve; }); }
    if (failRead) return route.fulfill({ status: 503, json: { code: "OFFLINE", message: "fixture: snapshot offline", retryAfterMs: 60000 } });
    return route.fulfill({ json: value });
  });
  await page.goto(`${WEB}/#crossex`);
  await expect(page.locator(".gate-crossex-runtime")).toContainText("WS 实时");
  return { ...base, patches, catalog, get reads() { return reads; },
    failRead: (value: boolean) => { failRead = value; }, failSave: (value: boolean) => { failSave = value; },
    holdRead: () => { holdRead = true; }, releaseRead: () => { releaseRead?.(); },
    holdSave: () => { holdSave = true; }, releaseSave: () => { holdSave = false; releaseSave?.(); },
    holdSearch: (value: string) => { heldSearch = value; }, releaseSearch: () => { heldSearch = undefined; releaseSearch?.(); },
    failSearch: (value?: string) => { failedSearch = value; },
    override: (value: Record<string, unknown>) => { snapshotOverride = value; },
  };
}

test("CrossEx layout and polling preserve route focus", async ({ page }, info) => {
  const fixture = await setup(page);
  const checkbox = page.locator(".gate-crossex-route-option input").first();
  await checkbox.focus();
  const reads = fixture.reads;
  await expect.poll(() => fixture.reads).toBeGreaterThan(reads);
  for (const width of [1440, 1024, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await page.screenshot({ path: info.outputPath(`crossex-${width}.png`), fullPage: true });
  }
  await expect(checkbox).toBeFocused();
  for (const table of await page.locator(".gate-crossex-panel table").all()) {
    expect(await table.evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBeTruthy();
  }
  expect(fixture.errors).toEqual([]);
  expect(fixture.writes).toEqual([]);
});

test("CrossEx saves once and rejects a read started before the save", async ({ page }) => {
  const fixture = await setup(page);
  fixture.holdRead();
  const count = fixture.reads;
  await expect.poll(() => fixture.reads).toBeGreaterThan(count);
  fixture.holdSave();
  const option = page.getByRole("checkbox", { name: "KRAKEN_FUTURE_BTC_USDT", exact: true });
  await option.click();
  await expect.poll(() => fixture.patches.length).toBe(1);
  await expect(option).toBeDisabled();
  await expect(page.getByRole("button", { name: "关闭", exact: true })).toBeDisabled();
  fixture.releaseSave();
  await expect(page.locator(".gate-crossex-notice")).toHaveText("配置已保存");
  fixture.releaseRead();
  await expect(option).toBeChecked();
  await expect(page.locator(".gate-crossex-route-table tbody tr")).toHaveCount(3);
  await page.getByRole("button", { name: "关闭", exact: true }).click();
  await expect(page.locator(".gate-crossex-runtime")).toContainText("已关闭");
  await expect(page.locator(".gate-crossex-candidate-table tbody tr")).toHaveCount(0);
  await expect(page.locator(".gate-crossex-route-table tbody tr")).toHaveCount(3);
  await expect(page.locator(".gate-crossex-route-table tbody")).not.toContainText("100.0000");
  expect(fixture.patches).toHaveLength(2);
  expect(fixture.errors).toEqual([]);
  expect(fixture.writes).toEqual([]);
});

test("CrossEx does not keep a failed route selection or select a delisted route", async ({ page }) => {
  const fixture = await setup(page);
  const option = page.getByRole("checkbox", { name: "KRAKEN_FUTURE_BTC_USDT", exact: true });
  fixture.failSave(true);
  await option.click();
  await expect(page.locator(".gate-crossex-notice")).toContainText("保存失败");
  await expect(option).not.toBeChecked();
  fixture.catalog[4].listingStatus = "suspended";
  await page.getByRole("button", { name: "重读路由", exact: true }).click();
  await expect(option).toBeDisabled();
  expect(fixture.patches).toHaveLength(1);
  expect(fixture.errors).toEqual([]);
});

test("CrossEx keeps the decimal draft across refresh and save failure", async ({ page }) => {
  const fixture = await setup(page);
  const minimum = page.getByLabel("最小毛价差 (%)", { exact: true });
  await minimum.fill("0.00125");
  const count = fixture.reads;
  await page.getByRole("button", { name: "刷新状态", exact: true }).click();
  await expect.poll(() => fixture.reads).toBeGreaterThan(count);
  await expect(minimum).toHaveValue("0.00125");
  fixture.failSave(true);
  await page.getByRole("button", { name: "应用", exact: true }).click();
  await expect(page.locator(".gate-crossex-notice")).toContainText("保存失败");
  await expect(minimum).toHaveValue("0.00125");
  fixture.failSave(false);
  await page.getByRole("button", { name: "应用", exact: true }).click();
  await expect(page.locator(".gate-crossex-notice")).toHaveText("配置已保存");
  expect(fixture.patches[1].minGrossSpreadPct).toBe(0.00125);
  await minimum.fill("-1");
  await page.getByRole("button", { name: "应用", exact: true }).click();
  await expect(page.locator(".gate-crossex-notice")).toContainText("0% 到 100%");
  expect(fixture.patches).toHaveLength(2);
  expect(fixture.errors).toEqual([]);
});

test("CrossEx search isolates old results and supports retry and empty states", async ({ page }) => {
  const fixture = await setup(page);
  const search = page.getByRole("searchbox");
  const options = page.locator(".gate-crossex-route-option");
  fixture.holdSearch("BTC");
  await search.fill("BTC");
  await page.waitForRequest((request) => request.url().includes("/gate-crossex/routes?search=BTC"));
  await search.fill("ETH");
  await expect(options).toHaveCount(3);
  await expect(options.first()).toContainText("ETH");
  fixture.releaseSearch();
  await expect(options.first()).toContainText("ETH");
  fixture.failSearch("SOL");
  await search.fill("SOL");
  await expect(page.locator(".gate-crossex-route-state")).toContainText("路由读取失败");
  await expect(options).toHaveCount(0);
  fixture.failSearch();
  await page.getByRole("button", { name: "重读路由", exact: true }).click();
  await expect(page.locator(".gate-crossex-route-state")).toContainText("没有匹配");
  await search.fill("");
  await expect(options).toHaveCount(6);
  await page.getByLabel("只看已选", { exact: true }).check();
  await expect(options).toHaveCount(2);
  expect(fixture.errors).toEqual([]);
});

test("CrossEx reports stale reads honestly, recovers and ignores disposed callbacks", async ({ page }) => {
  const fixture = await setup(page);
  fixture.failRead(true);
  await page.getByRole("button", { name: "刷新状态", exact: true }).click();
  await expect(page.locator(".gate-crossex-runtime")).toContainText("上次快照");
  await expect(page.locator(".gate-crossex-runtime")).not.toContainText("WS 实时");
  await expect(page.locator(".gate-crossex-candidate-table tbody tr")).toHaveCount(1);
  await expect(page.getByRole("button", { name: "监控", exact: true })).toBeDisabled();
  fixture.failRead(false);
  await page.getByRole("button", { name: "刷新状态", exact: true }).click();
  await expect(page.locator(".gate-crossex-runtime")).toContainText("WS 实时");
  fixture.override({ runtimeState: "degraded", liveCount: 0, routes: [], candidates: [],
    problem: { code: "GATE_CROSSEX_QUOTES_PARTIAL", message: "fixture: selected routes unavailable" } });
  await page.getByRole("button", { name: "刷新状态", exact: true }).click();
  await expect(page.locator(".gate-crossex-runtime")).toContainText("部分行情未就绪");
  await expect(page.locator(".gate-crossex-route-table tbody tr")).toHaveCount(2);
  await expect(page.locator(".gate-crossex-route-table tbody")).toContainText("等待报价");
  await expect(page.locator(".workbench-table-empty")).toContainText("部分路由缺少报价");
  fixture.override({});
  fixture.holdRead();
  const count = fixture.reads;
  await expect.poll(() => fixture.reads).toBeGreaterThan(count);
  await page.locator('.module-tabs button[data-module="futures"]').click();
  fixture.releaseRead();
  await expect(page.getByRole("heading", { name: "期货套利", exact: true })).toBeVisible();
  expect(fixture.errors).toEqual([]);
  expect(fixture.writes).toEqual([]);
});
