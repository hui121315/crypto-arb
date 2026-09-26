import { test, expect, type Page } from "@playwright/test";
import { API, WEB, NOW, setup as marketSetup } from "./fixtures/opportunity-workbench";

async function setup(page: Page) {
  const base = await marketSetup(page);
  const actions = await (await page.request.get(`${API}/api/trading/action-runs`)).json();
  actions.data = [];
  let holdAction = false;
  let releaseAction: (() => void) | undefined;
  await page.route(`${API}/api/trading/action-runs**`, async (route) => {
    const path = new URL(route.request().url()).pathname;
    const data = structuredClone(path === "/api/trading/action-runs" ? actions : actions.data.find((row: any) => path.endsWith(`/${row.id}`)));
    if (holdAction) { holdAction = false; await new Promise<void>((resolve) => { releaseAction = resolve; }); }
    return data ? route.fulfill({ json: data }) : route.fulfill({ status: 404, json: { code: "NOT_FOUND", message: "fixture: no original receipt" } });
  });
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
  let dropSave = false;
  let catalogLimit = 80;
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
      return route.fulfill({ json: { routes: rows.slice(0, catalogLimit), total: rows.length, generatedAtMs: NOW } });
    }
    if (route.request().method() === "PATCH") {
      const patch = route.request().postDataJSON();
      patches.push(patch);
      const headers = route.request().headers();
      const run = { id: `fixture-crossex-${patches.length}`, kind: "gate_cross_ex_mode_update", target: "gate_crossex",
        requestId: headers["x-request-id"], idempotencyKey: headers["idempotency-key"], status: "accepted",
        actor: "fixture", message: "fixture accepted", startedAtMs: NOW, updatedAtMs: NOW,
        result: null as any, problem: null as any };
      actions.data.unshift(run);
      if (holdSave) await new Promise<void>((resolve) => { releaseSave = resolve; });
      if (failSave) {
        run.status = "failed"; run.problem = { code: "SAVE_REJECTED", message: "fixture: save failed", status: 400 };
        return route.fulfill({ status: 400, json: run.problem });
      }
      if (patch.selectedRoutes?.some((native: string) => !catalog.some((row) => row.nativeSymbol === native && row.listingStatus === "trading"))) {
        run.status = "failed"; run.problem = { code: "INVALID_ROUTE", message: "fixture: selected route is not trading", status: 400 };
        return route.fulfill({ status: 400, json: run.problem });
      }
      config = { ...config, ...Object.fromEntries(Object.entries(patch).filter(([, value]) => value != null)) };
      config.selectedRoutes = [...new Set(config.selectedRoutes.map((value) => value.trim().toUpperCase()))].sort();
      run.status = "succeeded"; run.result = snapshot();
      if (dropSave) { dropSave = false; return route.abort("connectionreset"); }
      return route.fulfill({ json: run.result });
    }
    reads++;
    const value = snapshot();
    if (holdRead) { holdRead = false; await new Promise<void>((resolve) => { releaseRead = resolve; }); }
    if (failRead) return route.fulfill({ status: 503, json: { code: "OFFLINE", message: "fixture: snapshot offline", retryAfterMs: 60000 } });
    return route.fulfill({ json: value });
  });
  await page.goto(`${WEB}/#crossex`);
  await expect(page.locator(".gate-crossex-runtime")).toContainText("WS 实时");
  return { ...base, patches, catalog, snapshot, actions, get reads() { return reads; },
    setConfig: (value: Partial<typeof config>) => { config = { ...config, ...value }; },
    catalogLimit: (value: number) => { catalogLimit = value; },
    dropSave: () => { dropSave = true; },
    holdAction: () => { holdAction = true; }, releaseAction: () => { releaseAction?.(); },
    failRead: (value: boolean) => { failRead = value; }, failSave: (value: boolean) => { failSave = value; },
    holdRead: () => { holdRead = true; }, releaseRead: () => { releaseRead?.(); },
    holdSave: () => { holdSave = true; }, releaseSave: () => { holdSave = false; releaseSave?.(); },
    holdSearch: (value: string) => { heldSearch = value; }, releaseSearch: () => { heldSearch = undefined; releaseSearch?.(); },
    failSearch: (value?: string) => { failedSearch = value; },
    override: (value: Record<string, unknown>) => { snapshotOverride = value; },
  };
}

