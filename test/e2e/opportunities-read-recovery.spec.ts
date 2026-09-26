import { expect, test } from "@playwright/test";
import { API, NOW, setup } from "./fixtures/opportunity-workbench";

for (const kind of ["search", "page"] as const) {
  test(`scanner ${kind} timeout cancels its read and permits an exact retry`, async ({ page }) => {
    await page.clock.install({ time: NOW });
    const f = await setup(page);
    if (kind === "page") f.paginateList();
    await page.goto("/#opportunities");
    await page.getByRole("tab", { name: "永续跨所", exact: true }).click();
    const rows = page.locator(".opportunity-table tbody tr[id]");
    const builds = page.getByRole("button", { name: "构建对冲", exact: true });
    await expect(rows.first()).toContainText("60000");

    const initial = await (await page.request.get(`${API}/api/v3/arbitrage/opportunities/list`)).json();
    initial.rows[0].longLeg.price = 77777;
    let hold = true;
    let release: (() => void) | undefined;
    let attempts = 0;
    let cancelled = 0;
    let lateReplyReleased = false;
    const matches = (url: URL) => kind === "search"
      ? url.searchParams.get("symbol") === "BTC"
      : url.searchParams.get("cursor") === "page-2";
    page.on("requestfailed", request => {
      const url = new URL(request.url());
      if (url.pathname === "/api/v3/arbitrage/opportunities/list" && matches(url)) cancelled++;
    });
    await page.route("**/api/v3/arbitrage/opportunities/list?**", async route => {
      if (!matches(new URL(route.request().url()))) return route.fallback();
      attempts++;
      if (!hold) return route.fallback();
      await new Promise<void>(resolve => { release = resolve; });
      await route.fulfill({ json: initial });
      lateReplyReleased = true;
    });
    if (kind === "search") {
      await page.getByPlaceholder("币种 / 交易所 / 路由", { exact: true }).fill("BTC");
      await page.clock.fastForward(300);
    } else {
      await page.getByRole("button", { name: "下一页", exact: true }).click();
    }
    await expect.poll(() => attempts).toBe(1);
    const status = page.locator(kind === "search"
      ? ".futures-search-status:not(.opportunity-stream-recovery):not(.opportunity-page-status)"
      : ".opportunity-page-status");
    await expect(status).toContainText(kind === "search" ? "搜索中" : "快照刷新中");
    if (kind === "page") {
      await expect(page.locator(".futures-feed-status summary")).toContainText("分页读取中");
      await expect(rows.and(page.locator('[aria-selected="true"]'))).toHaveCount(0);
    }
    await page.clock.fastForward(15_100);
    f.tick();
    await expect(status).toContainText(kind === "search" ? "搜索失败" : "读取异常");
    await expect.poll(() => cancelled).toBe(1);
    await expect(status).toHaveClass(/is-error/);
    const retry = status.getByRole("button", { name: kind === "search" ? "重新搜索" : "刷新当前页", exact: true });
    await expect(retry).toBeEnabled();
    expect(attempts).toBe(1);
    await expect(builds.and(page.locator(":enabled"))).toHaveCount(0);
    for (const width of [1440, 390]) {
      await page.setViewportSize({ width, height: 900 });
      expect(await status.evaluate(el => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
      await retry.click({ trial: true });
      await page.screenshot({ path: test.info().outputPath(`scanner-${kind}-timeout-${width}.png`) });
    }
    hold = false;
    await retry.click();
    await expect.poll(() => attempts).toBe(2);
    await expect(status).not.toContainText(kind === "search" ? "搜索失败" : "读取异常");
    await expect(rows.first()).toContainText(kind === "page" ? "62000" : "60000");
    await expect(builds).toBeEnabled();
    await expect(rows.first()).toHaveAttribute("aria-selected", "true");
    await expect(page.locator(".opportunity-route-context")).toHaveCount(0);
    const request = new URLSearchParams(f.listRequests.at(-1));
    expect(request.get(kind === "page" ? "cursor" : "symbol")).toBe(kind === "page" ? "page-2" : "BTC");
    expect(request.get("strategy")).toBe("perp_cross");
    release!();
    await expect.poll(() => lateReplyReleased).toBe(true);
    await expect(rows.first()).not.toContainText("77777");
    if (kind === "page") {
      await status.getByRole("button", { name: "返回实时首页", exact: true }).click();
      await expect(status).toHaveCount(0);
      await expect(rows.first()).not.toContainText("62000");
      hold = true;
      lateReplyReleased = false;
      await page.getByRole("button", { name: "下一页", exact: true }).click();
      await expect.poll(() => attempts).toBe(3);
      await status.getByRole("button", { name: "返回实时首页", exact: true }).click();
      await expect.poll(() => cancelled).toBe(2);
      await expect(status).toHaveCount(0);
      release!();
      await expect.poll(() => lateReplyReleased).toBe(true);
      await expect(rows.first()).not.toContainText("77777");
      await expect(builds).toBeEnabled();
      expect(attempts).toBe(3);
    }
    expect(f.errors).toEqual([]);
    expect(f.writes).toEqual([]);
  });
}
