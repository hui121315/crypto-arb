import { expect, test } from "@playwright/test";
import { setup, strategies, NOW } from "./fixtures/opportunity-workbench";

test("paged and searched snapshots age independently of live WS and refresh in place", async ({ page }) => {
  await page.clock.install({ time: NOW });
  const f = await setup(page, true);
  f.paginateList();
  await page.goto("/#futures");
  const build = page.getByRole("button", { name: "构建新双腿", exact: true });
  const rows = page.locator(".futures-data-row");
  await page.getByRole("button", { name: "下一页", exact: true }).click();
  await expect(rows).toContainText("62000");
  await expect(build).toBeEnabled();
  const beforeIdle = f.listRequests.length;
  await page.clock.fastForward(31_000);
  f.tick();
  await expect(build).toBeDisabled();
  await expect(page.locator(".futures-page-status")).toContainText("本页快照已过期");
  await expect(rows).toContainText("62000");
  expect(f.listRequests).toHaveLength(beforeIdle);
  await page.setViewportSize({ width: 390, height: 844 });
  const refresh = page.getByRole("button", { name: "刷新当前页", exact: true });
  await refresh.scrollIntoViewIfNeeded();
  await expect(refresh).toBeInViewport();
  expect(await page.locator(".futures-toolbar").evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
  await page.screenshot({ path: test.info().outputPath("futures-page-expired-390.png"), fullPage: true });
  await refresh.click();
  await expect(build).toBeEnabled();
  expect(new URLSearchParams(f.listRequests.at(-1)).get("cursor")).toBe("page-2");
  await expect(rows).toContainText("62000");
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.getByPlaceholder("BTC / BINANCE", { exact: true }).fill("BTC");
  await expect(page.locator(".futures-search-status").last()).toContainText("BTC · 搜索快照");
  await page.getByRole("button", { name: "下一页", exact: true }).click();
  await expect(rows).toContainText("62000");
  await expect(build).toBeEnabled();
  const beforeSearchIdle = f.listRequests.length;
  await page.clock.fastForward(31_000);
  f.tick();
  await expect(build).toBeDisabled();
  await expect(page.locator(".futures-search-status")).toContainText("BTC · 搜索快照已过期");
  expect(f.listRequests).toHaveLength(beforeSearchIdle);
  await page.getByRole("button", { name: "重新搜索", exact: true }).click();
  await expect(build).toBeEnabled();
  expect(new URLSearchParams(f.listRequests.at(-1)).get("cursor")).toBe("page-2");
  await expect(rows).toContainText("62000");
  await page.getByRole("button", { name: "首页", exact: true }).click();
  await expect(rows).not.toContainText("62000");
  await page.getByPlaceholder("BTC / BINANCE", { exact: true }).clear();
  f.tick();
  await expect(build).toBeEnabled();
  await expect(page.locator(".futures-feed-status summary")).toContainText("候选实时更新");
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("five strategy screens keep evidence expansion and focus across live ticks", async ({ page }) => {
  const f = await setup(page);
  await page.goto("/#futures");
  const table = page.getByRole("table", { name: "期货套利候选", exact: true });
  await expect(table.locator("tr.futures-data-row")).toHaveCount(2);
  for (const [, label] of strategies) {
    await page.getByRole("tab", { name: label, exact: true }).click();
    await expect(table.locator("tr.futures-data-row")).toHaveCount(2);
  }
  await page.getByRole("tab", { name: "永续跨所", exact: true }).click();
  await table.getByRole("button", { name: "查看数据依据", exact: true }).first().click();
  const full = table.locator(".futures-evidence-more");
  await full.locator("summary").click();
  const collapse = table.getByRole("button", { name: "收起数据依据", exact: true });
  await collapse.focus();
  for (const width of [1440, 1024, 390, 320]) {
    await page.setViewportSize({ width, height: 900 });
    const tabs = page.getByRole("tablist", { name: "套利策略", exact: true });
    expect(await tabs.evaluate(el => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
    for (const [, label] of strategies) {
      const tab = page.getByRole("tab", { name: label, exact: true });
      const bounds = await tab.boundingBox();
      expect(bounds!.x).toBeGreaterThanOrEqual(0);
      expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(width);
      await tab.click({ trial: true });
    }
    const panel = table.locator(".futures-evidence-panel");
    const bounds = await panel.boundingBox();
    expect(bounds?.x).toBeGreaterThanOrEqual(0);
    expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(width);
    const wrap = table.locator("..");
    const visibleRight = await wrap.evaluate((el) => el.getBoundingClientRect().left + el.clientWidth);
    const close = await panel.getByRole("button", { name: "收起", exact: true }).boundingBox();
    expect(close!.x + close!.width).toBeLessThanOrEqual(visibleRight);
    const action = table.locator(".futures-data-row.is-selected .futures-cell-action");
    const background = await action.evaluate((el) => getComputedStyle(el).backgroundColor);
    expect(background).not.toMatch(/rgba\([^)]*,\s*0\./);
    await page.screenshot({ path: test.info().outputPath(`futures-evidence-${width}.png`), fullPage: true });
  }
  await page.setViewportSize({ width: 1440, height: 900 });
  await collapse.focus();
  await expect.poll(() => f.sockets.size).toBeGreaterThan(0);
  f.tick();
  await expect(table.locator(".futures-data-row").first()).toContainText("60000.5");
  await expect(full).toHaveAttribute("open", "");
  await expect(collapse).toBeFocused();
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("five mobile strategies show both legs, costs and actions without horizontal scrolling", async ({ page }) => {
  const f = await setup(page);
  await page.goto("/#futures");
  const rows = page.locator(".futures-data-row");
  const first = rows.first();
  for (const width of [720, 390, 320]) {
    await page.setViewportSize({ width, height: 900 });
    for (const [kind, label] of strategies) {
      await page.getByRole("tab", { name: label, exact: true }).click();
      await expect(rows).toHaveCount(2);
      await expect(first.locator(".futures-long-cell")).toContainText(kind.includes("spot") ? "买入现货" : "做多永续");
      await expect(first.locator(".futures-short-cell")).toContainText(kind === "spot_cross" ? "卖出现货" : "做空永续");
      await expect(first.locator(".futures-short-cell")).toContainText("60001");
      await expect(first.locator(".futures-net-cell")).toContainText("费后净边际");
      await expect(first.locator(".futures-cost-cell")).toContainText("完整成本");
      await expect(first.locator(".futures-condition-cell")).toContainText("兑现条件");
      expect(await page.locator(".paged-table-wrap").evaluate(el => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
      expect(await first.locator("td").evaluateAll(cells => {
        const boxes = cells.map(el => el.getBoundingClientRect());
        return cells.every((el, i) => {
          const a = boxes[i];
          return a.width > 0 && a.left >= 0 && a.right <= innerWidth && el.scrollWidth <= el.clientWidth + 1
            && boxes.every((b, j) => i === j || a.right <= b.left + 1 || a.left >= b.right - 1 || a.bottom <= b.top + 1 || a.top >= b.bottom - 1);
        });
      })).toBe(true);
      const build = first.getByRole("button", { name: "构建新双腿", exact: true });
      await build.scrollIntoViewIfNeeded();
      await build.click({ trial: true });
      if (kind === "perp_cross" || (kind === "spot_cross" && width === 320)) {
        await page.screenshot({ path: test.info().outputPath(`futures-${kind}-${width}.png`), fullPage: true });
      }
    }
  }
  const evidence = first.getByRole("button", { name: "查看数据依据", exact: true });
  await evidence.click();
  const collapse = first.getByRole("button", { name: "收起数据依据", exact: true });
  await collapse.focus();
  f.stale(true);
  await expect(first.getByRole("button", { name: "构建新双腿", exact: true })).toBeDisabled();
  await expect(first.locator(".futures-net-cell")).toContainText("上次测算边际");
  await expect(first.locator(".futures-net-cell")).toHaveClass(/muted/);
  await expect(first.locator(".leg-market-evidence")).toHaveText(["上次报价", "上次报价"]);
  await expect(collapse).toBeFocused();
  f.stale(false);
  f.tick();
  await expect(first.locator(".futures-long-cell")).toContainText("60000.5");
  await expect(page.locator(".futures-evidence-panel")).toHaveCount(1);
  await expect(collapse).toBeFocused();
  await first.getByRole("button", { name: "构建新双腿", exact: true }).click();
  await expect(page.locator(".execution-ticket h3")).toHaveText("BTC · 现货跨所");
  await page.goto("/#futures");
  await page.getByPlaceholder("BTC / BINANCE", { exact: true }).fill("kraken");
  await expect(rows).toHaveCount(0);
  await expect(page.locator(".futures-empty-message")).toBeVisible();
  expect(await page.locator(".paged-table-wrap").evaluate(el => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("late symbol results cannot roll back a newer query or venue filter", async ({ page }) => {
  const f = await setup(page);
  await page.goto("/#futures");
  const search = page.getByPlaceholder("BTC / BINANCE", { exact: true });
  const rows = page.locator(".futures-data-row");
  await expect(rows).toHaveCount(2);
  f.holdSearch("BTCUSDT");
  const oldResponse = page.waitForResponse((response) => new URL(response.url()).searchParams.get("symbol") === "BTCUSDT");
  await search.fill("BTCUSDT");
  await expect.poll(() => f.searches.includes("BTCUSDT")).toBe(true);
  await search.fill("ETH");
  await expect(page.locator(".futures-search-status")).toContainText("ETH · 搜索快照");
  await expect(rows).toHaveCount(1);
  await expect(rows.first()).toContainText("ETH");
  f.releaseSearch();
  await (await oldResponse).finished();
  await expect(rows.first()).not.toContainText("BTC");
  await search.fill("kraken");
  await expect(rows).toHaveCount(0);
  await expect(page.locator(".futures-search-status")).toHaveCount(0);
  expect(f.searches).not.toContain("KRAKEN");
  await search.fill("BTCUSDT");
  await expect(rows).toHaveCount(1);
  await expect(rows.first()).toContainText("BTC");
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("stale stream preserves quotes but disables building until recovery", async ({ page }) => {
  const f = await setup(page);
  await page.goto("/#futures");
  const build = page.getByRole("button", { name: "构建新双腿", exact: true });
  await expect(build).toBeEnabled();
  f.stale(true);
  await expect(build).toBeDisabled();
  await expect(page.locator(".futures-data-row")).toHaveCount(2);
  f.stale(false);
  await expect(build).toBeEnabled();
  f.partial(true);
  await expect(page.locator(".futures-feed-status summary")).toContainText("降级");
  await expect(build).toBeEnabled();
  f.partial(false);
  await build.click();
  await expect(page.locator(".execution-page")).toBeVisible();
  await expect(page.locator(".execution-ticket")).toContainText("BTC");
  await expect(page.locator(".execution-ticket")).toContainText("期货套利");
  for (const [, label] of strategies.slice(1)) {
    await page.goto("/#futures");
    await page.getByRole("tab", { name: label, exact: true }).click();
    await page.getByRole("button", { name: "构建新双腿", exact: true }).click();
    await expect(page.locator(".execution-ticket h3")).toHaveText(`BTC · ${label}`);
  }
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("a failed symbol search is visible and cannot offer an earlier symbol as current", async ({ page }) => {
  const f = await setup(page);
  await page.goto("/#futures");
  const search = page.getByPlaceholder("BTC / BINANCE", { exact: true });
  const table = page.getByRole("table", { name: "期货套利候选", exact: true });
  await expect(table.locator("tr.futures-data-row")).toHaveCount(2);
  await search.fill("BTC");
  await expect(table.locator("tr.futures-data-row")).toHaveCount(1);
  f.failSearch("SOL");
  await search.fill("SOL");
  await expect(page.locator(".futures-search-status")).toContainText("SOL · 搜索失败");
  await expect(table.locator("tr.futures-data-row")).toHaveCount(0);
  await page.screenshot({ path: test.info().outputPath("futures-search-failure.png"), fullPage: true });
  f.failSearch();
  await page.getByRole("button", { name: "重新搜索", exact: true }).click();
  await expect(page.locator(".futures-search-status")).toContainText("SOL · 搜索快照");
  await expect(table.locator("tr.futures-data-row")).toHaveCount(0);
  await search.clear();
  await expect(table.locator("tr.futures-data-row")).toHaveCount(2);
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("symbol pagination never inserts live first-page rows into a later page", async ({ page }) => {
  const f = await setup(page, true);
  await page.goto("/#futures");
  await page.getByPlaceholder("BTC / BINANCE", { exact: true }).fill("BTC");
  const rows = page.locator(".futures-data-row");
  await expect(rows).toHaveCount(1);
  await page.getByRole("button", { name: "查看数据依据", exact: true }).click();
  await page.getByRole("button", { name: "下一页", exact: true }).click();
  await expect(rows).toHaveCount(1);
  await expect(rows).toContainText("62000");
  await expect(page.locator(".futures-evidence-panel")).toHaveCount(0);
  f.tick();
  await expect(rows).toHaveCount(1);
  await expect(rows).toContainText("62000");
  await page.getByRole("button", { name: "首页", exact: true }).click();
  await expect(rows).toHaveCount(1);
  await expect(rows).toContainText("60000.5");
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});
