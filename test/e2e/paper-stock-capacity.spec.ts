import { test, expect } from "@playwright/test";

const API = "http://127.0.0.1:18000", WEB = "http://127.0.0.1:18080";

test("paper BP full batch selects across pages, completes 32 stocks and cancels its next queue", async ({ page, request }) => {
  test.skip(process.env.CROSSLINE_E2E_STOCK_CAPACITY !== "1", "Requires the isolated 44-security catalog");
  test.setTimeout(180_000);
  const headers = { Authorization: "Bearer isolated-paper-browser" };
  const read = async (path: string) => {
    const response = await request.get(API + path, { headers });
    expect(response.ok()).toBe(true);
    return response.json();
  };
  const state = async () => (await read("/api/stocks/peer/plans")).batch;
  const stats = () => read("/__paper/stocks");
  const errors: string[] = [], unexpected: string[] = [], writes: any[] = [];
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
    if (url.pathname === "/api/stocks/batch" && req.method() === "POST") writes.push(req.postDataJSON());
    return route.continue();
  });
  await page.goto("/#stocks");
  await page.getByRole("button", { name: "选择股票", exact: true }).click();
  const catalog = page.getByRole("complementary", { name: "Backpack 股票目录" });
  const panel = page.getByRole("region", { name: "批量链上监控" });
  await expect(catalog).toContainText("44 个证券");
  await expect(catalog).toContainText("监控选择 2 / 32");
  await catalog.getByRole("button", { name: "下一页", exact: true }).click();
  await catalog.getByRole("button", { name: "加入本页 4", exact: true }).click();
  await expect(catalog).toContainText("监控选择 6 / 32");
  await catalog.getByRole("button", { name: "上一页", exact: true }).click();
  await catalog.getByRole("button", { name: "加入本页 26", exact: true }).click();
  await expect(catalog).toContainText("监控选择 32 / 32");
  await expect(catalog.getByRole("button", { name: "加入本页 0", exact: true })).toBeDisabled();
  await catalog.getByRole("checkbox", { name: "监控 T027", exact: true }).click();
  await expect(catalog.getByRole("checkbox", { name: "监控 T027", exact: true })).not.toBeChecked();
  await catalog.getByLabel("只看已选监控", { exact: true }).check();
  await expect(catalog.getByRole("checkbox", { name: /^监控 / })).toHaveCount(32);
  const retainedRow = await catalog.getByRole("checkbox", { name: "监控 T041", exact: true }).elementHandle();
  await catalog.getByRole("checkbox", { name: "监控 T040", exact: true }).click();
  await expect(catalog.getByRole("checkbox", { name: "监控 T040", exact: true })).toHaveCount(0);
  await expect(catalog.getByRole("checkbox", { name: /^监控 / })).toHaveCount(31);
  expect(await retainedRow!.evaluate(element => element.isConnected)).toBe(true);
  await catalog.getByLabel("只看已选监控", { exact: true }).uncheck();
  await catalog.getByRole("button", { name: "下一页", exact: true }).click();
  await catalog.getByRole("button", { name: "加入本页 1", exact: true }).click();
  await catalog.getByLabel("只看已选监控", { exact: true }).check();
  await expect(catalog.getByRole("checkbox", { name: /^监控 / })).toHaveCount(32);
  expect(writes).toHaveLength(0);
  expect((await stats()).quotes).toHaveLength(0);

  await panel.getByLabel("批量询价金额").fill("10.5");
  await panel.getByLabel("批量更新间隔").selectOption("60");
  await panel.getByRole("button", { name: "开始批量轮询", exact: true }).click();
  await expect.poll(() => writes.length).toBe(1);
  expect(new Set(writes[0].request.assets).size).toBe(32);
  await expect(panel.locator("tbody tr")).toHaveCount(32);
  // Do not accelerate the quota clock: 64 requests use the real shared keyless gate.
  await expect.poll(async () => (await state()).completedRounds, {
    timeout: 150_000, intervals: [1_000],
  }).toBe(1);
  const completed = await state(), first = await stats();
  expect(completed.problem).toBeNull();
  expect(completed.running).toBe(false);
  expect(completed.request.assets).toHaveLength(32);
  expect(completed.rows).toHaveLength(32);
  expect(completed.rows.every((r: any) => r.buy && r.sell && !r.problem && !r.refreshing)).toBe(true);
  expect(completed.rows.filter((r: any) => r.issuerVerified)).toHaveLength(2);
  expect(first.quotes).toHaveLength(64);
  expect(first.quoteTimesMs[63] - first.quoteTimesMs[0]).toBeGreaterThanOrEqual(130_000);
  expect(first.quoteTimesMs.slice(1).every((at: number, i: number) => at - first.quoteTimesMs[i] >= 1_950)).toBe(true);
  expect(first.rpcBatches.length).toBeGreaterThanOrEqual(6);
  expect(first.rpcBatches.length).toBeLessThanOrEqual(8);
  expect(first.rpcBatches.every((batch: string[]) => batch.length === 34 && new Set(batch).size === 34)).toBe(true);
  expect(first.metadataReads).toBe(2);
  expect(first.maxWs).toBe(1);
  const subscriptions = [...new Set<string>(first.wsSubscriptions.flat())].sort();
  expect(subscriptions).toEqual([...completed.request.assets.map((asset: string) => `bookTicker.${asset}_USDC`),
    "bookTicker.USDT_USDC"].sort());
  expect(first.unexpected).toBe(0);
  const coverage = panel.getByLabel("双向新鲜报价", { exact: true });
  await expect(coverage).toHaveText(/双向新鲜 [1-3]\/32/);
  await expect(panel.locator("tbody tr").first().locator("td").nth(1)).toHaveText("—");
  await expect(panel.locator("tbody tr").last().getByRole("button", { name: "查看", exact: true })).toBeEnabled();
  await page.evaluate(() => window.scrollTo({ top: 0, behavior: "instant" }));
  await expect.poll(() => page.evaluate(() => window.scrollY)).toBe(0);
  await page.screenshot({ path: test.info().outputPath("bp-full-batch-desktop.png"), fullPage: true });

  await panel.getByRole("button", { name: "暂停", exact: true }).click();
  await expect(panel.locator(".stock-batch-state")).toHaveText("已暂停");
  await expect.poll(async () => (await stats()).activeWs).toBe(0);
  expect((await request.post(API + "/__paper/stocks", { headers, data: true })).status()).toBe(204);
  await panel.getByRole("button", { name: "开始批量轮询", exact: true }).click();
  await expect.poll(async () => (await stats()).quotes.length).toBe(65);
  await panel.getByRole("button", { name: "暂停", exact: true }).click();
  await expect(panel.locator(".stock-batch-state")).toHaveText("已暂停");
  await page.waitForTimeout(700);
  expect((await request.post(API + "/__paper/stocks", { headers, data: false })).status()).toBe(204);
  await page.waitForTimeout(2_300);
  const paused = await state(), final = await stats();
  expect(final.quotes).toHaveLength(65);
  expect(final.maxWs).toBe(1);
  expect(final.activeWs).toBe(0);
  expect(final.unexpected).toBe(0);
  expect(paused.completedRounds).toBe(1);
  expect(paused.rows.every((r: any) => !r.refreshing)).toBe(true);
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(catalog.getByRole("button", { name: "清空选择", exact: true })).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(390);
  await page.evaluate(() => window.scrollTo({ top: 0, behavior: "instant" }));
  await expect.poll(() => page.evaluate(() => window.scrollY)).toBe(0);
  await page.screenshot({ path: test.info().outputPath("bp-full-batch-mobile.png"), fullPage: true });
  expect(errors).toEqual([]);
  expect(unexpected).toEqual([]);
  expect(writes).toHaveLength(4);
  await test.info().attach("bp-capacity-metrics", {
    contentType: "application/json",
    body: JSON.stringify({ catalogRows: 44, monitoredRows: completed.rows.length,
      quoteRequests: first.quotes.length, quoteSpanMs: first.quoteTimesMs[63] - first.quoteTimesMs[0],
      rpcBatches: first.rpcBatches.length, addressesPerBatch: first.rpcBatches[0].length,
      metadataReads: first.metadataReads, maxWs: final.maxWs,
      stockSubscriptions: subscriptions.filter(stream => stream !== "bookTicker.USDT_USDC").length,
      conversionSubscriptions: subscriptions.filter(stream => stream === "bookTicker.USDT_USDC").length,
      quoteRequestsAfterCancellation: final.quotes.length, completedRoundsAfterCancellation: paused.completedRounds,
      issuerVerifiedRows: completed.rows.filter((r: any) => r.issuerVerified).length,
      unexpectedRequests: final.unexpected + unexpected.length,
      browserErrors: errors.length, configurationWrites: writes.length }, null, 2),
  });
});

