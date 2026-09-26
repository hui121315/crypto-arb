import { expect, type Page } from "@playwright/test";
import { readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { test, SETTINGS_API as API, settingsHeaders } from "./fixtures/settings-server";

async function watch(page: Page, writablePath: string) {
  const errors: string[] = [], writes: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.route("**/*", async route => {
    const request = route.request(), url = new URL(request.url());
    expect([API, "http://127.0.0.1:18080"]).toContain(url.origin);
    if (!["GET", "HEAD", "OPTIONS"].includes(request.method()) && url.pathname !== "/api/auth/ws-ticket") {
      expect(url.pathname).toBe(writablePath);
      writes.push(url.pathname);
    }
    await route.continue();
  });
  return { errors, writes };
}

async function captures(page: Page, name: string, openMobilePanel?: () => Promise<void>) {
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
  await page.screenshot({ path: test.info().outputPath(`${name}-desktop.png`) });
  await page.setViewportSize({ width: 390, height: 844 });
  await openMobilePanel?.();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
  await page.screenshot({ path: test.info().outputPath(`${name}-mobile.png`) });
  await page.setViewportSize({ width: 1440, height: 900 });
}

test.describe("Market restore failure", () => {
  test.use({ settingsTab: "market-data" });
  test("corrupt subscription checkpoints block actual feed gates and preserve the file until repaired", async ({ page, request, server }) => {
    const endpoint = "/api/system/market-subscriptions/config";
    expect((await server.patch(endpoint, { venue: "kraken", spotEnabled: false,
      perpEnabled: false, fundingEnabled: true }, "market-valid")).status()).toBe(200);
    const file = join(server.directory, "market-subscriptions.json");
    const original = await readFile(file, "utf8");
    const good = JSON.parse(original);
    const { errors, writes } = await watch(page, endpoint);
    const corruptions = [
      '{"version":1,"venues":[',
      JSON.stringify({ ...good, version: 999 }),
      JSON.stringify({ ...good, venues: [...good.venues, { ...good.venues[0], venue: " KRAKEN " }] }),
    ];
    for (const [index, contents] of corruptions.entries()) {
      await page.goto("about:blank");
      await server.stop();
      await writeFile(file, contents);
      // The native fixture asserts all Spot/Perp/Funding gates (including scoped venues) are off.
      await server.start();
      const read = await request.get(`${API}/api/system/market-subscriptions`, { headers: settingsHeaders });
      expect(read.status()).toBe(503);
      expect((await read.json()).error.code).toBe("MARKET_SUBSCRIPTION_RESTORE_FAILED");
      const save = await server.patch(endpoint, { venue: "kraken", spotEnabled: true }, `blocked-${index}`);
      expect(save.status()).toBe(503);
      expect((await save.json()).error.details.subscriptionsBlocked).toBe(true);
      expect(await readFile(file, "utf8")).toBe(contents);
      if (index === 0) {
        await page.goto("/#settings");
        const panel = page.locator(".settings-market-subscriptions");
        await expect(panel.getByRole("alert")).toContainText("行情订阅配置恢复失败");
        await expect(panel.locator(".settings-summary-line")).toContainText("配置不可用");
        await expect(panel.getByRole("checkbox")).toHaveCount(0);
        await expect(panel).not.toContainText("旧状态仅供参考");
        await expect(panel).not.toContainText("开关为已保存配置");
        await panel.getByRole("button", { name: "刷新行情订阅" }).click();
        await expect(panel.getByRole("alert")).toContainText("原文件已保留");
        await captures(page, "market-restore-blocked");
      }
    }
    await server.stop();
    await writeFile(file, original);
    await server.start();
    expect((await server.get("/api/system/market-subscriptions")).venues).toEqual(good.venues);
    await page.goto("/#settings");
    const toggle = page.getByRole("checkbox", { name: "kraken 现货", exact: true });
    await expect(toggle).not.toBeChecked();
    const saved = page.waitForResponse(r => r.url().endsWith(endpoint) && r.status() === 200);
    await toggle.click();
    await saved;
    await expect(toggle).toBeChecked();
    expect(writes).toHaveLength(1);
    await page.goto("about:blank");
    await server.restart();
    expect((await server.get("/api/system/market-subscriptions")).venues[0].spotEnabled).toBe(true);
    expect(errors).toEqual([]);
  });
});

test.describe("Webhook restore failure", () => {
  test.use({ settingsTab: "webhook" });
  test("invalid stored configuration blocks delivery and shared readiness without discarding pending notifications", async ({ page, server }) => {
    const path = "/api/webhook/config", statusPath = "/api/webhook/status";
    const values = { APP_WEBHOOK__ENABLED: "true", APP_WEBHOOK__PROVIDER: "generic",
      APP_WEBHOOK__URL: "https://example.com/synthetic-private-target",
      APP_WEBHOOK__SECRET: "synthetic-private-signing-value", APP_WEBHOOK__TIMEOUT_MS: "2500",
      APP_WEBHOOK__EVENT_KINDS: "opportunity,automation_decision" };
    const file = join(server.directory, ".env");
    expect((await server.patch(path, { enabled: true, provider: "generic", url: values.APP_WEBHOOK__URL,
      secret: values.APP_WEBHOOK__SECRET, timeoutMs: 2500,
      eventKinds: ["opportunity", "automation_decision"] }, "webhook-valid")).status()).toBe(200);
    const original = await readFile(file, "utf8");
    // This isolated API has no delivery worker: only a local outbox entry is created.
    expect((await server.patch("/api/webhook/test", { message: "synthetic local outbox only" }, "local-outbox", "POST")).status()).toBe(200);
    expect((await server.get(statusPath)).queueDepth).toBe(1);
    const { errors, writes } = await watch(page, path);
    const invalid = [
      { APP_WEBHOOK__TIMEOUT_MS: "not-a-number" },
      { APP_WEBHOOK__PROVIDER: "unknown-provider" },
      { APP_WEBHOOK__EVENT_KINDS: "opportunity,unknown-event" },
      { APP_WEBHOOK__ENABLED: "maybe" },
    ];
    for (const [index, overrides] of invalid.entries()) {
      await page.goto("about:blank");
      await server.stop();
      const contents = Object.entries({ ...values, ...overrides })
        .map(([key, value]) => `${key}=${JSON.stringify(value)}`).join("\n") + "\n";
      await writeFile(file, contents);
      await server.start();
      const status = await server.get(statusPath);
      expect(status.configurationProblem.code).toBe("WEBHOOK_CONFIG_RESTORE_FAILED");
      expect(status.config.enabled).toBe(false);
      expect(status.queueDepth).toBe(1);
      for (const forbidden of [values.APP_WEBHOOK__URL, values.APP_WEBHOOK__SECRET, Object.values(overrides)[0]]) {
        expect(JSON.stringify(status)).not.toContain(forbidden);
      }
      const blocked = await server.patch(path, { enabled: true }, `blocked-webhook-${index}`);
      expect(blocked.status()).toBe(503);
      expect((await blocked.json()).error.code).toBe("WEBHOOK_CONFIG_RESTORE_FAILED");
      const forced = await server.patch("/api/webhook/test", { message: "must not queue" }, `blocked-test-${index}`, "POST");
      expect(forced.status()).toBe(503);
      expect((await server.get(statusPath)).queueDepth).toBe(1);
      expect(await readFile(file, "utf8")).toBe(contents);
      if (index === 0) {
        await page.goto("/#settings");
        const panel = page.locator(".webhook-settings");
        await expect(panel.getByRole("alert")).toContainText("Webhook 配置恢复失败");
        await expect(panel.getByRole("button", { name: "保存配置", exact: true })).toBeDisabled();
        await expect(panel.getByRole("button", { name: "发送测试", exact: true })).toBeDisabled();
        await expect(panel.locator(".webhook-summary")).toHaveCount(0);
        await expect(panel.locator(".webhook-config-editor")).toHaveCount(0);
        await captures(page, "webhook-restore-blocked");
        for (const module of ["opportunities", "automation"]) {
          await page.goto(`/#${module}`);
          const disclosure = page.locator(".webhook-monitor-disclosure");
          await expect(disclosure.locator(":scope > summary")).toContainText("配置恢复失败 · 投递暂停");
          await disclosure.locator(":scope > summary").click();
          await expect(disclosure.getByRole("button", { name: "测试投递", exact: true })).toBeDisabled();
          await expect(disclosure.getByRole("alert")).toContainText("原配置未改动");
          if (module === "automation") {
            await page.getByRole("tab", { name: "处理流程", exact: true }).click();
          }
          await expect(page.locator(".deterministic-flow li").filter({ hasText: "Webhook" }))
            .toContainText("配置恢复失败");
        }
        await page.goto("/#onchain");
        await page.locator("#onchain-config-tab-alerts").click();
        const onchain = page.getByRole("region", { name: "链上价差 Webhook", exact: true });
        await expect(onchain.locator(".onchain-alert-heading")).toContainText("配置恢复失败");
        await expect(onchain.locator(".onchain-alert-runtime")).toBeVisible();
        await expect(onchain.locator(".onchain-alert-runtime")).toContainText("原配置未改动");
        await captures(page, "onchain-webhook-restore", async () => {
          await page.getByRole("navigation", { name: "链上套利工作区" })
            .getByRole("button", { name: "接入", exact: true }).click();
          const problem = onchain.locator(".onchain-alert-runtime");
          await problem.evaluate(element => element.scrollIntoView({ block: "center" }));
          await expect(problem).toBeInViewport({ ratio: 1 });
          expect(await problem.evaluate(element => {
            const rect = element.getBoundingClientRect();
            return [rect.top + 4, rect.bottom - 4].every(y =>
              element.contains(document.elementFromPoint(rect.left + rect.width / 2, y)));
          })).toBe(true);
        });
      }
    }
    await server.stop();
    await writeFile(file, original);
    await server.start();
    const restored = await server.get(statusPath);
    expect(restored.configurationProblem).toBeUndefined();
    expect(restored.config.enabled).toBe(true);
    expect(restored.queueDepth).toBe(1);
    expect(restored.deliveredTotal).toBe(0);
    await page.goto("/#settings");
    const panel = page.locator(".webhook-settings");
    await expect(panel.locator(".webhook-summary")).toContainText("已启用");
    await expect(panel.getByRole("button", { name: "保存配置", exact: true })).toBeEnabled();
    expect(writes).toEqual([]);
    const audit = await server.audit();
    expect(audit).not.toContain(values.APP_WEBHOOK__URL);
    expect(audit).not.toContain(values.APP_WEBHOOK__SECRET);
    expect(errors).toEqual([]);
  });
});
