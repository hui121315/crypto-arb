import { expect } from "@playwright/test";
import { test, SETTINGS_API as API } from "./fixtures/settings-server";
import { writeFile } from "node:fs/promises";

test("settings save failure preserves runtime; lost receipt recovers after an actual API restart", async ({ page, server }) => {
  const seed = await server.write("risk-config", { maxOrderNotional: 10,
    allowedExchanges: ["binance", "bitget"], autoProfitClose: { enabled: false, minNetProfitUsd: 0.125 } }, "seed", "PATCH");
  expect(seed.status()).toBe(200);
  const original = await server.snapshot();
  const errors: string[] = [], writes: any[] = [];
  page.on("pageerror", e => errors.push(e.message));
  let loseResponse = false, lost: any;
  await page.route("**/*", async route => {
    const request = route.request(), url = new URL(request.url());
    expect([API, "http://127.0.0.1:18080"]).toContain(url.origin);
    if (!["GET", "HEAD", "OPTIONS"].includes(request.method()) && url.pathname !== "/api/auth/ws-ticket") {
      expect(url.pathname).toBe("/api/trading/risk-config");
      writes.push({ data: request.postDataJSON(), key: request.headers()["idempotency-key"] });
      if (loseResponse) {
        loseResponse = false;
        const response = await route.fetch();
        expect(response.status()).toBe(200);
        lost = await response.json();
        return route.fulfill({ status: 504, json: { error: { code: "TIMEOUT", message: "isolated lost response", status: 504 } } });
      }
    }
    return route.continue();
  });
  await page.goto("/#settings");
  const amount = page.getByLabel(/单笔名义上限 USD/);
  const save = page.getByRole("button", { name: "保存风控", exact: true });
  const recovery = page.getByRole("alert", { name: "设置操作待核对" });
  await expect(amount).toHaveValue("10");
  await server.blockCheckpoint();
  await amount.fill("12.75");
  const failedResponse = page.waitForResponse(r => r.url().endsWith("/api/trading/risk-config") && r.status() === 503);
  await save.click();
  const failure = await (await failedResponse).json();
  expect(failure.error.code).toBe("TRADING_CONFIG_STORAGE_FAILED");
  expect(failure.error.details.runtimeApplied).toBe(false);
  expect((await server.status()).risk).toEqual(original.runtime.risk);
  await expect(amount).toHaveValue("12.75");
  await expect(recovery).toBeVisible();
  await recovery.getByRole("button").click();
  await expect(recovery).toHaveCount(0);
  await expect(save).toBeEnabled();
  await server.unblockCheckpoint();
  expect(await server.snapshot()).toEqual(original);

  await amount.fill("12.75");
  loseResponse = true;
  await save.click();
  await expect(recovery).toBeVisible();
  expect(writes).toHaveLength(2);
  expect(writes[0].key).not.toBe(writes[1].key);
  await expect.poll(() => lost?.risk?.maxOrderNotional).toBe(12.75);
  await page.goto("about:blank");
  await server.restart();
  expect((await server.status()).risk.maxOrderNotional).toBe(12.75);
  expect((await server.action(lost.actionRunId)).result).toEqual(lost);
  await page.goto("/#settings");
  await expect(recovery).toBeVisible();
  await recovery.getByRole("button").click();
  await expect(recovery).toHaveCount(0);
  await expect(amount).toHaveValue("12.75");
  await expect(save).toBeEnabled();
  expect(writes).toHaveLength(2);

  const current = await server.write("risk-config", { maxOrderNotional: 25.5 }, "later", "PATCH");
  expect(current.status()).toBe(200);
  const replay = await server.write("risk-config", writes[1].data, writes[1].key, "PATCH");
  expect(replay.status()).toBe(200);
  expect(await replay.json()).toEqual(lost);
  expect((await server.status()).risk.maxOrderNotional).toBe(25.5);
  await page.getByRole("button", { name: "刷新风控状态", exact: true }).click();
  await expect(amount).toHaveValue("25.5");
  expect((await server.snapshot()).runtime.risk.autoProfitClose.minNetProfitUsd).toBe(0.125);
  await page.screenshot({ path: test.info().outputPath("settings-persist-desktop.png") });
  await page.setViewportSize({ width: 390, height: 844 });
  await save.scrollIntoViewIfNeeded();
  await expect(save).toBeInViewport();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
  await page.screenshot({ path: test.info().outputPath("settings-persist-mobile.png") });
  expect(errors).toEqual([]);
});

