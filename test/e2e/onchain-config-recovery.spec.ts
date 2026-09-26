import { expect, test, type Page } from "@playwright/test";
import { API, WEB, NOW, setup, snapshot } from "./fixtures/onchain-workbench";

const recovery = (page: Page) => page.getByRole("alert", { name: "设置操作待核对" });
const recheck = (page: Page) => recovery(page).getByRole("button", { name: "核对上次操作", exact: true });
const amount = (page: Page) => page.getByLabel("统一对比资金 (USDC)", { exact: true });
const records = (page: Page) => page.evaluate(() => Object.entries(sessionStorage)
  .filter(([key]) => key.startsWith("crossline.settings.pending.v1:onchain-config:")));

async function capture(page: Page) {
  await page.screenshot({ path: test.info().outputPath("recovery-desktop.png") });
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(recheck(page)).toBeInViewport();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
  await page.screenshot({ path: test.info().outputPath("recovery-mobile.png") });
  await page.setViewportSize({ width: 1440, height: 900 });
}

test("real isolated backend recovers onchain configuration and batch writes without replaying mutations", async ({ page, request }) => {
  const headers = { Authorization: "Bearer isolated-paper-browser" };
  const errors: string[] = [], unexpected: string[] = [], writes: any[] = [];
  const mutating = ["/api/onchain/comparison/config", "/api/onchain/comparison/batch", "/api/onchain/comparison/batch/remove"];
  let lostPath: string | undefined = mutating[0], release = () => {};
  await page.addInitScript(api => {
    localStorage.setItem("api_base", JSON.stringify(api));
    localStorage.setItem("api_auth_token", JSON.stringify("isolated-paper-browser"));
    localStorage.setItem("crossline.settings.activeTab", JSON.stringify("action-runs"));
  }, API);
  page.on("pageerror", error => errors.push(error.message));
  await page.route("**/*", async route => {
    const req = route.request(), url = new URL(req.url()), path = url.pathname;
    if (![API, WEB].includes(url.origin) || (!["GET", "HEAD"].includes(req.method())
      && !["/api/auth/ws-ticket", ...mutating].includes(path))) {
      unexpected.push(`${req.method()} ${path}`); return route.abort();
    }
    // Catalog is local evidence for the form; configuration/receipts/storage are real Rust handlers.
    if (path === "/api/onchain/cex-pairs") return route.fulfill({ json: { venue: "binance", baseToken: "SOL", problem: null,
      pairs: [{ venue: "binance", baseToken: "SOL", quoteToken: "USDC", cexSymbol: "SOL/USDC",
        nativeSymbol: "SOLUSDC", quality: "fresh", source: "fixture", freshnessMs: 0, observedAtMs: Date.now() }] } });
    if (path === "/api/onchain/credentials") return route.fulfill({ json: { providers: [] } });
    if (req.method() !== "GET" && mutating.includes(path)) {
      const row = { path, headers: req.headers(), body: req.postDataJSON(), result: null as any };
      writes.push(row);
      const response = await route.fetch();
      expect(response.status()).toBe(200);
      row.result = await response.json();
      if (path === lostPath) {
        lostPath = undefined;
        await new Promise<void>(resolve => release = resolve);
        return route.abort().catch(() => {});
      }
      return route.fulfill({ response });
    }
    return route.continue();
  });
  const current = async () => (await request.get(`${API}/api/onchain/comparison`, { headers })).json();
  const original = await current();
  expect(original.config.enabled).toBe(false);
  await page.goto(`${WEB}/#onchain`);
  await expect(amount(page)).toHaveValue("100");
  await amount(page).fill("12.345");
  await page.getByRole("button", { name: "应用并开始监控", exact: true }).click();
  await expect.poll(() => writes[0]?.result?.config.quoteAmountRaw).toBe("12345000");
  await page.locator('.module-tabs button[data-module="futures"]').click();
  await page.locator('.module-tabs button[data-module="onchain"]').click();
  await expect(amount(page)).toBeDisabled();
  expect(writes).toHaveLength(1);
  const record = (await records(page))[0];
  expect(record).toBeDefined();
  const stored = JSON.parse(record[1]);
  expect(Object.keys(stored).sort()).toEqual(["context", "kind", "run_id", "target", "version"]);
  expect(record[1]).not.toContain("isolated-paper-browser");
  expect(record[1]).not.toContain("quoteAmountRaw");
  await page.reload(); release();
  await expect(recovery(page)).toContainText("保存链上配置结果待核对");
  await capture(page);
  // A newer independent configuration must win over the old successful receipt.
  const newer = await request.patch(`${API}/api/onchain/comparison/config`, {
    headers: { ...headers, "x-request-id": "test-current-config", "idempotency-key": "test-current-config" },
    data: { quoteAmountRaw: "20000000" },
  });
  expect(newer.ok()).toBe(true);
  await recheck(page).click();
  await expect(recovery(page)).toBeHidden();
  await expect(amount(page)).toHaveValue("20");
  await expect(amount(page)).toBeEnabled();
  const duplicate = await request.patch(`${API}${writes[0].path}`, {
    headers: { ...headers, "x-request-id": writes[0].headers["x-request-id"], "idempotency-key": writes[0].headers["idempotency-key"] },
    data: { enabled: false, quoteAmountRaw: "90000000" },
  });
  expect(duplicate.ok()).toBe(true);
  expect(await duplicate.json()).toEqual(writes[0].result);
  expect((await current()).config.quoteAmountRaw).toBe("20000000");
  await page.getByRole("button", { name: "暂停当前监控", exact: true }).click();
  await expect(page.locator(".onchain-rail-header-tools .read-only-flag")).toHaveText("已暂停");
  await page.getByRole("button", { name: "应用并开始监控", exact: true }).click();
  await expect(page.getByRole("button", { name: "暂停当前监控", exact: true })).toBeEnabled();

  lostPath = mutating[1];
  await page.getByRole("button", { name: "加入批量监控", exact: true }).click();
  await expect.poll(() => writes[3]?.result?.items.length).toBe(1);
  await page.reload(); release();
  await expect(recovery(page)).toContainText("加入批量监控结果待核对");
  await recheck(page).click();
  await expect(recovery(page)).toBeHidden();
  await page.locator("#onchain-runtime-tab-watchlist").click();
  const row = page.locator(".onchain-page tr[data-item-id]");
  await expect(row).toHaveCount(1);
  const itemId = await row.getAttribute("data-item-id");
  lostPath = mutating[2];
  await row.getByRole("button", { name: "移除", exact: true }).click();
  await expect.poll(() => writes[4]?.result?.items.length).toBe(0);
  await page.reload(); release();
  await expect(recovery(page)).toContainText("移除批量市场结果待核对");
  await recheck(page).click();
  await expect(recovery(page)).toBeHidden();
  await page.locator("#onchain-runtime-tab-watchlist").click();
  await expect(row).toHaveCount(0);
  const replayRemoval = await request.post(`${API}${mutating[2]}`, {
    headers: { ...headers, "x-request-id": writes[4].headers["x-request-id"], "idempotency-key": writes[4].headers["idempotency-key"] },
    data: { itemId },
  });
  expect(replayRemoval.ok()).toBe(true);
  expect(await replayRemoval.json()).toEqual(writes[4].result);
  expect((await current()).config.enabled).toBe(true);
  expect((await current()).batch.items).toHaveLength(0);
  expect(await records(page)).toHaveLength(0);
  const actions = await (await request.get(`${API}/api/trading/action-runs`, { headers })).json();
  for (const write of writes) {
    const matching = actions.data.filter((run: any) => run.requestId === write.headers["x-request-id"]);
    expect(matching).toHaveLength(1);
    expect(matching[0].status).toBe("succeeded");
    expect(matching[0].result).toEqual(write.result);
  }
  await page.locator('.module-tabs button[data-module="settings"]').click();
  await page.getByRole("tab", { name: "动作账本", exact: true }).click();
  await expect(page.locator(".action-runs-table")).toContainText("加入链上批量监控");
  await expect(page.locator(".action-runs-table")).toContainText("移除链上批量市场");
  expect(writes).toHaveLength(5); expect(errors).toEqual([]); expect(unexpected).toEqual([]);
});