test("CrossEx reload recovers original saves and manages selected routes outside the catalog window", async ({ page }, info) => {
  const fixture = await setup(page);
  const recovery = page.getByRole("alert", { name: "设置操作待核对" });
  const recheck = recovery.getByRole("button", { name: "核对上次操作", exact: true });
  const minimum = page.getByLabel("最小毛价差 (%)", { exact: true });
  const apply = page.getByRole("button", { name: "应用", exact: true });
  const records = () => page.evaluate(() => Object.entries(sessionStorage).filter(([key]) => key.startsWith("crossline.settings.pending.v1:crossex:")));
  fixture.holdSave();
  await minimum.fill("0.00125");
  await apply.click();
  await expect.poll(() => fixture.patches.length).toBe(1);
  const savedRecord = await records();
  expect(savedRecord).toHaveLength(1);
  const original = JSON.parse(savedRecord[0][1]);
  expect(Object.keys(original).sort()).toEqual(["context", "kind", "run_id", "target", "version"]);
  expect(JSON.stringify(savedRecord)).not.toContain("isolated-fixture-token");
  expect(JSON.stringify(savedRecord)).not.toContain("minGrossSpreadPct");
  await page.reload();
  await expect(recovery).toContainText("保存 CrossEx 配置结果待核对");
  await expect(minimum).toBeDisabled();
  await expect(minimum).toHaveValue("0.1");
  await expect(page.getByRole("button", { name: "关闭", exact: true })).toBeDisabled();
  const run = fixture.actions.data[0];
  fixture.actions.data = [];
  await recheck.click();
  await expect(recovery).toContainText("SETTINGS_RECEIPT_NOT_FOUND");
  fixture.actions.data = [run];
  await recheck.click();
  await expect(recovery).toContainText("后端已受理");
  run.requestId = "unrelated-request";
  await recheck.click();
  await expect(recovery).toContainText("SETTINGS_RECEIPT_MISMATCH");
  run.requestId = original.context.request_id;
  fixture.releaseSave();
  await expect.poll(() => run.status).toBe("succeeded");
  const receipt = structuredClone(run.result);
  run.result = null;
  await recheck.click();
  await expect(recovery).toContainText("SETTINGS_RECEIPT_MISSING");
  run.result = { ...receipt, selectedCount: 99 };
  await recheck.click();
  await expect(recovery).toContainText("SETTINGS_RECEIPT_MISMATCH");
  await expect(minimum).toBeDisabled();
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await recovery.scrollIntoViewIfNeeded();
    await expect(recheck).toBeInViewport();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBeTruthy();
    await page.screenshot({ path: info.outputPath(`crossex-recovery-${width}.png`) });
  }
  run.result = receipt;
  fixture.setConfig({ minGrossSpreadPct: 0.75,
    selectedRoutes: ["BINANCE_FUTURE_BTC_USDT", "OKX_FUTURE_BTC_USDT", "KRAKEN_FUTURE_BTC_USDT"] });
  fixture.catalogLimit(1);
  fixture.override({ runtimeState: "warming", liveCount: 0, routes: [], candidates: [] });
  fixture.holdAction();
  const checking = page.waitForRequest((request) => request.url().endsWith(`/action-runs/${run.id}`));
  await recheck.click();
  await checking;
  await page.locator('.module-tabs button[data-module="futures"]').click();
  fixture.releaseAction();
  await expect.poll(async () => (await records()).length).toBe(0);
  await page.locator('.module-tabs button[data-module="crossex"]').click();
  await expect(minimum).toBeEnabled();
  await expect(minimum).toHaveValue("0.75");
  await expect(recovery).toHaveCount(0);
  await expect(page.locator(".gate-crossex-runtime")).toContainText("等待 WS 报价");
  await expect(page.locator(".gate-crossex-candidate-table tbody tr")).toHaveCount(0);
  await expect(page.locator(".gate-crossex-route-option")).toHaveCount(1);
  await page.getByLabel("只看已选", { exact: true }).check();
  await expect(page.locator(".gate-crossex-route-option")).toHaveCount(3);
  const kraken = "KRAKEN_FUTURE_BTC_USDT";
  await expect(page.getByRole("checkbox", { name: kraken, exact: true })).toBeChecked();
  const remove = page.getByRole("button", { name: `移除 ${kraken}`, exact: true });
  fixture.override({});
  await page.getByRole("button", { name: "刷新状态", exact: true }).click();
  await expect(page.locator(".gate-crossex-runtime")).toContainText("WS 实时");
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await remove.scrollIntoViewIfNeeded();
    await expect(remove).toBeInViewport();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBeTruthy();
    for (const value of await page.locator(".gate-crossex-bid-value").all()) {
      expect(await value.evaluate((element) => {
        const range = document.createRange(); range.selectNodeContents(element);
        return range.getClientRects().length;
      })).toBe(1);
    }
    await page.screenshot({ path: info.outputPath(`crossex-selected-${width}.png`) });
  }
  fixture.dropSave();
  await remove.click();
  await expect(page.locator(".gate-crossex-notice")).toContainText("保存结果待核对");
  await expect(page.getByRole("button", { name: "清空已选", exact: true })).toBeDisabled();
  await recheck.click();
  await expect(recovery).toHaveCount(0);
  await expect(page.locator(".gate-crossex-route-table tbody tr")).toHaveCount(2);
  expect(fixture.patches).toHaveLength(2);
  fixture.override({ runtimeState: "warming", liveCount: 0, routes: [], candidates: [] });
  fixture.catalog[2].listingStatus = "suspended";
  await page.getByRole("button", { name: "移除 BINANCE_FUTURE_BTC_USDT", exact: true }).click();
  await expect(page.locator(".gate-crossex-notice")).toContainText("保存失败");
  await expect(recovery).toHaveCount(0);
  await expect(page.locator(".gate-crossex-route-table tbody tr")).toHaveCount(2);
  await page.getByRole("button", { name: "清空已选", exact: true }).click();
  await expect(page.locator(".gate-crossex-route-table tbody tr")).toHaveCount(0);
  await expect(page.locator(".gate-crossex-runtime")).toContainText("待选择路由");
  await expect(page.locator(".gate-crossex-notice")).toHaveText("配置已保存");
  expect(fixture.patches).toHaveLength(4);
  expect(fixture.patches.at(-1).selectedRoutes).toEqual([]);
  expect(await records()).toEqual([]);
  expect(fixture.errors).toEqual([]);
  expect(fixture.writes).toEqual([]);
});

