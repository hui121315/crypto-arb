import { expect, test } from "@playwright/test";

const API = "http://127.0.0.1:18000";
const WEB = "http://127.0.0.1:18080";
const headers = { Authorization: "Bearer isolated-paper-browser" };

test("partial compensation cancels only the remainder, retains fills and cannot retry the full quantity", async ({ page, request }) => {
  const incidents = async () => {
    const response = await request.get(`${API}/api/trading/portfolio/snapshot`, { headers });
    expect(response.status()).toBe(200);
    return (await response.json()).snapshot?.recentCloseRuns ?? [];
  };
  const find = async (id: string) => (await incidents()).find((row: any) => row.id === id);
  await expect.poll(async () => (await incidents()).length).toBe(3);
  const retryAction = (run: any) => run.unwindPlan.nextActions.some((a: any) => a.kind === "submit_compensation_order");
  expect(retryAction(await find("paper-close-zero"))).toBe(true);
  expect(retryAction(await find("paper-close-unknown"))).toBe(false);

  const errors: string[] = [], unexpected: string[] = [], writes: string[] = [];
  await page.addInitScript((api) => {
    localStorage.setItem("api_base", JSON.stringify(api));
    localStorage.setItem("api_auth_token", JSON.stringify("isolated-paper-browser"));
  }, API);
  page.on("pageerror", error => errors.push(error.stack ?? error.message));
  await page.route("**/*", route => {
    const req = route.request(), url = new URL(req.url());
    if (![API, WEB].includes(url.origin)) { unexpected.push(req.url()); return route.abort(); }
    if (!["GET", "HEAD"].includes(req.method())) {
      if (url.pathname === "/api/trading/orders/paper-partial-compensation/cancel"
        || url.pathname === "/api/trading/portfolio/close-runs/paper-close-zero/compensation-orders") {
        writes.push(url.pathname);
      } else if (url.pathname !== "/api/auth/ws-ticket") {
        unexpected.push(`${req.method()} ${url.pathname}`); return route.abort();
      }
    }
    return route.continue();
  });
  await page.routeWebSocket(/.*/, socket => {
    if (new URL(socket.url()).host === "127.0.0.1:18000") socket.connectToServer();
    else { unexpected.push(socket.url()); socket.close(); }
  });
  await page.goto("/#positions");
  await page.getByRole("tab", { name: "平仓", exact: true }).click();
  const incident = page.locator('.close-incident[data-run-id="paper-close-partial"]');
  await incident.locator("summary").click();
  const progress = incident.getByLabel("补偿订单进度");
  await expect(progress).toContainText("部分成交");
  await expect(progress).toContainText("已成交 0.4 / 目标 1 · 未完成 0.6");
  await expect(incident.getByRole("button", { name: "撤补买", exact: true })).toBeEnabled();
  await page.screenshot({ path: test.info().outputPath("partial-compensation-desktop.png"), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  const cancel = incident.getByRole("button", { name: "撤补买", exact: true });
  await cancel.scrollIntoViewIfNeeded();
  await cancel.click({ trial: true });
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(390);
  await page.screenshot({ path: test.info().outputPath("partial-compensation-mobile.png"), fullPage: true });
  await cancel.click();
  await expect.poll(async () => (await find("paper-close-partial"))?.status).toBe("compensation_failed");
  await expect(progress).toContainText("已取消未成交部分");
  await expect(progress).toContainText("已成交 0.4 / 目标 1 · 未完成 0.6");
  await expect(incident.getByRole("button", { name: "撤补买", exact: true })).toHaveCount(0);
  await expect(incident).toContainText("禁止按原数量整笔重试");
  expect(writes).toHaveLength(1);
  const terminal = await find("paper-close-partial");
  expect(terminal.unwindPlan.compensationAttempts[0].order.filledQuantity).toBe(0.4);
  expect(terminal.legs.map((leg: any) => leg.order.filledQuantity)).toEqual([1, 0]);
  expect(retryAction(terminal)).toBe(false);

  // Exercise the real server guard as well as the absence of a frontend retry button.
  const rejected = await request.post(`${API}/api/trading/portfolio/close-runs/paper-close-partial/compensation-orders`, {
    headers: { ...headers, "Idempotency-Key": "paper-reject-full-retry" },
    data: { confirmationPhrase: "COMPENSATE_CLOSE_RUN", snapshotVersion: terminal.snapshotVersion,
      candidateIndex: 0, targetQuantity: 1, limitPrice: 100 },
  });
  expect(rejected.status()).toBe(409);
  expect((await find("paper-close-partial")).unwindPlan.compensationAttempts).toHaveLength(1);
  await page.reload();
  await page.getByRole("tab", { name: "平仓", exact: true }).click();
  await incident.locator("summary").click();
  await expect(progress).toContainText("已成交 0.4");
  await expect(incident.getByRole("button", { name: "撤补买", exact: true })).toHaveCount(0);
  const zero = page.locator('.close-incident[data-run-id="paper-close-zero"]');
  await zero.locator("summary").click();
  await zero.getByLabel("补偿确认短语", { exact: true }).fill("COMPENSATE_CLOSE_RUN");
  await zero.getByRole("button", { name: /重试.*#1/ }).click();
  await expect.poll(async () => (await find("paper-close-zero"))?.status).toBe("compensated");
  const retried = await find("paper-close-zero");
  expect(retried.unwindPlan.compensationAttempts).toHaveLength(2);
  expect(retried.unwindPlan.compensationAttempts[1].order.filledQuantity).toBe(1);
  expect(retried.unwindPlan.compensationAttempts[1].confirmedFilledAtMs).toBeGreaterThan(0);
  await expect(zero).toHaveCount(0);
  await expect.poll(() => page.evaluate(() => Object.keys(sessionStorage)
    .filter(key => key.startsWith("crossline.settings.pending.v1:position-remedy:")))).toEqual([]);
  await expect(page.getByRole("alert", { name: "补偿 / 人工终结", exact: true })).toHaveCount(0);
  await expect(page.getByRole("region", { name: "平仓记录", exact: true }))
    .toContainText("平仓事故补偿已完成：1 条补偿订单已确认成交");
  const review = page.waitForRequest(req => {
    const url = new URL(req.url());
    return url.pathname === "/api/review/executed" && url.searchParams.get("closeRunId") === "paper-close-partial";
  });
  await incident.getByRole("link", { name: "关联复盘", exact: true }).click();
  await review;
  await expect(page.locator('.review-record-scope[role="status"]')).toContainText("paper-close-partial");
  // No original opening execution was seeded: do not substitute another trade.
  await expect(page.locator('.review-record-scope[role="status"]')).toContainText("未找到可核对的关联记录");
  expect(errors).toEqual([]); expect(unexpected).toEqual([]); expect(writes).toHaveLength(2);
});
