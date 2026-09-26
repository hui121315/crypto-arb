import { expect, test } from "@playwright/test";

test("review includes old openings closed today and keeps page time and totals consistent", async ({ page, request }, info) => {
  const api = "http://127.0.0.1:18000";
  const headers = { Authorization: "Bearer isolated-paper-browser" };
  const errors: string[] = [], writes: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.addInitScript(api => {
    localStorage.setItem("api_base", JSON.stringify(api));
    localStorage.setItem("api_auth_token", JSON.stringify("isolated-paper-browser"));
  }, api);
  await page.route("**/*", route => {
    const req = route.request(), url = new URL(req.url());
    if (![api, "http://127.0.0.1:18080"].includes(url.origin)) {
      writes.push(req.url()); return route.abort();
    }
    if (!["GET", "HEAD"].includes(req.method()) && url.pathname !== "/api/auth/ws-ticket") {
      writes.push(`${req.method()} ${url.pathname}`); return route.abort();
    }
    return route.continue();
  });
  const response = await request.get(`${api}/api/review/executed?days=1&limit=1`, { headers });
  expect(response.ok()).toBe(true);
  const result = await response.json();
  expect(result.rows[0].id).toBe("hot-hedge");
  expect(result.rows[0].openedAtMs).toBeLessThan(Date.now() - 39 * 86400_000);
  expect(result.rows[0].closedAtMs).toBeGreaterThan(Date.now() - 60_000);
  expect(result.rows[0].netPnlUsd).toBeCloseTo(-0.04, 8);
  expect(result.page.totalRows).toBe(2);
  const next = await (await request.get(`${api}/api/review/executed?days=1&limit=1&cursor=${encodeURIComponent(result.page.nextCursor)}`, { headers })).json();
  expect(next.rows.map((row: any) => row.id)).toEqual(["recent-hedge"]);
  await page.goto("/#review");
  const trade = page.locator('[data-trade-id="hot-hedge"]');
  await expect(trade).toBeVisible();
  await expect(page.locator(".review-executed-table tbody tr").first()).toHaveAttribute("data-trade-id", "hot-hedge");
  await expect(trade.locator("td").nth(2)).toContainText("平仓");
  await expect(trade.locator("td").nth(6)).toHaveText("-$0.04估算");
  await expect(page.locator('[data-trade-id="unrelated-hedge"]')).toHaveCount(0);
  await expect(page.locator(".review-executed-summary")).toContainText("估算 -$0.04");
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await trade.scrollIntoViewIfNeeded();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    await page.screenshot({ path: info.outputPath(`review-history-${width}.png`) });
  }
  await page.reload();
  await expect(trade).toBeVisible();
  await page.getByRole("tab", { name: /策略表现/ }).click();
  await expect(page.locator(".review-strategy-table")).toContainText("估算 -$0.04");
  expect(errors).toEqual([]); expect(writes).toEqual([]);
});
