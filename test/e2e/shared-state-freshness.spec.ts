import { expect, test, type Page } from "@playwright/test";
import { API, NOW, setup } from "./fixtures/opportunity-workbench";
import { settingsAccountFixture } from "./fixtures/settings-account";

const paths = ["/api/system/health", "/api/system/venue-operation-health", "/api/trading/status"];
const original = "isolated-fixture-token";
const mode = (page: Page) => page.getByRole("group", { name: "执行环境", exact: true }).locator(".slot");
const frame = async (page: Page) => page.evaluate(() => new Promise<void>(resolve =>
  requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));

test("shared snapshots reset across login and backend changes, including held A-B-A replies", async ({ page }, info) => {
  const f = await settingsAccountFixture(page, "diagnostics");
  await page.clock.install({ time: NOW });
  const seeds = new Map(await Promise.all(paths.map(async path => [path,
    await (await page.request.get(API + path)).json()] as const)));
  let phase = 0, hold = false, fail = false;
  const pending: { path: string; auth: string; release: () => void; done: Promise<void> }[] = [];
  const reads: { path: string; auth: string }[] = [];
  await page.route("**/api/**", async route => {
    const req = route.request(), url = new URL(req.url());
    if (url.origin !== API) return route.abort();
    const path = url.pathname.replace("/e2e-shared", "");
    if (req.method() !== "GET" || !paths.includes(path)) {
      if (url.pathname.startsWith("/e2e-shared/")) {
        if (req.method() !== "GET" && path !== "/api/auth/ws-ticket") return route.abort();
        return route.fulfill({ response: await route.fetch({ url: req.url().replace("/e2e-shared", "") }) });
      }
      return route.fallback();
    }
    const auth = req.headers().authorization ?? "";
    reads.push({ path: url.pathname, auth });
    const data = structuredClone(seeds.get(path));
    const old = phase === 0;
    if (path === paths[0]) Object.assign(data.data, {
      risk: old ? "block" : "ok", netDeltaUsd: old ? 111 : phase * 111,
      // A newly selected backend may have an older clock than the previous one.
      updatedAtMs: NOW - phase * 1_000,
    });
    if (path === paths[1]) {
      data.generatedAtMs = NOW - phase * 1_000;
      data.rows = old ? [{ venue: "fixture", operation: "background_task:OLD_LOGIN", configured: true,
        status: "blocked", source: "task_registry", message: "OLD_LOGIN_TASK", observedAtMs: NOW }] : [];
    }
    if (path === paths[2]) Object.assign(data, { environment: old ? "live" : "paper" });
    const failed = fail;
    let finish: (() => void) | undefined;
    if (hold) {
      const done = new Promise<void>(resolve => { finish = resolve; });
      await new Promise<void>(release => pending.push({ path, auth, release, done }));
    }
    try {
      await route.fulfill(failed ? { status: 503, json: { error: {
        code: "SHARED_READ_FAILED", message: "fixture shared state unavailable", retryAfterMs: 60_000,
      } } } : { json: data });
    } finally { finish?.(); }
  });
  await page.goto("/#settings");
  const summary = page.locator(".status-summary");
  await expect(summary).toContainText("风险已阻断");
  await expect(mode(page)).toContainText("实盘");
  hold = true;
  await page.clock.runFor(10_100);
  await expect.poll(() => new Set(pending.map(row => row.path)).size).toBe(3);
  const token = async (value: string) => {
    await page.getByRole("tab", { name: "连接", exact: true }).click();
    await page.locator(".settings-api-token-task input").fill(value);
    await page.getByRole("button", { name: "保存 Token", exact: true }).click();
  };
  await token("isolated-other-token");
  await expect.poll(() => new Set(pending.filter(row => row.auth === "Bearer isolated-other-token").map(row => row.path)).size).toBe(3);
  await expect(mode(page)).toHaveText("环境-");
  await expect(summary).not.toContainText("风险已阻断");
  await summary.click();
  await expect(page.getByRole("group", { name: "风险与资金状态", exact: true })).not.toContainText("$111");
  await expect(page.getByRole("group", { name: "系统状态详情", exact: true })).not.toContainText("OLD_LOGIN_TASK");
  await summary.click();
  hold = false; phase = 2;
  await token(original);
  await expect(mode(page)).toContainText("模拟");
  await summary.click();
  const exposure = page.getByRole("group", { name: "风险与资金状态", exact: true });
  await expect(exposure).toContainText("$222");
  pending.forEach(row => row.release());
  await Promise.all(pending.map(row => row.done));
  await frame(page);
  await expect(exposure).toContainText("$222");
  await expect(mode(page)).toContainText("模拟");
  await expect(summary).not.toContainText("风险已阻断");
  await expect(page.getByRole("group", { name: "系统状态详情", exact: true })).not.toContainText("OLD_LOGIN_TASK");
  await summary.click();

  fail = true;
  await page.clock.runFor(10_100);
  await expect(mode(page)).toHaveAttribute("title", /SHARED_READ_FAILED/);
  await expect(summary).toContainText("系统数据待确认");
  phase = 3; fail = false;
  await page.getByRole("textbox", { name: "API Base", exact: true }).fill(API + "/e2e-shared");
  await page.getByRole("textbox", { name: "确认应用", exact: true }).fill("apply");
  await page.getByRole("button", { name: "保存并应用", exact: true }).click();
  await expect.poll(() => new Set(reads.filter(row => row.path.startsWith("/e2e-shared/")).map(row => row.path)).size).toBe(3);
  await expect(mode(page)).not.toHaveClass(/degraded/);
  await summary.click();
  await expect(exposure).toContainText("$333");
  await expect(page.getByRole("group", { name: "系统快照状态", exact: true })).toHaveCount(0);
  await page.screenshot({ path: info.outputPath("shared-connection-desktop.png") });
  expect(reads.filter(row => row.path.startsWith("/e2e-shared/")).every(row => row.auth === `Bearer ${original}`)).toBe(true);
  expect(f.calls.filter(row => row.key.startsWith("POST "))).toEqual([]);
  expect(f.writes).toEqual([]); expect(f.errors).toEqual([]);
});