test("storage failure cannot change adapters or release a stop; emergency stop and restart receipts stay truthful", async ({ page, server }) => {
  expect((await server.write("risk-config", { allowedExchanges: ["binance"] }, "seed", "PATCH")).status()).toBe(200);
  const before = await server.status();
  await server.blockCheckpoint();
  const adapter = await server.write("adapters/select", { adapterId: "mock" }, "adapter-failed");
  expect(adapter.status()).toBe(503);
  expect((await adapter.json()).error.code).toBe("TRADING_CONFIG_STORAGE_FAILED");
  expect(await server.status()).toEqual(before);
  const stop = (active: boolean, expectedActive: boolean) => ({ active, expectedActive, expectedOpenOrderCount: 0, reason: "isolated settings check" });
  await page.goto("/#settings");
  const emergencyResponse = page.waitForResponse(r => r.url().endsWith("/api/trading/kill-switch") && r.status() === 503);
  await page.getByRole("button", { name: "切换 Kill Switch", exact: true }).click();
  const emergency = await emergencyResponse;
  expect(emergency.status()).toBe(503);
  expect((await emergency.json()).error.details.runtimeApplied).toBe(true);
  expect((await server.status()).risk.killSwitchActive).toBe(true);
  await expect(page.locator('[data-settings-risk-scope="kill-switch-action"]')).toContainText("当前进程已急停");
  const recovery = page.getByRole("alert", { name: "设置操作待核对" });
  await expect(recovery).toBeVisible();
  await recovery.getByRole("button").click();
  await expect(recovery).toHaveCount(0);
  await expect(page.locator('[data-settings-risk-scope="runtime-readonly"]')).toContainText("Kill Switch 开启");
  const release = await server.write("kill-switch", stop(false, true), "release-failed");
  expect(release.status()).toBe(503);
  expect((await release.json()).error.details.runtimeApplied).toBe(false);
  expect((await server.status()).risk.killSwitchActive).toBe(true);
  await expect(page.locator('[data-settings-risk-scope="runtime-readonly"]')).toContainText("Kill Switch 开启");
  await server.unblockCheckpoint();
  const saved = await server.write("kill-switch", stop(true, true), "stop-saved");
  expect(saved.status()).toBe(200);
  const receipt = await saved.json();
  await page.goto("about:blank");
  await server.restart();
  expect((await server.status()).risk.killSwitchActive).toBe(true);
  expect((await server.action(receipt.actionRunId)).result).toEqual(receipt);
  expect((await server.write("kill-switch", stop(true, true), "stop-saved")).status()).toBe(200);
  const selected = await server.write("adapters/select", { adapterId: "mock" }, "adapter-saved");
  expect(selected.status()).toBe(200);
  const selectedReceipt = await selected.json();
  await server.restart();
  expect((await server.action(selectedReceipt.actionRunId)).result).toEqual(selectedReceipt);
  expect((await server.write("adapters/select", { adapterId: "mock" }, "adapter-saved")).status()).toBe(200);

  // Corrupt only the isolated checkpoint: invalid risk must not activate a live adapter.
  await server.stop();
  const corrupt = await server.snapshot();
  corrupt.runtime.adapterId = "live";
  corrupt.runtime.risk.maxOrderNotional = 0;
  await writeFile(server.checkpoint, JSON.stringify(corrupt));
  await server.start();
  const safe = await server.status();
  expect(safe.adapter).toBe("mock");
  expect(safe.risk.liveTradingEnabled).toBe(false);
  expect(safe.risk.maxOrderNotional).toBeGreaterThan(0);
  const audit = await server.audit();
  expect(audit).not.toContain("isolated-settings-browser");
});
