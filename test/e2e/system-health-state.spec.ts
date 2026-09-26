import { expect, test, type WebSocketRoute } from "@playwright/test";
import { API, NOW, setup } from "./fixtures/opportunity-workbench";

const problem = { code: "SYSTEM_HEALTH_FAILED", message: "fixture: system snapshot unavailable", retryAfterMs: 60_000 };
const failure = { status: 503, json: { error: problem } };

function publish(sockets: Set<WebSocketRoute>, payload: unknown) {
  expect(sockets.size).toBeGreaterThan(0);
  for (const socket of sockets) socket.send(JSON.stringify(payload));
}

test("system read failures stay visible with retained values and risk warnings until recovery", async ({ page }, info) => {
  const fixture = await setup(page);
  const envelope = await (await page.request.get(API + "/api/system/health")).json();
  await page.route(API + "/api/system/health", route => route.fulfill(failure));
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/#futures");
  const summary = page.locator(".status-summary");
  const evidence = page.getByRole("group", { name: "系统快照状态", exact: true });
  const risk = page.getByRole("button", { name: /^Risk/ });
  const exposure = page.getByRole("group", { name: "风险与资金状态", exact: true });
  await expect(summary).toContainText("系统数据读取失败");
  await summary.click();
  await expect(evidence).toContainText("尚无可用的风险与资金快照");
  await expect(evidence).toContainText(problem.code);
  await expect(risk).toContainText("错误");
  const sockets = fixture.channelSockets.get("system")!;
  let revision = 0;
  const sendHealth = (risk: string) => publish(sockets, { type: "message", channel: "system",
    payload: { ...envelope.data, risk, netDeltaUsd: 123, netDeltaPctOfNav: 1.5, updatedAtMs: NOW + ++revision } });
  const sendError = () => publish(sockets, { type: "error", channel: "system", ...problem });
  sendHealth("ok");
  await expect(summary).toHaveAttribute("data-state", "healthy");
  await expect(evidence).toHaveCount(0);
  await expect(exposure).toContainText("$123 (+1.5%)");
  sendError();
  await expect(summary).toContainText("系统数据待确认");
  await expect(evidence).toContainText("仅供参考");
  await expect(exposure).toContainText("$123 (+1.5%)");
  await expect(risk).toHaveClass(/degraded/);
  await expect(risk).toHaveAttribute("title", new RegExp(problem.code));
  const delta = exposure.locator(".slot").filter({ has: page.getByText("Delta", { exact: true }) });
  await expect(delta).toHaveClass(/degraded/);
  await page.screenshot({ path: info.outputPath("retained-system-desktop.png") });

  for (const [riskValue, label, state] of [["block", "风险已阻断", "blocked"], ["warn", "风险警告", "warning"]]) {
    sendHealth(riskValue);
    await expect(summary).toContainText(label);
    sendError();
    await expect(evidence).toContainText(problem.code);
    await expect(summary).toHaveAttribute("data-state", state);
    await expect(summary).toContainText(label);
    await expect(summary).toContainText("仅供参考");
  }
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(evidence).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth - innerWidth)).toBeLessThanOrEqual(1);
  const rect = await evidence.boundingBox();
  expect(rect!.x).toBeGreaterThanOrEqual(0);
  expect(rect!.x + rect!.width).toBeLessThanOrEqual(391);
  await page.screenshot({ path: info.outputPath("retained-system-mobile.png") });
  sendHealth("ok");
  await expect(summary).toHaveAttribute("data-state", "healthy");
  await expect(evidence).toHaveCount(0);
  await expect(risk).not.toHaveClass(/degraded/);
  await expect(delta).not.toHaveClass(/degraded/);
  expect(fixture.writes).toEqual([]);
  expect(fixture.errors).toEqual([]);
});

test("late fallback responses cannot roll back WS state and degraded envelopes never look healthy", async ({ page }) => {
  const fixture = await setup(page);
  const envelope = await (await page.request.get(API + "/api/system/health")).json();
  let release: (() => void) | undefined;
  let held = false;
  let response = { status: 200, json: envelope };
  await page.route(API + "/api/system/health", async route => {
    held = true;
    await new Promise<void>(resolve => { release = resolve; });
    return route.fulfill(response);
  });
  const summary = page.locator(".status-summary");
  for (const lateError of [false, true]) {
    held = false;
    if (lateError) response = failure;
    if (lateError) await page.reload();
    else await page.goto("/#futures");
    await expect.poll(() => held).toBe(true);
    await expect.poll(() => fixture.channelSockets.get("system")?.size ?? 0).toBeGreaterThan(0);
    const sockets = fixture.channelSockets.get("system")!;
    publish(sockets, { type: "message", channel: "system",
      payload: { ...envelope.data, risk: lateError ? "ok" : "block", updatedAtMs: NOW + 100 } });
    await expect(summary).toHaveAttribute("data-state", lateError ? "healthy" : "blocked");
    const returned = page.waitForResponse(API + "/api/system/health");
    release!();
    await (await returned).finished();
    await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
    await expect(summary).toHaveAttribute("data-state", lateError ? "healthy" : "blocked");
    await expect(page.getByRole("group", { name: "系统快照状态", exact: true })).toHaveCount(0);
  }
  await page.unroute(API + "/api/system/health");
  await page.route(API + "/api/system/health", route => route.fulfill({ json: {
    ...envelope, status: "partial", problems: [problem],
  } }));
  await page.reload();
  await expect(summary).toContainText("系统数据待确认");
  await expect(summary).toHaveAttribute("data-state", "degraded");
  await summary.click();
  await expect(page.getByRole("group", { name: "系统快照状态", exact: true })).toContainText(problem.code);
  expect(fixture.writes).toEqual([]);
  expect(fixture.errors).toEqual([]);
});
