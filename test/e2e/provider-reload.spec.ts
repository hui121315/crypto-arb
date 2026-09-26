import { expect, test, type Page } from "@playwright/test";
import { evidenceRoutes } from "./fixtures/settings-evidence";
import { setup as onchainFixture } from "./fixtures/onchain-workbench";
import { NOW } from "./fixtures/opportunity-workbench";

const prefix = "crossline.provider.pending.v1:";
const save = "POST /api/onchain/credentials";
const clear = "POST /api/onchain/credentials/clear";

async function setup(page: Page) {
  const base = await onchainFixture(page);
  const fixture = await evidenceRoutes(page, true);
  await page.addInitScript(() => {
    localStorage.setItem("crossline.settings.activeTab", JSON.stringify("credentials"));
    localStorage.setItem("crossline.settings.credentialTask", JSON.stringify("onchain-provider"));
    const { getItem, setItem, removeItem } = Storage.prototype;
    const fault = () => getItem.call(sessionStorage, "fixture.providerFault");
    Storage.prototype.getItem = function (key) {
      if (key.startsWith("crossline.provider.pending.v1:") && fault() === "read") throw new DOMException("fixture storage denied", "SecurityError");
      return getItem.call(this, key);
    };
    Storage.prototype.setItem = function (key, value) {
      if (key.startsWith("crossline.provider.pending.v1:") && fault() === "write") throw new DOMException("fixture quota", "QuotaExceededError");
      return setItem.call(this, key, value);
    };
    Storage.prototype.removeItem = function (key) {
      if (key.startsWith("crossline.provider.pending.v1:") && fault() === "remove") throw new DOMException("fixture storage denied", "SecurityError");
      return removeItem.call(this, key);
    };
  });
  await page.route(/\/api\/stocks(?:\/|$)/, (route) => {
    if (route.request().method() !== "GET") return route.fallback();
    return route.fulfill({ json: new URL(route.request().url()).pathname === "/api/stocks" ? {
      security: null, tokens: [], books: [], connected: false, reference: null, problem: null, observedAtMs: NOW,
    } : { rows: [], recoveryProblem: null, observedAtMs: NOW } });
  });
  await page.goto("/#settings");
  const form = page.locator(".provider-credentials-body");
  await expect(form.locator("input")).toBeEnabled();
  return { ...base, ...fixture, form };
}

async function records(page: Page) {
  return page.evaluate((prefix) => Object.entries(sessionStorage).filter(([key]) => key.startsWith(prefix)), prefix);
}

async function fault(page: Page, mode: string) {
  await page.evaluate((mode) => sessionStorage.setItem("fixture.providerFault", mode), mode);
}

async function auth(page: Page, token: string) {
  await page.getByRole("tab", { name: "诊断", exact: true }).click();
  await page.getByLabel("Token", { exact: true }).fill(token);
  await page.getByRole("button", { name: "保存 Token", exact: true }).click();
  await page.getByRole("tab", { name: "凭证", exact: true }).click();
}

test("provider reload recovers in-flight save and clear across all editors without resending", async ({ page }) => {
  const f = await setup(page);
  const secret = "fixture-reload-secret-never-store";
  await f.form.locator("input").fill(secret);
  f.holdEvidence(save);
  await f.form.getByRole("button", { name: "保存新凭证" }).click();
  await expect.poll(() => f.calls.filter((row) => row.key === save).length).toBe(1);
  const stored = await records(page);
  expect(stored).toHaveLength(1);
  expect(stored[0][0]).toMatch(/^crossline\.provider\.pending\.v1:[a-f0-9]{64}$/);
  const identity = JSON.parse(stored[0][1]);
  expect(identity.attempt.context.request_id).toBe(f.calls.find((row) => row.key === save)!.requestId);
  expect(identity.attempt.context.idempotency_key).toBe(f.calls.find((row) => row.key === save)!.idempotency);
  expect(JSON.stringify(stored)).not.toContain(secret);
  expect(JSON.stringify(stored)).not.toContain("isolated-fixture-token");

  // Reload before the 20-second timeout, while the server has accepted the write.
  await page.reload();
  const recovery = page.getByRole("alert", { name: "凭证结果待核对" });
  await expect(recovery).toContainText("Jupiter API Key · 保存结果待核对");
  await expect(f.form.locator("input")).toHaveValue("");
  await expect(f.form.locator("input")).toBeDisabled();
  const run = f.actions.data[0];
  await auth(page, "fixture-other-auth");
  await expect(recovery).toHaveCount(0);
  await expect(f.form.locator("input")).toBeEnabled();
  expect(await records(page)).toEqual(stored);
  await auth(page, "isolated-fixture-token");
  await expect(recovery).toBeVisible();
  await expect(f.form.locator("input")).toBeDisabled();

  await page.evaluate(() => { location.hash = "onchain"; });
  await page.getByRole("tab", { name: /^接入/ }).click();
  await expect(recovery).toContainText("Jupiter API Key · 保存结果待核对");
  await recovery.getByRole("button", { name: "核对上次操作" }).click();
  await expect(recovery).toContainText("后端已受理，尚未完成");
  expect(JSON.parse((await records(page))[0][1]).attempt.run_id).toBe(run.id);
  await page.evaluate(() => { location.hash = "stocks"; });
  await page.getByRole("button", { name: "询价与执行", exact: true }).click();
  await page.locator(".stock-arbitrage-page .provider-credentials summary").first().click();
  await expect(recovery).toContainText("Jupiter");
  await expect(page.locator(".stock-arbitrage-page .provider-credentials input").first()).toBeDisabled();

  await page.evaluate(() => { location.hash = "settings"; });
  await page.reload();
  await expect(recovery).toContainText("Jupiter");
  await page.screenshot({ path: test.info().outputPath("provider-reload-desktop.png") });
  await page.setViewportSize({ width: 390, height: 844 });
  await recovery.scrollIntoViewIfNeeded();
  await expect(recovery.getByRole("button")).toBeInViewport();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
  await page.screenshot({ path: test.info().outputPath("provider-reload-mobile.png") });

  // An indexed run ID survives a second reload, even if the list is pruned.
  await page.route(`**/api/trading/action-runs/${run.id}`, (route) => route.fulfill({ json: run }));
  f.actions.data = [];
  f.releaseEvidence(save);
  await expect.poll(() => run.status).toBe("succeeded");
  await expect(recovery).toBeVisible();
  await recovery.getByRole("button", { name: "核对上次操作" }).click();
  await expect(recovery).toHaveCount(0);
  await expect(f.form.getByRole("status")).toContainText("fixture saved");
  expect(await records(page)).toEqual([]);

  f.failEvidence(clear); f.holdEvidence(clear);
  await f.form.getByRole("button", { name: "清除当前凭证" }).click();
  await f.form.getByRole("button", { name: "再次点击确认清除" }).click();
  await expect.poll(() => f.calls.filter((row) => row.key === clear).length).toBe(1);
  await page.reload();
  await expect(recovery).toContainText("清除结果待核对");
  f.releaseEvidence(clear);
  await expect.poll(() => f.actions.data[0].status).toBe("failed");
  await recovery.getByRole("button", { name: "核对上次操作" }).click();
  await expect(recovery).toHaveCount(0);
  await expect(f.form.getByRole("alert")).toContainText("原操作已确认失败");
  await expect(f.form.locator("input")).toBeEnabled();
  expect(await records(page)).toEqual([]);
  expect(f.calls.filter((row) => row.key === save)).toHaveLength(1);
  expect(f.calls.filter((row) => row.key === clear)).toHaveLength(1);
  expect(JSON.stringify(f.actions)).not.toContain(secret);
  expect(await page.evaluate(() => JSON.stringify({ ...localStorage, ...sessionStorage }))).not.toContain(secret);
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});

