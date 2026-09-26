import { expect, test } from "@playwright/test";

test("unavailable real history is an error, not zero trades or zero profit", async ({ page, request }, info) => {
  const api = "http://127.0.0.1:18000";
  const headers = { Authorization: "Bearer isolated-paper-browser" };
  const writes: string[] = [], errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.addInitScript(api => {
    localStorage.setItem("api_base", JSON.stringify(api));
    localStorage.setItem("api_auth_token", JSON.stringify("isolated-paper-browser"));
  }, api);
  await page.route("**/*", route => {
    const req = route.request(), url = new URL(req.url());
    if (![api, "http://127.0.0.1:18080"].includes(url.origin) ||
        (!["GET", "HEAD"].includes(req.method()) && url.pathname !== "/api/auth/ws-ticket")) {
      writes.push(`${req.method()} ${req.url()}`); return route.abort();
    }
    return route.continue();
  });
  const response = await request.get(`${api}/api/review/executed?days=1&limit=1`, { headers });
  expect(response.ok()).toBe(true);
  const envelope = await response.json();
  expect(envelope.status).toBe("degraded");
  expect(envelope.rows).toEqual([]);
  expect(envelope.missingFields).toContain("net");
  expect(envelope.problems[0].code).toBe("REVIEW_HISTORY_READ_FAILED");
  expect(JSON.stringify(envelope)).not.toContain("postgres://");
  await page.goto("/#review");
  await expect(page.locator(".review-state-disclosure summary")).toContainText("历史读取失败");
  await expect(page.locator(".review-executed-table")).toContainText("读取失败");
  await expect(page.locator(".review-executed-summary")).toHaveCount(0);
  await page.getByRole("tab", { name: /策略表现/ }).click();
  await expect(page.locator("#review-panel-strategy .review-business-empty")).toContainText("读取失败");
  await expect(page.locator(".review-strategy-table")).toHaveCount(0);
  await page.getByRole("button", { name: "刷新复盘记录", exact: true }).click();
  await expect(page.locator(".review-state-disclosure summary")).toContainText("历史读取失败");
  const body = page.locator("#review-panel-strategy .review-business-empty");
  await expect(body).not.toContainText("request_id");
  await expect(body).not.toContainText("retry");
  await page.locator(".review-state-disclosure summary").click();
  await expect(page.locator(".review-state-disclosure p")).toContainText("retry 5000ms");
  await page.locator(".review-state-disclosure summary").click();
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    await page.screenshot({ path: info.outputPath(`review-history-unavailable-${width}.png`) });
  }
  const portfolio = (await (await request.get(`${api}/api/trading/portfolio/snapshot`, { headers })).json()).snapshot;
  expect(portfolio.summary.pnlBreakdown.evidence.quality).toBe("missing");
  expect(portfolio.summary.pnlBreakdown.evidence.problem.code).toBe("REVIEW_HISTORY_READ_FAILED");
  await page.goto("/#positions");
  const pnl = page.locator(".summary-card").filter({ hasText: "今日已结算盈亏" });
  await expect(pnl.locator(":scope > strong")).toHaveText("未知");
  expect(writes).toEqual([]); expect(errors).toEqual([]);
});
