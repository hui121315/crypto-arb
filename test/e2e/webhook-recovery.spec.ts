import { expect, test, type Page } from "@playwright/test";
import { settingsFixture } from "./fixtures/settings-workbench";

const API = "http://127.0.0.1:18000";
const WEB = "http://127.0.0.1:18080";
const recovery = (page: Page) => page.getByRole("alert", { name: "测试投递待核对" });
const records = (page: Page) => page.evaluate(() => Object.entries(sessionStorage).filter(([key]) => key.startsWith("crossline.settings.pending.v1:webhook-test:")));
async function openMonitor(page: Page, module: string) {
  await page.goto(`/#${module}`);
  const disclosure = page.locator(".webhook-monitor-disclosure");
  if (await disclosure.count()) await disclosure.locator("summary").first().click();
}
async function capture(page: Page, name: string) {
  const monitor = page.locator(".webhook-monitor");
  if (await monitor.count()) {
    const widths = await monitor.evaluate(el => ({
      monitor: el.clientWidth, feedback: el.querySelector(".webhook-test-feedback")!.clientWidth,
    }));
    expect(widths.feedback).toBeGreaterThan(widths.monitor - 30);
  }
  await page.screenshot({ path: test.info().outputPath(`${name}-desktop.png`) });
  await page.setViewportSize({ width: 390, height: 844 });
  await recovery(page).scrollIntoViewIfNeeded();
  await expect(recovery(page).getByRole("button")).toBeInViewport();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
  await page.screenshot({ path: test.info().outputPath(`${name}-mobile.png`) });
}

test("real queue survives lost response and refresh across three views without a second event", async ({ page, request }) => {
  const errors: string[] = [], unexpected: string[] = [];
  const headers = { Authorization: "Bearer isolated-paper-browser" };
  let posted: Record<string, string> | undefined, receipt: any, release = () => {};
  let writes = 0;
  await page.addInitScript(api => {
    localStorage.setItem("api_base", JSON.stringify(api));
    localStorage.setItem("api_auth_token", JSON.stringify("isolated-paper-browser"));
    localStorage.setItem("crossline.settings.activeTab", JSON.stringify("webhook"));
  }, API);
  page.on("pageerror", error => errors.push(error.message));
  await page.route("**/*", async route => {
    const req = route.request(), url = new URL(req.url());
    if (![API, WEB].includes(url.origin) || (!["GET", "HEAD"].includes(req.method())
      && !["/api/auth/ws-ticket", "/api/webhook/test"].includes(url.pathname))) {
      unexpected.push(`${req.method()} ${url.pathname}`); return route.abort();
    }
    if (url.pathname === "/api/webhook/test") {
      writes++;
      posted = req.headers();
      const response = await route.fetch();
      expect(response.ok()).toBe(true);
      receipt = await response.json();
      await new Promise<void>(resolve => release = resolve);
      await route.abort().catch(() => {});
      return;
    }
    return route.continue();
  });
  await page.goto("/#settings");
  await page.getByRole("button", { name: "发送测试", exact: true }).click();
  await expect.poll(() => receipt?.queued).toBe(true);
  const saved = JSON.stringify(await records(page));
  expect(saved).not.toContain("isolated-paper-browser");
  expect(saved).not.toContain("CROSSLINE UI test");
  await page.reload(); release();
  await expect(recovery(page)).toContainText("结果待核对");
  await page.getByRole("button", { name: "核对测试投递", exact: true }).click();
  await expect(recovery(page)).toContainText("等待投递结果");
  await expect(page.getByRole("button", { name: "发送测试", exact: true })).toBeDisabled();
  await expect(page.getByRole("button", { name: "保存配置", exact: true })).toBeDisabled();
  for (const module of ["opportunities", "automation"]) {
    await openMonitor(page, module);
    await expect(recovery(page)).toContainText("等待投递结果");
    await expect(page.getByRole("button", { name: "测试投递", exact: true })).toBeDisabled();
  }
  const duplicate = await request.post(`${API}/api/webhook/test`, {
    headers: { ...headers, "x-request-id": posted!["x-request-id"], "idempotency-key": posted!["idempotency-key"] },
    data: { message: "same request must replay without another event" },
  });
  expect(duplicate.ok()).toBe(true);
  expect(await duplicate.json()).toEqual(receipt);
  const status = await (await request.get(`${API}/api/webhook/status`, { headers })).json();
  expect(status.queueDepth).toBe(1);
  expect(status.deliveredTotal).toBe(0);
  const run = await (await request.get(`${API}/api/trading/action-runs/${receipt.actionRunId}`, { headers })).json();
  expect(run.kind).toBe("webhook_test"); expect(run.status).toBe("succeeded");
  expect(run.result).toEqual(receipt);
  expect(JSON.stringify(run)).not.toContain("CROSSLINE UI test");
  await capture(page, "queued-test");
  expect(writes).toBe(1); expect(errors).toEqual([]); expect(unexpected).toEqual([]);
});

