import { expect, test } from "@playwright/test";
import { setup, strategies, NOW } from "./fixtures/opportunity-workbench";

test("five strategies apply confirmed empty windows immediately and recover without remounting live rows", async ({ page }) => {
  const f = await setup(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/#futures");
  const rows = page.locator(".futures-data-row");
  for (const [kind, label] of strategies) {
    await page.getByRole("tab", { name: label, exact: true }).click();
    await expect(rows).toHaveCount(2);
    await rows.first().getByRole("button", { name: "查看数据依据", exact: true }).click();
    const collapse = rows.first().getByRole("button", { name: "收起数据依据", exact: true });
    await collapse.focus();
    f.tick();
    await expect(collapse).toBeFocused();
    await expect(page.locator(".futures-evidence-panel")).toHaveCount(1);

    const removed = f.rows.filter((row) => row.strategyKind === kind);
    for (const row of removed) f.rows.splice(f.rows.indexOf(row), 1);
    f.tick(); // One authoritative empty frame, with no later frame to clear it.
    await expect(rows).toHaveCount(0);
    await expect(page.getByRole("button", { name: "构建新双腿", exact: true })).toHaveCount(0);
    await expect(page.locator(".futures-evidence-panel")).toHaveCount(0);
    await expect(page.getByRole("table", { name: "期货套利候选" })).toContainText(`${label}当前暂无候选`);
    if (kind === "perp_cross") {
      await page.screenshot({ path: test.info().outputPath("futures-confirmed-empty-1440.png"), fullPage: true });
    }
    f.rows.push(...removed);
    f.tick();
    await expect(rows).toHaveCount(2);
    await page.getByRole("button", { name: "构建新双腿", exact: true }).click();
    await expect(page.locator(".execution-ticket h3")).toHaveText(`BTC · ${label}`);
    await page.goto("/#futures");
  }
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("empty degraded refreshes retain the original quote age and disable both opportunity consumers", async ({ page }) => {
  await page.clock.install({ time: NOW });
  const f = await setup(page);
  for (const module of ["futures", "opportunities"]) {
    await page.goto(`/#${module}`);
    const isFutures = module === "futures";
    const rows = page.locator(isFutures ? ".futures-data-row" : ".opportunity-table tbody tr[id]");
    const build = rows.first().getByRole("button", { name: isFutures ? "构建新双腿" : "构建对冲", exact: true });
    const count = isFutures ? 2 : 10;
    await expect(rows).toHaveCount(count);
    await expect(build).toBeEnabled();
    const retainedPrice = f.rows.find((row) => row.id === "fixture-perp_cross-BTC").longLeg.price;
    const removed = f.rows.splice(0);
    const requests = f.listRequests.length;
    f.partial(true);
    await expect(rows).toHaveCount(count);
    await expect(build).toBeDisabled();
    await expect(page.locator(isFutures ? ".futures-kpis" : ".scan-kpis")).toContainText("保留上次报价");
    if (isFutures) {
      await expect(rows.first().locator(".leg-market-evidence").first()).toHaveText("上次报价");
      await rows.first().getByRole("button", { name: "查看数据依据", exact: true }).click();
      await expect(page.locator(".futures-evidence-decision")).toContainText("当前报价不可用");
      await expect(page.locator(".futures-evidence-decision")).not.toContainText("可检查交易");
      await expect(page.locator(".futures-evidence-panel")).toContainText("上次做多腿行情");
    }
    for (let i = 0; i < 3; i++) {
      await page.clock.fastForward(6_000);
      f.tick();
    }
    await expect(rows.first()).toContainText(String(retainedPrice));
    await expect(build).toBeDisabled();
    await expect(page.locator(".futures-feed-status summary")).toContainText(/候选 1[789]\.\ds 前/);
    expect(f.listRequests).toHaveLength(requests);
    await page.setViewportSize({ width: 390, height: 844 });
    await build.scrollIntoViewIfNeeded();
    await expect(build).toBeInViewport();
    if (isFutures) {
      expect(await rows.first().locator(".leg-market-line > span").first()
        .evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
      await expect(rows.first().locator(".futures-net-cell")).toHaveClass(/muted/);
      await expect(rows.first()).toContainText("上次测算边际");
    }
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    await page.screenshot({ path: test.info().outputPath(`${module}-retained-390.png`), fullPage: true });
    await page.setViewportSize({ width: 1440, height: 900 });

    f.rows.push(...removed);
    f.tick(); // Valid rows in a partial snapshot remain usable, unlike retained rows.
    await expect(build).toBeEnabled();
    if (isFutures) {
      await expect(page.locator(".futures-evidence-decision")).toContainText("可检查交易");
      await expect(rows.first().locator(".leg-market-evidence").first()).not.toHaveText("上次报价");
    }
    await expect(page.locator(isFutures ? ".futures-kpis" : ".scan-kpis")).not.toContainText("保留上次报价");
    f.partial(false);
    await build.click();
    await expect(page.locator(".execution-ticket h3")).toHaveText("BTC · 永续跨所");
  }
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});