for (const kind of ["status", "catalog"] as const) {
  test(`CrossEx ${kind} timeout cancels the read and preserves an exact retry`, async ({ page }, info) => {
    await page.clock.install({ time: NOW });
    const fixture = await setup(page);
    const path = kind === "status" ? "/api/system/gate-crossex" : "/api/system/gate-crossex/routes";
    const refresh = page.getByRole("button", { name: kind === "status" ? "刷新状态" : "重读路由", exact: true });
    const state = page.locator(kind === "status" ? ".gate-crossex-runtime" : ".gate-crossex-route-picker");
    const minimum = page.getByLabel("最小毛价差 (%)", { exact: true });
    await minimum.fill("0.00125");
    let hold = true;
    let attempts = 0;
    let cancelled = 0;
    let release: (() => void) | undefined;
    let lateReplyReleased = false;
    const queries: string[] = [];
    const late = kind === "status" ? { ...fixture.snapshot(), config: { ...fixture.snapshot().config, minGrossSpreadPct: 77 },
      candidates: fixture.snapshot().candidates.map(row => ({ ...row, longAsk: 77777 })) }
      : { routes: fixture.catalog.filter(row => row.baseAsset === "ETH"), total: 3, generatedAtMs: NOW };
    page.on("requestfailed", request => { if (new URL(request.url()).pathname === path) cancelled++; });
    await page.route(`${API}${path}**`, async route => {
      const url = new URL(route.request().url());
      if (url.pathname !== path || route.request().method() !== "GET") return route.fallback();
      attempts++;
      queries.push(url.searchParams.get("search") ?? "");
      if (!hold) return route.fallback();
      await new Promise<void>(resolve => { release = resolve; });
      await route.fulfill({ json: late });
      lateReplyReleased = true;
    });
    if (kind === "catalog") {
      await page.getByRole("searchbox").fill("BTC");
      await page.clock.fastForward(250);
    } else await refresh.click();
    await expect.poll(() => attempts).toBe(1);
    await page.clock.fastForward(15_100);
    await expect(state).toContainText("后台状态读取超过 15 秒未返回");
    await expect.poll(() => cancelled).toBe(1);
    await expect(refresh).toBeEnabled();
    await expect(minimum).toHaveValue("0.00125");
    await expect(page.locator(".gate-crossex-candidate-table")).not.toContainText("77777");
    expect(attempts).toBe(1);
    if (kind === "status") {
      await expect(state).not.toContainText("WS 实时");
      await expect(page.locator(".gate-crossex-candidate-table tbody")).toContainText("上次快照");
      await expect(page.getByRole("button", { name: "应用", exact: true })).toBeDisabled();
    } else await expect(page.locator(".gate-crossex-route-option")).toHaveCount(0);
    await page.setViewportSize({ width: 390, height: 900 });
    await state.scrollIntoViewIfNeeded();
    await page.screenshot({ path: info.outputPath(`crossex-${kind}-timeout-390.png`) });
    hold = false;
    await refresh.click();
    await expect.poll(() => attempts).toBe(2);
    await expect(state).not.toContainText("后台状态读取超过 15 秒未返回");
    if (kind === "status") await expect(state).toContainText("WS 实时");
    else {
      await expect(page.locator(".gate-crossex-route-option")).toHaveCount(3);
      await expect(page.locator(".gate-crossex-route-option").first()).toContainText("BTC");
      expect(queries).toEqual(["BTC", "BTC"]);
    }
    release!();
    await expect.poll(() => lateReplyReleased).toBe(true);
    await expect(minimum).toHaveValue("0.00125");
    await expect(page.locator(".gate-crossex-candidate-table")).not.toContainText("77777");
    if (kind === "catalog") await expect(page.locator(".gate-crossex-route-option").first()).toContainText("BTC");
    hold = true;
    lateReplyReleased = false;
    await refresh.click();
    await expect.poll(() => attempts).toBe(3);
    await page.locator('.module-tabs button[data-module="futures"]').click();
    await expect.poll(() => cancelled).toBe(2);
    hold = false;
    await page.locator('.module-tabs button[data-module="crossex"]').click();
    await expect.poll(() => attempts).toBe(4);
    release!();
    await expect.poll(() => lateReplyReleased).toBe(true);
    await expect(page.locator(".gate-crossex-runtime")).toContainText("WS 实时");
    await expect(minimum).toHaveValue("0.00125");
    await expect(page.locator(".gate-crossex-candidate-table")).not.toContainText("77777");
    expect(fixture.patches).toEqual([]);
    expect(fixture.errors).toEqual([]);
    expect(fixture.writes).toEqual([]);
  });
}