test("test recovery rejects missing or mismatched receipts and tracks only its own delivery", async ({ page }) => {
  const f = await settingsFixture(page);
  const path = "POST /api/webhook/test";
  f.hold(path);
  await page.goto("/#settings");
  await page.getByRole("button", { name: "发送测试", exact: true }).click();
  await expect.poll(() => f.actions.data.length).toBe(1);
  const run = f.actions.data[0];
  await page.reload();
  await expect(recovery(page)).toBeVisible();
  await page.getByRole("button", { name: "核对测试投递", exact: true }).click();
  await expect(recovery(page)).toContainText("后端已受理");
  f.release(path);
  await expect.poll(() => run.status).toBe("succeeded");
  const result = structuredClone(run.result);
  run.result = null;
  await page.getByRole("button", { name: "核对测试投递", exact: true }).click();
  await expect(recovery(page)).toContainText("SETTINGS_RECEIPT_MISSING");
  run.result = { ...result, eventId: "unrelated-event" };
  await page.getByRole("button", { name: "核对测试投递", exact: true }).click();
  await expect(recovery(page)).toContainText("SETTINGS_RECEIPT_MISMATCH");
  run.result = result;
  f.fail("GET /api/webhook/status");
  await page.getByRole("button", { name: "核对测试投递", exact: true }).click();
  await expect(recovery(page)).toContainText("等待投递结果");
  await expect(page.locator(".webhook-test-feedback")).toContainText("SETTINGS_FIXTURE_UNAVAILABLE");
  f.fail("GET /api/webhook/status", false);
  // The unrelated successful history row cannot release this request.
  await expect(page.getByRole("button", { name: "发送测试", exact: true })).toBeDisabled();
  await page.reload();
  await page.getByRole("button", { name: "核对测试投递", exact: true }).click();
  await expect(recovery(page)).toContainText("等待投递结果");
  f.webhook.recentDeliveries.unshift({ ...f.webhook.recentDeliveries[0], eventId: result.eventId,
    status: "queued", applicationAck: "unknown", responseStatus: null as any });
  f.webhook.updatedAtMs++; f.emit();
  await expect(page.getByRole("button", { name: "发送测试", exact: true })).toBeDisabled();
  await expect(page.locator(".webhook-test-feedback")).not.toContainText("SETTINGS_FIXTURE_UNAVAILABLE");
  await capture(page, "pending-test");
  f.webhook.recentDeliveries[0].status = "delivered";
  f.webhook.recentDeliveries[0].applicationAck = "transport_only";
  f.webhook.updatedAtMs++; f.emit();
  await expect(recovery(page)).toHaveCount(0);
  await expect(page.getByRole("status").filter({ hasText: "仅传输成功" })).toBeVisible();
  expect(await records(page)).toEqual([]);
  await expect(page.getByRole("button", { name: "保存配置", exact: true })).toBeEnabled();

  // Second deliberate request, failure belongs to it, not the previous success.
  f.hold(path); await page.getByRole("button", { name: "发送测试", exact: true }).click();
  await expect.poll(() => f.actions.data.length).toBe(2);
  await page.reload(); f.release(path);
  const second = f.actions.data[0];
  await expect.poll(() => second.status).toBe("succeeded");
  f.webhook.recentDeliveries.unshift({ ...f.webhook.recentDeliveries[0],
    eventId: second.result.eventId, status: "failed", applicationAck: "rejected", responseStatus: 500 });
  f.webhook.updatedAtMs++;
  await page.getByRole("button", { name: "核对测试投递", exact: true }).click();
  await expect(recovery(page)).toHaveCount(0);
  await expect(page.getByRole("status").filter({ hasText: "测试消息投递失败" })).toBeVisible();
  expect(f.requests.filter(r => r.method !== "GET")).toHaveLength(2);
  expect(await records(page)).toEqual([]);
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});