test("silent and repeated snapshots expire despite connected WS, then fresh evidence recovers", async ({ page }, info) => {
  const f = await setup(page);
  await page.clock.install({ time: NOW });
  const system = await (await page.request.get(API + paths[0])).json();
  const operations = await (await page.request.get(API + paths[1])).json();
  const trading = await (await page.request.get(API + paths[2])).json();
  let revision = 0, hold = false, requests = 0;
  const pending: { release: () => void; done: Promise<void> }[] = [];
  await page.route("**/api/**", async route => {
    const path = new URL(route.request().url()).pathname;
    if (route.request().method() !== "GET" || !paths.includes(path)) return route.fallback();
    requests++;
    const data = structuredClone(path === paths[0] ? system : path === paths[1] ? operations : trading);
    if (path === paths[0]) Object.assign(data.data, { updatedAtMs: NOW + revision, risk: "ok", netDeltaUsd: 456 });
    if (path === paths[1]) data.generatedAtMs = NOW + revision;
    let finish: (() => void) | undefined;
    if (hold && path !== paths[1]) {
      const done = new Promise<void>(resolve => { finish = resolve; });
      await new Promise<void>(release => pending.push({ release, done }));
    }
    try { await route.fulfill({ json: data }); } finally { finish?.(); }
  });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/#futures");
  const summary = page.locator(".status-summary");
  const evidence = page.getByRole("group", { name: "系统快照状态", exact: true });
  await expect(summary).toHaveAttribute("data-state", "healthy");
  await expect.poll(() => f.channelSockets.get("system")?.size ?? 0).toBeGreaterThan(0);
  const send = (version: number, risk = "ok") => {
    for (const socket of f.channelSockets.get("system")!) socket.send(JSON.stringify({ type: "message", channel: "system",
      payload: { ...system.data, updatedAtMs: NOW + version, risk, netDeltaUsd: 456 } }));
  };
  send(1, "block");
  await expect(summary).toContainText("风险已阻断");
  hold = true;
  await page.clock.runFor(8_000);
  send(1, "ok"); // Same version cannot silently remove the previous risk block.
  await expect(summary).toContainText("风险已阻断");
  await page.clock.runFor(8_100);
  await summary.click();
  await expect(evidence).toContainText("SYSTEM_HEALTH_STALE");
  await expect(summary).toContainText("仅供参考");
  await expect(mode(page)).toHaveAttribute("title", /TRADING_STATUS_STALE/);
  await expect(page.getByTestId("status-api-runtime")).toHaveAttribute("title", /OPERATION_HEALTH_STALE/);
  await expect(page.getByRole("group", { name: "风险与资金状态", exact: true })).toContainText("$456");
  send(0);
  await expect(evidence).toContainText("SYSTEM_HEALTH_UNCONFIRMED");
  await expect(summary).toContainText("风险已阻断");
  await page.screenshot({ path: info.outputPath("shared-stale-desktop.png") });
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(evidence).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
  await page.screenshot({ path: info.outputPath("shared-stale-mobile.png") });

  // Valid new WS data recovers independently of still-held fallback requests.
  revision = 2;
  send(revision);
  await expect(evidence).toHaveCount(0);
  pending.forEach(row => row.release());
  await Promise.all(pending.map(row => row.done));
  await frame(page);
  await expect(evidence).toHaveCount(0);
  hold = false;
  await page.clock.runFor(5_100);
  f.tick(21_200);
  await expect(summary).toHaveAttribute("data-state", "healthy");
  await expect(mode(page)).not.toHaveClass(/degraded/);
  await expect(page.getByTestId("status-api-runtime")).not.toHaveAttribute("title", /OPERATION_HEALTH_STALE/);
  // Clock rollback and duplicate frames cannot give the snapshot a new lease.
  await page.clock.setSystemTime(NOW - 60_000);
  await page.clock.runFor(8_000);
  send(revision);
  await page.clock.runFor(8_100);
  await expect(evidence).toContainText("SYSTEM_HEALTH_STALE");
  expect(requests).toBeLessThan(45);
  expect(f.writes).toEqual([]); expect(f.errors).toEqual([]);
});