test("unknown onchain writes stay gated until the original receipt and current configuration are verified", async ({ page }) => {
  const f = await setup(page);
  let current: any = snapshot(), run: any, writes = 0, reject = true, hideRun = false, failRead = false;
  const actions = await (await page.request.get(`${API}/api/trading/action-runs`)).json();
  await page.route(`${API}/api/trading/action-runs**`, route => {
    const list = new URL(route.request().url()).pathname.endsWith("/action-runs");
    return route.fulfill({ json: list ? { ...actions, status: "ready", data: hideRun || !run ? [] : [run] } : run });
  });
  await page.route(`${API}/api/onchain/comparison`, route => failRead
    ? route.fulfill({ status: 503, json: { code: "CURRENT_UNAVAILABLE", message: "fixture: current settings unavailable" } })
    : route.fulfill({ json: current }));
  await page.route(`${API}/api/onchain/comparison/config`, route => {
    writes++;
    if (reject) return route.fulfill({ status: 400, json: { code: "INVALID_INPUT", message: "fixture: rejected configuration" } });
    const headers = route.request().headers();
    run = { id: "onchain-original", kind: "onchain_comparison_config_update", target: "onchain-cex-comparison",
      requestId: headers["x-request-id"], idempotencyKey: headers["idempotency-key"], actor: "fixture",
      status: "accepted", message: "fixture: original write pending", startedAtMs: NOW, updatedAtMs: NOW, result: null, problem: null };
    return route.fulfill({ status: 504, json: { code: "TIMEOUT", message: "fixture: response lost" } });
  });
  await page.goto(`${WEB}/#onchain`);
  await expect(amount(page)).toHaveValue("100");
  await amount(page).fill("12.345");
  await page.getByRole("button", { name: "应用变更", exact: true }).click();
  await expect(page.locator(".onchain-config-problem")).toContainText("INVALID_INPUT");
  await expect(amount(page)).toBeEnabled();
  await expect(amount(page)).toHaveValue("12.345");
  reject = false;
  await page.getByRole("button", { name: "应用变更", exact: true }).click();
  await expect(recovery(page)).toContainText("结果待核对");
  await expect(amount(page)).toBeDisabled();
  await expect(page.locator(".onchain-rail-header-tools .read-only-flag")).toHaveText("结果待核对");
  await page.reload();
  await expect(page.locator(".onchain-decision-board")).toContainText("当前配置待确认");
  await expect(page.getByLabel("双源行情时效")).not.toContainText("读取中");
  hideRun = true;
  await recheck(page).click();
  await expect(recovery(page)).toContainText("SETTINGS_RECEIPT_NOT_FOUND");
  hideRun = false;
  await recheck(page).click();
  await expect(recovery(page)).toContainText("后端已受理");
  const requestId = run.requestId;
  run.requestId = "unrelated";
  await recheck(page).click();
  await expect(recovery(page)).toContainText("SETTINGS_RECEIPT_MISMATCH");
  run.requestId = requestId; run.status = "succeeded";
  await recheck(page).click();
  await expect(recovery(page)).toContainText("SETTINGS_RECEIPT_MISSING");
  run.result = { ...snapshot(), observedAtMs: 0 };
  await recheck(page).click();
  await expect(recovery(page)).toContainText("SETTINGS_RECEIPT_MISMATCH");
  await capture(page);
  run.result = snapshot(NOW + 1);
  run.result.config.quoteAmountRaw = "12345000";
  failRead = true;
  await recheck(page).click();
  await expect(page.getByRole("alert", { name: "链上当前配置待同步" })).toBeVisible();
  await expect(page.locator(".onchain-config-problem")).toContainText("CURRENT_UNAVAILABLE");
  await expect(amount(page)).toBeDisabled();
  current = snapshot(NOW + 30); current.config.quoteAmountRaw = "21000000";
  failRead = false;
  await page.getByRole("button", { name: "读取当前配置", exact: true }).click();
  await expect(page.getByRole("alert", { name: "链上当前配置待同步" })).toBeHidden();
  await expect(amount(page)).toHaveValue("21");
  await expect(amount(page)).toBeEnabled();
  f.emit(snapshot());
  await expect(amount(page)).toHaveValue("21");
  expect(await records(page)).toHaveLength(0);
  expect(writes).toBe(2); expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});