test("paper BP catalog bulk controls remain visible at desktop and mobile without starting quotes", async ({ page, request }) => {
  test.skip(process.env.CROSSLINE_E2E_STOCK_CAPACITY !== "1", "Requires the isolated 44-security catalog");
  const headers = { Authorization: "Bearer isolated-paper-browser" };
  const errors: string[] = [], writes: string[] = [];
  const stats = async () => (await request.get(API + "/__paper/stocks", { headers })).json();
  const before = await stats();
  await page.addInitScript((api) => {
    localStorage.setItem("api_base", JSON.stringify(api));
    localStorage.setItem("api_auth_token", JSON.stringify("isolated-paper-browser"));
  }, API);
  page.on("pageerror", error => errors.push(error.message));
  await page.route("**/*", route => {
    const req = route.request(), url = new URL(req.url());
    if (![API, WEB].includes(url.origin) || (!["GET", "HEAD"].includes(req.method()) && url.pathname !== "/api/auth/ws-ticket")) {
      writes.push(req.method() + " " + url.pathname);
      return route.abort();
    }
    return route.continue();
  });
  await page.goto("/#stocks");
  await page.getByRole("button", { name: "选择股票", exact: true }).click();
  const catalog = page.getByRole("complementary", { name: "Backpack 股票目录" });
  await expect(catalog).toContainText("44 个证券");
  await catalog.getByRole("button", { name: "清空选择", exact: true }).click();
  await catalog.getByRole("searchbox", { name: "搜索股票", exact: true }).fill("T04");
  await catalog.getByRole("button", { name: "加入本页 3", exact: true }).click();
  await catalog.getByRole("searchbox", { name: "搜索股票", exact: true }).fill("");
  await catalog.getByRole("button", { name: "加入本页 29", exact: true }).click();
  await catalog.getByLabel("只看已选监控", { exact: true }).check();
  await expect(catalog.getByRole("checkbox", { name: /^监控 / })).toHaveCount(32);
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await page.evaluate(() => window.scrollTo({ top: 0, behavior: "instant" }));
    await expect.poll(() => page.evaluate(() => window.scrollY)).toBe(0);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
    const banner = await page.getByRole("banner").boundingBox();
    const controls = await catalog.locator(".stock-catalog-actions").boundingBox();
    expect(banner!.y).toBe(0);
    expect(controls!.y).toBeGreaterThan(banner!.y + banner!.height);
    await expect(catalog.getByRole("button", { name: "清空选择", exact: true })).toBeInViewport();
    await page.screenshot({ path: test.info().outputPath(`bp-selection-${width}.png`), fullPage: true });
  }
  expect((await stats()).quotes.length).toBe(before.quotes.length);
  expect(errors).toEqual([]); expect(writes).toEqual([]);
});
