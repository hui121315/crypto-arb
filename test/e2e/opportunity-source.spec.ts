import { expect, test } from "@playwright/test";
import { API, NOW } from "./fixtures/opportunity-workbench";
import { settingsFixture } from "./fixtures/settings-workbench";

for (const module of ["futures", "opportunities"] as const) {
  test(`${module} never reuses quotes, search pages or detail across a connection change`, async ({ page }, info) => {
    const f = await settingsFixture(page, "diagnostics");
    await page.clock.install({ time: NOW });
    f.paginateList();
    const seed = await (await page.request.get(`${API}/api/v3/arbitrage/opportunities/list`)).json();
    let source = 0, hold = false, holdDetail = false;
    const pending: { source: number; kind: string; release: () => void; done: Promise<void> }[] = [];
    const reads: { source: number; symbol: string | null; cursor: string | null; auth: string; url: string }[] = [];
    await page.route("**/api/**", async route => {
      const request = route.request(), url = new URL(request.url());
      const path = url.pathname.replace("/e2e-candidates", "");
      if (path !== "/api/v3/arbitrage/opportunities/list" && !path.endsWith("/detail")) {
        if (!url.pathname.startsWith("/e2e-candidates/")) return route.fallback();
        if (request.method() !== "GET" && path !== "/api/auth/ws-ticket") return route.abort();
        return route.fulfill({ response: await route.fetch({ url: request.url().replace("/e2e-candidates", "") }) });
      }
      const captured = source, symbol = url.searchParams.get("symbol"), cursor = url.searchParams.get("cursor");
      const kind = path.endsWith("/detail") ? "detail" : symbol ? "search" : "page";
      const data = structuredClone(seed);
      if (kind === "detail") {
        const detail = await (await route.fetch({ url: request.url().replace("/e2e-candidates", "") })).json();
        detail.opportunity.id = path.split("/").at(-2);
        detail.requestId = `source-${captured}-detail`;
        Object.assign(data, detail);
      } else {
        reads.push({ source: captured, symbol, cursor, auth: request.headers().authorization ?? "", url: request.url() });
        const strategy = url.searchParams.get("strategy");
        const kinds = strategy?.split(",") ?? [];
        const row = structuredClone(f.rows.find(row => (!kinds.length || kinds.includes(row.strategyKind)) && (!symbol || row.symbol === symbol))!);
        row.longLeg.price = 62000 + captured * 10000;
        row.longLeg.marketEvidence.price = row.longLeg.price;
        if (cursor) row.id += "-page-2";
        Object.assign(data, { rows: [row], cachedAt: new Date(NOW + 10000 * captured + 1000).toISOString(),
          observedAtMs: NOW + 10000 * captured + 1000 });
        data.page = { ...data.page, snapshotId: `source-${captured}-${kind}`, totalRows: 2, returnedCount: 1,
          pageSize: 50, startOffset: cursor ? 1 : 0, hasNextPage: !cursor,
          nextCursor: cursor ? null : "page-2", previousCursor: null, lastCursor: cursor ? null : "page-2" };
        data.requestMeta = { fast: false, fresh: false,
          filter: { scope: "main_p0", strategyKinds: kinds, symbol, minYield: null },
          sortKey: data.page.sortKey, requestedPageSize: 50, appliedPageSize: 50, maxPageSize: 50 };
      }
      let finish: (() => void) | undefined;
      if (kind === "detail" ? holdDetail : hold) {
        const done = new Promise<void>(resolve => { finish = resolve; });
        await new Promise<void>(release => pending.push({ source: captured, kind, release, done }));
      }
      try { await route.fulfill({ json: data }); } finally { finish?.(); }
    });
    const release = async (generation: number) => {
      const batch = pending.filter(row => row.source === generation);
      batch.forEach(row => row.release());
      await Promise.all(batch.map(row => row.done));
      await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
    };
    const settings = async () => {
      await page.getByRole("button", { name: "切换到设置", exact: true }).click();
      await page.getByRole("tab", { name: "诊断", exact: true }).click();
      await page.getByRole("tab", { name: "连接", exact: true }).click();
    };
    const open = () => page.getByRole("button", { name: module === "futures" ? "切换到期货套利" : "切换到机会扫描", exact: true }).click();
    const token = async (value: string) => {
      await page.locator(".settings-api-token-task input").fill(value);
      await page.getByRole("button", { name: "保存 Token", exact: true }).click();
    };
    const rows = page.locator(module === "futures" ? ".futures-data-row" : ".opportunity-table tbody tr[id]");
    const search = page.getByPlaceholder(module === "futures" ? "BTC / BINANCE" : "币种 / 交易所 / 路由", { exact: true });
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto(`/#${module}`);
    await expect(rows).toHaveCount(1);
    hold = true;
    await page.getByRole("button", { name: "下一页", exact: true }).click();
    await expect.poll(() => pending.some(row => row.source === 0 && row.kind === "page")).toBe(true);
    await settings();
    source = 1; f.streamMode("silent");
    await token("isolated-candidate-login-b");
    await open();
    await expect(rows).toHaveCount(0);
    await release(0);
    await expect(rows).toHaveCount(0);
    expect(reads.some(row => row.source === 1 && row.cursor)).toBe(false);
    await page.clock.runFor(5_100);
    await expect(page.locator(".opportunity-stream-recovery")).toContainText("尚不能判断有无机会");
    await expect(page.locator(".table-pager strong")).toHaveText("分页数据待确认");
    await expect(page.locator(".toast-item")).toHaveCount(0);
    await page.screenshot({ path: info.outputPath(`${module}-new-source-waiting.png`) });

    for (const row of f.rows) {
      row.longLeg.price = 65000;
      row.longLeg.marketEvidence.price = 65000;
    }
    hold = false; f.streamMode("live"); f.tick(6000);
    await expect(rows).toHaveCount(1);
    await expect(rows).toContainText("65000.5");
    await search.fill("BTC");
    await page.clock.runFor(350);
    await expect(rows).toContainText("72000");
    if (module === "opportunities") {
      const detail = page.locator("#opportunity-detail-panel");
      await expect(detail.getByRole("button", { name: "刷新数据依据", exact: true })).toBeEnabled();
      holdDetail = true;
      await detail.getByRole("button", { name: "刷新数据依据", exact: true }).click();
      await expect.poll(() => pending.some(row => row.source === 1 && row.kind === "detail")).toBe(true);
    }
    hold = true;
    await page.getByRole("button", { name: "下一页", exact: true }).click();
    await expect.poll(() => pending.some(row => row.source === 1 && row.kind === "search")).toBe(true);
    await settings();
    source = 2; f.streamMode("silent");
    await token("isolated-fixture-token");
    await open();
    await expect(search).toHaveValue("BTC");
    await expect(rows).toHaveCount(0);
    await expect.poll(() => pending.some(row => row.source === 2 && row.kind === "search")).toBe(true);
    await release(1);
    await expect(rows).toHaveCount(0);
    if (module === "opportunities")
      await expect(page.locator("#opportunity-detail-panel")).not.toContainText("source-1-detail");
    hold = false; holdDetail = false;
    await release(2);
    await expect(rows).toHaveCount(1);
    await expect(rows).toContainText("82000");

    await settings();
    source = 3; hold = true;
    await page.getByRole("textbox", { name: "API Base", exact: true }).fill(`${API}/e2e-candidates`);
    await page.getByRole("textbox", { name: "确认应用", exact: true }).fill("apply");
    await page.getByRole("button", { name: "保存并应用", exact: true }).click();
    await open();
    await expect(rows).toHaveCount(0);
    await expect.poll(() => pending.some(row => row.source === 3 && row.kind === "search")).toBe(true);
    hold = false;
    await release(3);
    await expect(rows).toContainText("92000");
    expect(reads.filter(row => row.source === 3).every(row => row.url.includes("/e2e-candidates/") && row.auth === "Bearer isolated-fixture-token")).toBe(true);
    const build = rows.first().getByRole("button", { name: module === "futures" ? "构建新双腿" : "构建对冲", exact: true });
    await expect(build).toBeEnabled();
    await page.setViewportSize({ width: 390, height: 844 });
    await build.scrollIntoViewIfNeeded();
    await expect(build).toBeInViewport();
    await expect(page.locator(".toast-item")).toHaveCount(0);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    await page.screenshot({ path: info.outputPath(`${module}-new-source-ready.png`), fullPage: true });
    expect(f.writes).toEqual([]); expect(f.errors).toEqual([]);
  });
}
