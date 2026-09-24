import { expect, test } from "@playwright/test";

const API = "http://127.0.0.1:18000";
const WEB = "http://127.0.0.1:18080";

test("paper opportunity opens a pair, closes both legs and reaches its exact review", async ({ page, request }) => {
  const errors: string[] = [], unexpected: string[] = [];
  const headers = { Authorization: "Bearer isolated-paper-browser" };
  const status = await (await request.get(`${API}/api/trading/status`, { headers })).json();
  expect(status.environment).toBe("paper"); expect(status.adapter).toBe("mock");
  await page.addInitScript((api) => {
    localStorage.setItem("api_base", JSON.stringify(api));
    localStorage.setItem("api_auth_token", JSON.stringify("isolated-paper-browser"));
  }, API);
  page.on("pageerror", (error) => errors.push(error.message));
  await page.route("**/*", (route) => {
    const req = route.request(), url = new URL(req.url());
    if (![API, WEB].includes(url.origin)) { unexpected.push(req.url()); return route.abort(); }
    if (!["GET", "HEAD"].includes(req.method()) && !(
      url.pathname === "/api/auth/ws-ticket" || /\/api\/arbitrage\/opportunities\/[^/]+\/(preview|confirm)$/.test(url.pathname)
      || /^\/api\/automation\/execution-artifacts\/(build|validate)$/.test(url.pathname)
      || /^\/api\/trading\/portfolio\/positions\/[^/]+\/[^/]+\/close-pair$/.test(url.pathname)
    )) { unexpected.push(`${req.method()} ${url.pathname}`); return route.abort(); }
    return route.continue();
  });
  await page.goto("/#futures");
  await page.getByRole("button", { name: "构建新双腿", exact: true }).click();
  const previewResponse = page.waitForResponse((response) => response.url().endsWith("/preview")
    && response.request().postDataJSON()?.capitalUsd === 10);
  await page.getByRole("textbox", { name: "计划本金 USD", exact: true }).fill("10");
  const preview = await (await previewResponse).json();
  const artifact = page.locator(".execution-artifact");
  await expect(artifact.locator(".execution-artifact-status")).toContainText("待校验");
  await expect(artifact.locator(".execution-artifact-identifiers")).toContainText(preview.ticket.ticketId);
  await artifact.getByRole("button", { name: "校验票据" }).click();
  await artifact.getByRole("checkbox").check();
  await page.locator(".confirm-action.primary").click();
  await page.getByRole("link", { name: "去持仓平仓", exact: true }).click();
  const scope = page.locator(".positions-run-scope");
  await expect(scope).toBeVisible();
  const runId = new URLSearchParams(new URL(page.url()).hash.split("?")[1]).get("run");
  expect(runId).toBeTruthy();
  const closeButtons = page.locator(".positions-table .row-close-button");
  await expect(closeButtons).toHaveCount(2);
  await closeButtons.first().click();
  const confirmation = page.getByRole("group", { name: /^确认平配对：/ });
  await expect(confirmation).toHaveCount(1);
  await expect(closeButtons.last()).toHaveAttribute("aria-expanded", "false");
  await confirmation.getByRole("button", { name: "取消", exact: true }).press("Escape");
  await expect(closeButtons.first()).toBeFocused();
  await closeButtons.last().click();
  await expect(confirmation).toHaveCount(1);
  await expect(closeButtons.first()).toHaveAttribute("aria-expanded", "false");
  expect(await closeButtons.first().getAttribute("aria-controls")).not.toBe(await closeButtons.last().getAttribute("aria-controls"));
  await page.setViewportSize({ width: 390, height: 844 });
  await confirmation.getByRole("button", { name: "模拟平配对", exact: true }).click({ trial: true });
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(390);
  await page.screenshot({ path: test.info().outputPath("paper-close-mobile.png"), fullPage: true });
  const close = page.waitForResponse((response) => response.url().endsWith("/close-pair"));
  await page.getByRole("button", { name: "模拟平配对", exact: true }).click();
  const closed = await (await close).json();
  expect(closed.status).toBe("succeeded");
  await expect(page.locator(".positions-table .row-close-button")).toHaveCount(0);
  await page.getByRole("tab", { name: "平仓", exact: true }).click();
  await page.locator(".position-history-record summary").first().click();
  await page.getByRole("link", { name: "关联复盘", exact: true }).click();
  await expect(page.locator(".review-record-scope")).toContainText(closed.id);
  await expect(page.locator(".review-page")).toContainText("平仓");
  const review = await (await request.get(`${API}/api/review/executed?runId=${encodeURIComponent(runId!)}&days=365`, { headers })).json();
  expect(review.rows.length).toBeGreaterThan(0);
  expect(review.rows.some((row: any) => row.evidence?.closeRunEvidence?.some((e: any) => e.runId === runId))).toBe(true);
  expect(errors).toEqual([]); expect(unexpected).toEqual([]);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.screenshot({ path: test.info().outputPath("paper-cycle-review.png"), fullPage: true });
});
