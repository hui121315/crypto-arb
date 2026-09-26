import { test, expect } from "@playwright/test";

const API = "http://127.0.0.1:18000", WEB = "http://127.0.0.1:18080";

test("paper BP failure shows partial quotes, backs off complete failures and recovers automatically", async ({ page, request }) => {
  test.setTimeout(80_000);
  const headers = { Authorization: "Bearer isolated-paper-browser" };
  const errors: string[] = [], unexpected: string[] = [];
  const read = async (path: string) => {
    const response = await request.get(API + path, { headers });
    expect(response.ok()).toBe(true);
    return response.json();
  };
  const state = async () => (await read("/api/stocks/peer/plans")).batch;
  const stats = () => read("/__paper/stocks");
  const failure = async (mode: string) => {
    const response = await request.post(API + "/__paper/stocks/quote-failure", {
      headers: { ...headers, "Content-Type": "application/json" }, data: JSON.stringify(mode),
    });
    expect(response.status()).toBe(204);
  };
  const completed = async (round: number, timeout = 20_000) => {
    let result: any;
    await expect.poll(async () => {
      result = await state();
      return !result.running && result.completedRounds === round;
    }, { timeout, intervals: [200] }).toBe(true);
    return result;
  };
  expect((await request.post(API + "/__paper/stocks/quote-failure", {
    headers: { "Content-Type": "application/json" }, data: JSON.stringify("all"),
  })).status()).toBe(401);
  await failure("one_sell");
  await page.addInitScript((api) => {
    localStorage.setItem("api_base", JSON.stringify(api));
    localStorage.setItem("api_auth_token", JSON.stringify("isolated-paper-browser"));
  }, API);
  page.on("pageerror", error => errors.push(error.message));
  await page.route("**/*", route => {
    const req = route.request(), url = new URL(req.url());
    if (![API, WEB].includes(url.origin) || (!["GET", "HEAD"].includes(req.method())
      && !["/api/auth/ws-ticket", "/api/stocks/batch"].includes(url.pathname))) {
      unexpected.push(req.method() + " " + url.pathname);
      return route.abort();
    }
    return route.continue();
  });
  await page.goto("/#stocks");
  const panel = page.getByRole("region", { name: "批量链上监控" });
  await expect(panel).toContainText("已选 2 / 32");
  await panel.getByLabel("批量更新间隔").selectOption("5");
  await panel.getByRole("button", { name: "开始批量轮询", exact: true }).click();
  const first = await completed(1);
  expect(first.problem).toContain("1/2");
  expect(first.nextAtMs - Date.now()).toBeGreaterThan(3_000);
  expect(first.nextAtMs - Date.now()).toBeLessThanOrEqual(5_000);
  const mu = panel.getByRole("row").filter({ hasText: "Micron Technology" });
  const sndk = panel.getByRole("row").filter({ hasText: "Sandisk Corporation" });
  await expect(panel.locator(".stock-batch-state")).toContainText("等待重试");
  await expect(panel.locator(".stock-batch-state")).toHaveClass(/is-warning/);
  await expect(panel.locator(".stock-problem")).toContainText("1/2");
  await expect(mu.locator("td").nth(1)).toHaveText("50");
  await expect(mu.locator("td").nth(2)).toHaveText("—");
  await expect(mu).toContainText("缺链卖报价");
  await mu.locator(".stock-batch-issue summary").click();
  await expect(mu).toContainText("503");
  await expect(sndk.locator("td").nth(1)).toHaveText("50");
  await expect(sndk.locator("td").nth(2)).toHaveText("48");
  const initial = await stats();
  expect(initial.quotes).toHaveLength(4);
  await page.screenshot({ path: test.info().outputPath("bp-partial-quotes.png"), fullPage: true });

  await failure("all");
  const second = await completed(2);
  expect(second.problem).toContain("未取得完整双向");
  expect(second.rows.every((r: any) => r.buy === null && r.sell === null && r.problem.includes("503"))).toBe(true);
  expect(second.nextAtMs - Date.now()).toBeGreaterThan(3_000);
  expect(second.nextAtMs - Date.now()).toBeLessThanOrEqual(5_000);
  const third = await completed(3);
  expect(third.problem).toContain("未取得完整双向");
  expect(third.nextAtMs - Date.now()).toBeGreaterThan(8_000);
  expect(third.nextAtMs - Date.now()).toBeLessThanOrEqual(10_000);
  expect((await stats()).quotes).toHaveLength(8);
  await expect(panel.locator(".stock-batch-state")).toContainText("等待重试");
  await expect(mu.locator("td").nth(1)).toHaveText("—");
  await page.setViewportSize({ width: 390, height: 844 });
  await panel.scrollIntoViewIfNeeded();
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(390);
  await page.evaluate(() => window.scrollTo({ top: 0, behavior: "instant" }));
  await expect.poll(() => page.evaluate(() => window.scrollY)).toBe(0);
  await page.screenshot({ path: test.info().outputPath("bp-source-failure-mobile.png"), fullPage: true });

  // No additional browser apply/resume request: the real worker recovers on its saved schedule.
  await failure("none");
  const recovered = await completed(4, 25_000);
  expect(recovered.problem).toBeNull();
  expect(recovered.rows.every((r: any) => r.buy !== null && r.sell !== null && r.problem === null)).toBe(true);
  expect(recovered.nextAtMs - Date.now()).toBeGreaterThan(3_000);
  expect(recovered.nextAtMs - Date.now()).toBeLessThanOrEqual(5_000);
  await expect(panel.locator(".stock-batch-state")).toHaveText("监控中 · 4 轮");
  await expect(panel.locator(".stock-batch-state")).not.toHaveClass(/is-warning/);
  await expect(panel.locator(".stock-problem")).toHaveCount(0);
  await expect(mu.locator("td").nth(1)).toHaveText("50");
  await expect(mu.locator("td").nth(2)).toHaveText("48");
  await panel.getByRole("button", { name: "暂停", exact: true }).click();
  await expect(panel.locator(".stock-batch-state")).toHaveText("已暂停");
  await expect(panel.locator(".stock-batch-state")).toHaveClass(/is-muted/);
  await expect.poll(async () => (await stats()).activeWs).toBe(0);
  const final = await stats();
  expect(final.quotes).toHaveLength(12);
  expect(final.quoteTimesMs[8]).toBeGreaterThanOrEqual(third.nextAtMs - 50);
  expect(final.quoteTimesMs.slice(1).every((at: number, i: number) => at - final.quoteTimesMs[i] >= 1_950)).toBe(true);
  expect(final.rpcBatches).toHaveLength(4);
  expect(final.metadataReads).toBe(initial.metadataReads);
  expect(final.maxWs).toBe(1); expect(final.unexpected).toBe(0);
  expect(errors).toEqual([]); expect(unexpected).toEqual([]);
});
