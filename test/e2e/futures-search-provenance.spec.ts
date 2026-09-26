import { expect, test } from "@playwright/test";
import { NOW, setup } from "./fixtures/opportunity-workbench";

test("searched rows use their own latest source and cannot borrow search freshness", async ({ page }) => {
  await page.clock.install({ time: NOW });
  const f = await setup(page);
  const live = f.rows[0];
  const outside = structuredClone(live);
  outside.id += "-outside-window";
  outside.longLeg.venue = "kraken";
  outside.longLeg.action = "kraken 做多永续";
  outside.longLeg.price = 65000;
  Object.assign(outside.longLeg.marketEvidence, { venue: "kraken", price: 65000 });
  f.rows.push(outside);
  f.paginateList();
  f.transformSearch((response) => {
    response.cachedAt = new Date(NOW + 200).toISOString();
    response.observedAtMs = NOW + 200;
    response.page.snapshotId = "newer-search";
    response.rows[0].longLeg.price = 61000;
    response.rows[0].longLeg.marketEvidence.price = 61000;
  });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/#futures");
  await page.getByPlaceholder("BTC / BINANCE", { exact: true }).fill("BTC");
  const rows = page.locator(".futures-data-row");
  const first = rows.filter({ hasText: "binance 做多永续" });
  const other = rows.filter({ hasText: "kraken 做多永续" });
  const build = (row: typeof first) => row.getByRole("button", { name: "构建新双腿", exact: true });
  await expect(rows).toHaveCount(2);
  await expect(first).toContainText("61000");
  await expect(build(first)).toBeEnabled();
  await expect(page.locator(".futures-search-status")).toContainText("WS 报价 0 条 · 搜索快照 2 条");

  f.tick(1000);
  await expect(first).toContainText("60000.5");
  await expect(other).toContainText("65000");
  await expect(page.locator(".futures-search-status")).toContainText("WS 报价 1 条 · 搜索快照 1 条");
  f.stale(true);
  await expect(build(first)).toBeDisabled();
  await expect(first).toContainText("上次报价");
  await expect(build(other)).toBeEnabled();
  await expect(page.locator(".futures-search-status")).toContainText("WS 报价 1 条待更新");
  await expect(page.locator(".futures-kpis .large-kpi").nth(1).locator("strong")).toHaveText("1");
  const eligibility = page.getByRole("group", { name: "按可执行性筛选", exact: true });
  await expect(eligibility.getByRole("button", { name: /可预检/ })).toHaveText("可检查交易1");
  await eligibility.getByRole("button", { name: /可预检/ }).click();
  await expect(rows).toHaveCount(1);
  await expect(other).toBeVisible();
  await eligibility.getByRole("button", { name: /行情证据不完整/ }).click();
  await expect(rows).toHaveCount(1);
  await expect(build(first)).toBeDisabled();
  await eligibility.getByRole("button", { name: /全部/ }).click();
  await expect(rows).toHaveCount(2);
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await build(first).scrollIntoViewIfNeeded();
    await expect(build(first)).toBeInViewport();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    await page.screenshot({ path: test.info().outputPath(`futures-mixed-source-${width}.png`), fullPage: true });
  }
  await page.setViewportSize({ width: 1440, height: 900 });
  live.execution.eligible = false;
  live.execution.blockers = ["fixture: 当前结算窗口不再对齐"];
  f.stale(false);
  await expect(build(first)).toHaveCount(0);
  await expect(first).toContainText("仅观察");
  await expect(build(other)).toBeEnabled();
  live.execution.eligible = true;
  live.execution.blockers = [];
  f.tick();
  await expect(build(first)).toBeEnabled();
  const count = f.listRequests.length;
  await page.clock.fastForward(31_000);
  f.tick();
  await expect(build(first)).toBeDisabled();
  await expect(build(other)).toBeDisabled();
  expect(f.listRequests).toHaveLength(count);
  await expect(page.locator(".futures-search-status")).toContainText("搜索快照已过期");
  f.transformSearch();
  await page.getByRole("button", { name: "重新搜索", exact: true }).click();
  await expect(build(other)).toBeEnabled();
  await build(other).click();
  await expect(page.locator(".execution-ticket h3")).toHaveText("BTC · 永续跨所");
  await expect(page.locator(".execution-ticket")).toContainText(/kraken/i);
  await page.goto("/#opportunities");
  const scanRows = page.locator(".opportunity-table tbody tr[id]");
  await expect(scanRows).toHaveCount(1);
  await page.getByRole("group", { name: "按可执行性筛选", exact: true })
    .getByRole("button", { name: /可预检/ }).click();
  await expect(scanRows).toHaveCount(1);
  await expect(scanRows.first().getByRole("button", { name: "构建对冲", exact: true })).toBeEnabled();
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("late search snapshots cannot resurrect removed candidates or pollute later pages", async ({ page }) => {
  const f = await setup(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/#futures");
  const rows = page.locator(".futures-data-row");
  await expect(rows).toHaveCount(2);
  await rows.first().getByRole("button", { name: "查看数据依据", exact: true }).click();
  await expect(page.locator(".futures-evidence-panel")).toHaveCount(1);
  f.holdSearch("BTC");
  const pending = page.waitForResponse((response) => new URL(response.url()).searchParams.get("symbol") === "BTC");
  await page.getByPlaceholder("BTC / BINANCE", { exact: true }).fill("BTC");
  await expect.poll(() => f.searches.includes("BTC")).toBe(true);
  const removed = f.rows.splice(f.rows.findIndex((row) => row.id === "fixture-perp_cross-BTC"), 1)[0];
  f.tick();
  f.releaseSearch();
  await (await pending).finished();
  await expect(rows).toHaveCount(0);
  await expect(page.locator(".futures-evidence-panel")).toHaveCount(0);
  await expect(page.locator(".futures-search-status")).toContainText("当前匹配 0 条");
  await expect(page.locator(".table-pager-bar")).toContainText("0 条");
  await expect(page.locator(".futures-kpis .large-kpi").first().locator("strong")).toHaveText("0");
  f.rows.push(removed);
  f.tick();
  await expect(rows).toHaveCount(1);
  await expect(rows).toContainText("60000.5");
  await expect(rows.getByRole("button", { name: "构建新双腿", exact: true })).toBeEnabled();
  const olderSearchCount = f.searches.length;
  f.tick();
  await expect(rows).toContainText("60001");
  expect(f.searches).toHaveLength(olderSearchCount);

  // A separately bound search page must never become the live first page.
  await page.getByPlaceholder("BTC / BINANCE", { exact: true }).clear();
  f.transformSearch((response) => {
    response.page = { ...response.page, totalRows: 2, returnedCount: 1,
      hasNextPage: true, nextCursor: "page-2", lastCursor: "page-2" };
  });
  await page.getByPlaceholder("BTC / BINANCE", { exact: true }).fill("BTC");
  await expect(page.getByRole("button", { name: "下一页", exact: true })).toBeEnabled();
  f.transformSearch((response) => {
    response.rows[0].id = "fixture-search-page-2";
    response.rows[0].longLeg.price = 62000;
    response.page = { ...response.page, startOffset: 1, totalRows: 2, returnedCount: 1,
      hasNextPage: false, nextCursor: null, previousCursor: null, lastCursor: null };
  });
  await page.getByRole("button", { name: "下一页", exact: true }).click();
  await expect(rows).toHaveCount(1);
  await expect(rows).toContainText("62000");
  f.tick();
  await expect(rows).toHaveCount(1);
  await expect(rows).toContainText("62000");
  await expect(page.locator(".futures-search-status")).toContainText("WS 报价 0 条 · 搜索快照 1 条");
  expect(f.listRequests.some((query) => new URLSearchParams(query).get("cursor") === "page-2")).toBe(true);
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});
