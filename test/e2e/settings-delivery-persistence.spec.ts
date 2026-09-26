import { expect, type Page } from "@playwright/test";
import { readFile, stat, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { test, SETTINGS_API as API } from "./fixtures/settings-server";

const webhookPath = "/api/webhook/config";
const webhookStatus = "/api/webhook/status";
const marketPath = "/api/system/market-subscriptions/config";
const marketStatus = "/api/system/market-subscriptions";

async function observe(page: Page, path: string) {
  const errors: string[] = [], writes: { data: any; key: string }[] = [];
  const network = { loseNext: false, lost: undefined as any };
  page.on("pageerror", error => errors.push(error.message));
  await page.route("**/*", async route => {
    const request = route.request(), url = new URL(request.url());
    expect([API, "http://127.0.0.1:18080"]).toContain(url.origin);
    if (!["GET", "HEAD", "OPTIONS"].includes(request.method()) && url.pathname !== "/api/auth/ws-ticket") {
      expect(url.pathname).toBe(path);
      writes.push({ data: request.postDataJSON(), key: request.headers()["idempotency-key"] });
      if (network.loseNext) {
        network.loseNext = false;
        const response = await route.fetch();
        expect(response.status()).toBe(200);
        network.lost = await response.json();
        return route.fulfill({ status: 504, json: { error: {
          code: "TIMEOUT", message: "isolated lost response", status: 504,
        } } });
      }
    }
    return route.continue();
  });
  return { errors, writes, network };
}

async function screenshot(page: Page, name: string) {
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
  await page.screenshot({ path: test.info().outputPath(name) });
}

test.describe("Webhook actual storage", () => {
  test.use({ settingsTab: "webhook" });
  test("failed clear preserves delivery configuration; lost save recovers without secrets after restart", async ({ page, server }) => {
    const secret = "synthetic-signing-secret-not-a-real-key";
    const target = "https://example.com/synthetic-device-token?key=synthetic-query-secret";
    const envPath = join(server.directory, ".env");
    await writeFile(envPath, "UNRELATED_FIXTURE_VALUE=keep-me\n");
    const seed = await server.patch(webhookPath, {
      enabled: true, provider: "generic", url: target, secret, timeoutMs: 1500,
    }, "webhook-seed");
    expect(seed.status()).toBe(200);
    const original = (await seed.json()).config;
    const originalFile = await readFile(envPath, "utf8");
    expect((await stat(envPath)).mode & 0o777).toBe(0o600);
    const { errors, writes, network } = await observe(page, webhookPath);
    await page.goto("/#settings");
    const panel = page.locator(".webhook-settings");
    await expect(panel.locator(".webhook-summary")).toContainText("已启用");
    await panel.locator(".webhook-advanced-settings > summary").click();
    const timeout = panel.getByLabel("超时 ms", { exact: true });
    const save = panel.getByRole("button", { name: "保存配置", exact: true });
    const recovery = page.getByRole("alert", { name: "设置操作待核对" });
    await server.blockFile(".env");
    const clear = await server.patch(webhookPath, { enabled: false, url: "", clearSecret: true }, "clear-failed");
    expect(clear.status()).toBe(503);
    expect((await clear.json()).error.code).toBe("WEBHOOK_CONFIG_STORAGE_FAILED");
    expect((await server.get(webhookStatus)).config).toEqual(original);
    await timeout.fill("2750");
    const failed = page.waitForResponse(r => r.url().endsWith(webhookPath) && r.status() === 503);
    await save.click();
    expect((await (await failed).json()).error.details.runtimeApplied).toBe(false);
    await expect(timeout).toHaveValue("2750");
    await expect(recovery).toBeVisible();
    await recovery.getByRole("button").click();
    await expect(recovery).toHaveCount(0);
    await server.unblockFile(".env");
    expect(await readFile(envPath, "utf8")).toBe(originalFile);

    network.loseNext = true;
    await save.click();
    await expect(recovery).toBeVisible();
    expect(writes).toHaveLength(2);
    await expect.poll(() => network.lost?.config?.timeoutMs).toBe(2750);
    const key = writes[1].key;
    const receipt = await server.receipt(key);
    expect(receipt.status).toBe("succeeded");
    await page.goto("about:blank");
    await server.restart();
    expect((await server.get(webhookStatus)).config).toEqual(network.lost.config);
    expect((await server.action(receipt.id)).result.config.timeoutMs).toBe(2750);
    const later = await server.patch(webhookPath, { timeoutMs: 3200 }, "webhook-later");
    expect(later.status()).toBe(200);
    await page.goto("/#settings");
    await expect(recovery).toBeVisible();
    await recovery.getByRole("button").click();
    await expect(recovery).toHaveCount(0);
    await panel.locator(".webhook-advanced-settings > summary").click();
    await expect(timeout).toHaveValue("3200");
    await expect(panel.getByRole("status")).toContainText("已核对：上次配置保存成功");
    await expect(panel.getByText("正在读取当前状态", { exact: false })).toHaveCount(0);
    expect(writes).toHaveLength(2);
    const replay = await server.patch(webhookPath, writes[1].data, key);
    expect(replay.status()).toBe(200);
    expect((await replay.json()).config.timeoutMs).toBe(2750);
    expect((await server.get(webhookStatus)).config.timeoutMs).toBe(3200);

    const together = await Promise.all([
      server.patch(webhookPath, { maxAttempts: 4 }, "webhook-concurrent-attempts"),
      server.patch(webhookPath, { queueCapacity: 42 }, "webhook-concurrent-capacity"),
    ]);
    for (const response of together) expect(response.status()).toBe(200);
    expect((await server.get(webhookStatus)).config).toMatchObject({ maxAttempts: 4, queueCapacity: 42 });
    await page.getByRole("button", { name: "刷新 Webhook 状态", exact: true }).click();
    await expect(panel.getByLabel("队列上限", { exact: true })).toHaveValue("42");
    await screenshot(page, "webhook-storage-desktop.png");
    await page.setViewportSize({ width: 390, height: 844 });
    await save.scrollIntoViewIfNeeded();
    await expect(save).toBeInViewport();
    await screenshot(page, "webhook-storage-mobile.png");
    const browserStorage = await page.evaluate(() => JSON.stringify({ ...sessionStorage, ...localStorage }));
    const audit = await server.audit();
    for (const value of [secret, "synthetic-device-token", "synthetic-query-secret"]) {
      expect(audit).not.toContain(value);
      expect(browserStorage).not.toContain(value);
      expect(JSON.stringify(await server.action(receipt.id))).not.toContain(value);
    }
    expect(audit).not.toContain("example.com");
    const cleared = await server.patch(webhookPath, { enabled: false, url: "", clearSecret: true }, "clear-success");
    expect(cleared.status()).toBe(200);
    const finalFile = await readFile(envPath, "utf8");
    expect(finalFile).toContain("UNRELATED_FIXTURE_VALUE=keep-me");
    expect(finalFile).not.toContain("APP_WEBHOOK__URL=");
    expect(finalFile).not.toContain("APP_WEBHOOK__SECRET=");
    await page.goto("about:blank");
    await server.restart();
    const final = await server.get(webhookStatus);
    expect(final.config).toMatchObject({ enabled: false, urlConfigured: false,
      secretConfigured: false, maxAttempts: 4, queueCapacity: 42, timeoutMs: 3200 });
    expect(final.queueDepth).toBe(0);
    expect(final.deliveredTotal).toBe(0);
    expect(final.recentDeliveries).toEqual([]);
    expect(errors).toEqual([]);
  });
});

test.describe("Market subscription actual storage", () => {
  test.use({ settingsTab: "market-data" });
  test("concurrent subscription patches persist; failed and lost responses recover after restart", async ({ page, server }) => {
    const concurrent = await Promise.all([
      server.patch(marketPath, { venue: "kraken", spotEnabled: false }, "kraken-spot"),
      server.patch(marketPath, { venue: "kraken", perpEnabled: false }, "kraken-perp"),
      server.patch(marketPath, { venue: "binance", fundingEnabled: false }, "binance-funding"),
    ]);
    for (const response of concurrent) expect(response.status()).toBe(200);
    const before = await server.get(marketStatus);
    expect(before.venues.find((row: any) => row.venue === "kraken")).toMatchObject({ spotEnabled: false, perpEnabled: false });
    expect(before.venues.find((row: any) => row.venue === "binance").fundingEnabled).toBe(false);
    const file = join(server.directory, "market-subscriptions.json");
    const checkpoint = await readFile(file, "utf8");
    const { errors, writes, network } = await observe(page, marketPath);
    await page.goto("/#settings");
    const toggle = page.getByRole("checkbox", { name: "kraken 现货", exact: true });
    const recovery = page.getByRole("alert", { name: "设置操作待核对" });
    await expect(toggle).not.toBeChecked();
    await server.blockFile("market-subscriptions.json");
    const failed = page.waitForResponse(r => r.url().endsWith(marketPath) && r.status() === 503);
    await toggle.click();
    expect((await (await failed).json()).error.code).toBe("MARKET_SUBSCRIPTION_STORAGE_FAILED");
    expect(await server.get(marketStatus)).toEqual(before);
    await expect(toggle).not.toBeChecked();
    await expect(recovery).toBeVisible();
    await recovery.getByRole("button").click();
    await expect(recovery).toHaveCount(0);
    await server.unblockFile("market-subscriptions.json");
    expect(await readFile(file, "utf8")).toBe(checkpoint);
    network.loseNext = true;
    await toggle.click();
    await expect(recovery).toBeVisible();
    expect(writes).toHaveLength(2);
    await expect.poll(() => network.lost?.venues?.find((row: any) => row.venue === "kraken")?.spotEnabled).toBe(true);
    const receipt = await server.receipt(writes[1].key);
    expect(receipt.status).toBe("succeeded");
    await page.goto("about:blank");
    await server.restart();
    expect((await server.get(marketStatus)).venues).toEqual(network.lost.venues);
    expect((await server.action(receipt.id)).result.venues).toEqual(network.lost.venues);
    expect((await server.patch(marketPath, { venue: "kraken", spotEnabled: false }, "kraken-newer")).status()).toBe(200);
    await page.goto("/#settings");
    await expect(recovery).toBeVisible();
    await recovery.getByRole("button").click();
    await expect(recovery).toHaveCount(0);
    await expect(toggle).not.toBeChecked();
    await expect(toggle).toBeEnabled();
    expect(writes).toHaveLength(2);
    const replay = await server.patch(marketPath, writes[1].data, writes[1].key);
    expect(replay.status()).toBe(200);
    expect((await replay.json()).venues).toEqual(network.lost.venues);
    expect((await server.get(marketStatus)).venues.find((row: any) => row.venue === "kraken").spotEnabled).toBe(false);
    const row = page.getByRole("row").filter({ has: toggle });
    await expect(row).toContainText("订阅已停用");
    await expect(row).toContainText("随永续暂停");
    await expect(page.locator(".settings-market-subscriptions .settings-summary-line")).toContainText("1/2 场所启用");
    await expect(page.locator(".settings-market-subscriptions").getByRole("status")).toHaveText("上次订阅配置已保存");
    const receiptDetails = page.locator(".settings-market-subscriptions details.settings-environment-evidence");
    await expect(receiptDetails).not.toHaveAttribute("open", "");
    await receiptDetails.getByText("操作结果", { exact: true }).click();
    await expect(receiptDetails).toContainText(receipt.id);
    await receiptDetails.getByText("操作结果", { exact: true }).click();
    await screenshot(page, "market-storage-desktop.png");
    await page.setViewportSize({ width: 390, height: 844 });
    await toggle.scrollIntoViewIfNeeded();
    await expect(toggle).toBeInViewport();
    await screenshot(page, "market-storage-mobile.png");
    expect(errors).toEqual([]);
  });
});