test("CrossEx layout and polling preserve route focus", async ({ page }, info) => {
  const fixture = await setup(page);
  const initial = fixture.snapshot();
  fixture.override({ routes: initial.routes.map((row, i) => ({ ...row, bid: 64000.12 + i, ask: 64000.34 + i, last: 64000.23 + i })),
    candidates: initial.candidates.map(row => ({ ...row, longAsk: 64000.34, shortBid: 64001.12 })) });
  await page.getByRole("button", { name: "刷新状态", exact: true }).click();
  await expect(page.locator(".gate-crossex-candidate-table")).toContainText("64000.34");
  const checkbox = page.locator(".gate-crossex-route-option input").first();
  await checkbox.focus();
  const reads = fixture.reads;
  await expect.poll(() => fixture.reads).toBeGreaterThan(reads);
  for (const width of [1440, 1024, 820, 720, 390, 320]) {
    await page.setViewportSize({ width, height: 900 });
    for (const row of await page.locator(".gate-crossex-panel tbody tr").all()) {
      const boxes = await row.locator("td").evaluateAll(cells => cells.map(cell => {
        const box = cell.getBoundingClientRect();
        return { x: box.x, y: box.y, right: box.right, bottom: box.bottom, width: box.width, height: box.height,
          overflow: cell.scrollWidth > cell.clientWidth + 1 };
      }));
      for (const [index, box] of boxes.entries()) {
        expect(box.width).toBeGreaterThan(0);
        expect(box.height).toBeGreaterThan(0);
        expect(box.overflow).toBe(false);
        for (const other of boxes.slice(index + 1)) expect(
          Math.min(box.right, other.right) - Math.max(box.x, other.x) > 1
          && Math.min(box.bottom, other.bottom) - Math.max(box.y, other.y) > 1
        ).toBe(false);
      }
    }
    await expect(page.locator(".gate-crossex-candidate-table tbody .gate-crossex-time")).toBeVisible();
    await expect(page.locator(".gate-crossex-route-table tbody .gate-crossex-last").first()).toBeVisible();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    if (width <= 760) {
      const market = await page.locator(".gate-crossex-main").boundingBox();
      const controls = await page.locator(".gate-crossex-control").boundingBox();
      expect(market!.y + market!.height).toBeLessThanOrEqual(controls!.y + 1);
    }
    await page.screenshot({ path: info.outputPath(`crossex-${width}.png`), fullPage: true });
  }
  await expect(checkbox).toBeFocused();
  for (const table of await page.locator(".gate-crossex-panel table").all()) {
    expect(await table.evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBeTruthy();
  }
  await page.getByRole("button", { name: "管理路由", exact: true }).click();
  await expect(page.locator("#crossex-controls")).toBeFocused();
  await expect(page.getByRole("button", { name: "监控", exact: true })).toBeInViewport();
  await page.getByRole("button", { name: "返回行情", exact: true }).click();
  await expect(page.locator("#crossex-quotes")).toBeFocused();
  await expect(page.locator(".gate-crossex-candidate-table tbody tr")).toBeInViewport();
  expect(fixture.patches).toEqual([]);
  expect(fixture.errors).toEqual([]);
  expect(fixture.writes).toEqual([]);
});

test("CrossEx saves once across navigation and rejects a read started before the save", async ({ page }) => {
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
  const readsWhileSaving = fixture.reads;
  await page.locator('.module-tabs button[data-module="futures"]').click();
  await page.locator('.module-tabs button[data-module="crossex"]').click();
  await expect(option).toBeDisabled();
  await expect(page.locator(".gate-crossex-notice")).toHaveText("正在保存…");
  expect(fixture.reads).toBe(readsWhileSaving);
  expect(fixture.patches).toHaveLength(1);
  await page.locator('.module-tabs button[data-module="futures"]').click();
  const saved = page.waitForResponse((response) => response.url().endsWith("/gate-crossex/config") && response.request().method() === "PATCH");
  fixture.releaseSave();
  await saved;
  await page.locator('.module-tabs button[data-module="crossex"]').click();
  await expect(page.locator(".gate-crossex-notice")).toHaveText("配置已保存");
  await expect(option).toBeChecked();
  await expect.poll(() => fixture.reads).toBeGreaterThan(readsWhileSaving);
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

test("CrossEx keeps the decimal draft and save failure across navigation", async ({ page }, info) => {
  const fixture = await setup(page);
  const minimum = page.getByLabel("最小毛价差 (%)", { exact: true });
  await minimum.fill("0.00125");
  await page.locator('.module-tabs button[data-module="futures"]').click();
  await page.locator('.module-tabs button[data-module="crossex"]').click();
  await expect(minimum).toHaveValue("0.00125");
  const count = fixture.reads;
  await page.getByRole("button", { name: "刷新状态", exact: true }).click();
  await expect.poll(() => fixture.reads).toBeGreaterThan(count);
  await expect(minimum).toHaveValue("0.00125");
  fixture.failSave(true);
  fixture.holdSave();
  await page.getByRole("button", { name: "应用", exact: true }).click();
  await expect.poll(() => fixture.patches.length).toBe(1);
  await page.locator('.module-tabs button[data-module="futures"]').click();
  const failed = page.waitForResponse((response) => response.url().endsWith("/gate-crossex/config") && response.request().method() === "PATCH");
  fixture.releaseSave();
  await failed;
  await page.locator('.module-tabs button[data-module="crossex"]').click();
  await expect(page.locator(".gate-crossex-notice")).toContainText("保存失败");
  await expect(minimum).toHaveValue("0.00125");
  await expect(minimum).toBeEnabled();
  await page.setViewportSize({ width: 390, height: 900 });
  await page.locator(".gate-crossex-control").screenshot({ path: info.outputPath("crossex-save-retry-390.png") });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth + 1)).toBeTruthy();
  fixture.failSave(false);
  await page.getByRole("button", { name: "应用", exact: true }).click();
  await expect(page.locator(".gate-crossex-notice")).toHaveText("配置已保存");
  expect(fixture.patches[1].minGrossSpreadPct).toBe(0.00125);
  await expect(minimum).toHaveValue("0.00125");
  await expect(page.getByRole("button", { name: "应用", exact: true })).toBeDisabled();
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

test("CrossEx frozen quotes age across navigation and identical refreshes", async ({ page }, info) => {
  await page.clock.install({ time: NOW });
  const fixture = await setup(page);
  const baseline = fixture.snapshot();
  fixture.override({ observedAtMs: NOW, routes: baseline.routes, candidates: baseline.candidates });
  const runtime = page.locator(".gate-crossex-runtime");
  const refresh = page.getByRole("button", { name: "刷新状态", exact: true });
  await refresh.click();
  await expect(runtime).toContainText("WS 实时");
  const reads = fixture.reads;
  await page.clock.fastForward(1000);
  await expect.poll(() => fixture.reads).toBeGreaterThan(reads);
  await page.clock.fastForward(31000);
  await expect(runtime).toContainText("报价已过期");
  await expect(runtime).not.toContainText("WS 实时");
  await expect(runtime.locator("div").filter({ hasText: "WS 报价 / 已选" })).toContainText("0 / 2");
  await expect(runtime.locator("div").filter({ hasText: "毛价差候选" }).locator("strong")).toHaveText("0");
  await expect(page.locator(".gate-crossex-candidate-table tbody .is-positive")).toHaveCount(0);
  await expect(page.locator(".gate-crossex-candidate-table tbody")).toContainText("上次快照");
  await expect(page.locator(".gate-crossex-route-table tbody")).toContainText("过期");
  await page.locator('.module-tabs button[data-module="futures"]').click();
  await page.locator('.module-tabs button[data-module="crossex"]').click();
  await expect(runtime).toContainText("报价已过期");
  await refresh.click();
  await expect(runtime).toContainText("报价已过期");
  await page.getByLabel("最小毛价差 (%)", { exact: true }).fill("0.00125");
  await page.getByRole("button", { name: "应用", exact: true }).click();
  await expect(page.locator(".gate-crossex-notice")).toHaveText("配置已保存");
  await expect(runtime).toContainText("报价已过期");
  await page.setViewportSize({ width: 390, height: 900 });
  await runtime.screenshot({ path: info.outputPath("crossex-expired-390.png") });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth + 1)).toBeTruthy();
  const freshAt = NOW + 40000;
  fixture.override({ observedAtMs: freshAt,
    routes: baseline.routes.map((row, index) => ({ ...row, observedAtMs: index ? NOW : freshAt })),
    candidates: baseline.candidates,
  });
  await refresh.click();
  await expect(runtime).toContainText("部分报价已过期");
  await expect(runtime.locator("div").filter({ hasText: "WS 报价 / 已选" })).toContainText("1 / 2");
  await expect(page.locator(".gate-crossex-candidate-table tbody .is-positive")).toHaveCount(0);
  fixture.override({ observedAtMs: freshAt,
    routes: baseline.routes.map((row) => ({ ...row, observedAtMs: freshAt })),
    candidates: baseline.candidates.map((row) => ({ ...row, synchronizedAtMs: freshAt })),
  });
  await refresh.click();
  await expect(runtime).toContainText("WS 实时");
  await expect(runtime.locator("div").filter({ hasText: "WS 报价 / 已选" })).toContainText("2 / 2");
  await expect(page.locator(".gate-crossex-candidate-table tbody .is-positive")).toHaveCount(1);
  await expect(page.locator(".gate-crossex-candidate-table tbody")).not.toContainText("上次快照");
  await page.getByRole("button", { name: "关闭", exact: true }).click();
  fixture.override({});
  await refresh.click();
  await page.clock.fastForward(31000);
  await expect(runtime).toContainText("已关闭");
  await expect(runtime).not.toContainText("报价已过期");
  expect(fixture.patches).toHaveLength(2);
  expect(fixture.errors).toEqual([]);
  expect(fixture.writes).toEqual([]);
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
