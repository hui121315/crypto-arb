import { expect, test } from "@playwright/test";

test("partial cancellation and retry are one settled trade with both costs preserved", async ({ page, request }, info) => {
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
    if (![api, "http://127.0.0.1:18080"].includes(url.origin)
      || (!["GET", "HEAD"].includes(req.method()) && url.pathname !== "/api/auth/ws-ticket")) {
      writes.push(req.url()); return route.abort();
    }
    return route.continue();
  });
  const response = await request.get(`${api}/api/review/executed?days=1&limit=1`, { headers });
  expect(response.ok()).toBe(true);
  const row = (await response.json()).rows[0];
  expect(row.id).toBe("hot-hedge");
  expect(row.closedAtMs).toBeGreaterThan(Date.now() - 60_000);
  expect(row.grossPnlUsd).toBeCloseTo(0, 8);
  expect(row.feeUsd).toBeCloseTo(0.04, 8);
  expect(row.netPnlUsd).toBeCloseTo(-0.04, 8);
  expect(row.missingFields).not.toContain("net");
  expect(row.evidence.closeRunEvidence.map((run: any) => run.closeRunId).sort()).toEqual(["partial-hot", "retry-hot"]);
  await page.goto("/#review");
  const trade = page.locator('[data-trade-id="hot-hedge"]');
  await expect(trade.locator("td").nth(2)).toContainText("平仓");
  await expect(trade.locator("td").nth(6)).toHaveText("-$0.04估算");
  await trade.getByRole("button", { name: "查看", exact: true }).click();
  const details = page.locator("#review-selected-trade-detail");
  await expect(details).toContainText("平仓 2 次");
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await trade.scrollIntoViewIfNeeded();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    await page.screenshot({ path: info.outputPath(`partial-close-review-${width}.png`) });
  }
  await page.reload();
  await expect(trade.locator("td").nth(6)).toHaveText("-$0.04估算");
  await page.getByRole("tab", { name: /策略表现/ }).click();
  await expect(page.locator(".review-strategy-table")).toContainText("估算 -$0.04");
  expect(errors).toEqual([]); expect(writes).toEqual([]);
});