test("provider storage failures never dispatch or silently unlock an unconfirmed mutation", async ({ page }) => {
  const f = await setup(page);
  const storageError = page.getByRole("alert", { name: "凭证恢复记录不可用" });
  const recovery = page.getByRole("alert", { name: "凭证结果待核对" });
  await fault(page, "read");
  await page.reload();
  await expect(storageError).toContainText("无法读取恢复记录");
  await expect(f.form.locator("input")).toBeDisabled();
  await fault(page, "");
  await storageError.getByRole("button").click();
  await expect(f.form.locator("input")).toBeEnabled();
  await fault(page, "write");
  await f.form.locator("input").fill("fixture-quota-secret");
  await f.form.getByRole("button", { name: "保存新凭证" }).click();
  await expect(storageError).toContainText("无法更新浏览器恢复记录");
  expect(f.calls.filter((row) => row.key === save)).toEqual([]);
  await expect(f.form.locator("input")).toBeDisabled();
  await fault(page, "");
  await storageError.getByRole("button").click();
  await expect(f.form.locator("input")).toBeEnabled();

  f.holdEvidence(save);
  await f.form.getByRole("button", { name: "保存新凭证" }).click();
  await expect.poll(() => f.calls.filter((row) => row.key === save).length).toBe(1);
  const stored = (await records(page))[0];
  await fault(page, "remove");
  f.releaseEvidence(save);
  await expect(recovery).toContainText("无法更新浏览器恢复记录");
  await expect(f.form.locator("input")).toBeDisabled();
  await page.reload();
  await recovery.getByRole("button", { name: "核对上次操作" }).click();
  await expect(recovery).toContainText("无法更新浏览器恢复记录");
  expect(await records(page)).toEqual([stored]);

  await fault(page, "");
  await page.evaluate(([key]) => sessionStorage.setItem(key, "{broken"), stored);
  await page.reload();
  await expect(storageError).toContainText("恢复记录格式不完整");
  await storageError.getByRole("button").click();
  await expect(storageError).toBeVisible();
  await expect(f.form.locator("input")).toBeDisabled();
  await page.setViewportSize({ width: 390, height: 844 });
  await storageError.scrollIntoViewIfNeeded();
  await expect(storageError.getByRole("button")).toBeInViewport();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
  await page.screenshot({ path: test.info().outputPath("provider-storage-mobile.png") });
  // Repair the fixture record, not the UI: unreadable records are never discarded.
  await page.evaluate(([key, value]) => sessionStorage.setItem(key, value), stored);
  await storageError.getByRole("button").click();
  await expect(recovery).toBeVisible();
  await recovery.getByRole("button", { name: "核对上次操作" }).click();
  await expect(recovery).toHaveCount(0);
  await expect(f.form.getByRole("status")).toContainText("fixture saved");
  await expect(f.form.locator("input")).toBeEnabled();
  expect(await records(page)).toEqual([]);
  expect(f.calls.filter((row) => row.key === save)).toHaveLength(1);
  expect(f.calls.filter((row) => row.key === clear)).toHaveLength(0);
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});
