import { expect, test } from "@playwright/test";

const API = "http://127.0.0.1:18000";
const WEB = "http://127.0.0.1:18080";
const headers = { Authorization: "Bearer isolated-paper-browser" };

test("paper account replacement rejects old confirmation and validation without placing orders", async ({ page, request }) => {
  const errors: string[] = [], unexpected: string[] = [];
  let confirms = 0;
  await page.addInitScript(api => {
    localStorage.setItem("api_base", JSON.stringify(api));
    localStorage.setItem("api_auth_token", JSON.stringify("isolated-paper-browser"));
  }, API);
  page.on("pageerror", error => errors.push(error.message));
  await page.route("**/*", route => {
    const req = route.request(), url = new URL(req.url());
    const allowed = url.pathname === "/api/auth/ws-ticket"
      || /^\/api\/arbitrage\/opportunities\/[^/]+\/(preview|confirm)$/.test(url.pathname)
      || /^\/api\/automation\/execution-artifacts\/(build|validate)$/.test(url.pathname);
    if (![API, WEB].includes(url.origin) || (!["GET", "HEAD"].includes(req.method()) && !allowed)) {
      unexpected.push(`${req.method()} ${url.origin}${url.pathname}`); return route.abort();
    }
    if (url.pathname.endsWith("/confirm")) confirms++;
    return route.continue();
  });
  await page.goto("/#futures");
  await page.getByRole("button", { name: "构建新双腿", exact: true }).click();
  const built = page.waitForResponse(r => r.url().endsWith("/preview") && r.request().postDataJSON()?.capitalUsd === 10);
  await page.getByRole("textbox", { name: "计划本金 USD", exact: true }).fill("10");
  const preview = await (await built).json();
  expect(preview.executionBinding.adapter).toBe("mock");
  expect(preview.executionBinding.mode).toBe("dry_run");
  const artifact = page.locator(".execution-artifact");
  await expect(artifact.locator(".execution-artifact-identifiers")).toContainText(preview.ticket.ticketId);
  await artifact.getByRole("button", { name: "校验票据", exact: true }).click();
  await artifact.getByRole("checkbox").check();
  // The real settings route replaces the paper account. No credentials or live adapter exist here.
  for (const suffix of ["a", "b"]) {
    const selected = await request.post(`${API}/api/trading/adapters/select`, {
      headers: { ...headers, "Idempotency-Key": `paper-context-${suffix}` }, data: { adapterId: "mock" },
    });
    expect(selected.ok()).toBe(true);
  }
  const rejected = page.waitForResponse(r => r.url().endsWith("/confirm"));
  await page.locator(".confirm-action.primary").click();
  const response = await rejected;
  expect(response.status()).toBe(409);
  const problem = (await response.json()).error;
  expect(problem.code).toBe("HEDGE_EXECUTION_CONTEXT_CHANGED");
  expect(problem.details.confirmContext.ticketId).toBe(preview.ticket.ticketId);
  expect(problem.details.current.accountEpoch).toBeGreaterThan(preview.executionBinding.accountEpoch);
  await expect(page.locator(".execution-page")).toContainText("执行账户或环境已改变");
  await expect(page.getByRole("button", { name: "查询提交结果", exact: true })).toHaveCount(0);
  await expect(page.locator(".confirm-action.primary")).toBeDisabled();
  await expect(artifact.locator(".execution-artifact-status")).toContainText("校验未通过");
  await expect(page.getByTestId("top-status-bar")).toContainText("账户已改变，待重建");
  await expect(page.getByTestId("top-status-bar")).not.toContainText("读取失败");
  await expect(artifact.getByRole("checkbox")).not.toBeChecked();
  const orders = await (await request.get(`${API}/api/trading/orders`, { headers })).json();
  expect(orders.rows.filter((row: any) => [preview.longLeg.id, preview.shortLeg.id].includes(row.intent.id))).toHaveLength(0);
  // Revalidation of the same artifact must also describe it as blocked, never ready.
  const validation = page.waitForResponse(r => r.url().endsWith("/execution-artifacts/validate"));
  await artifact.getByRole("button", { name: "校验票据", exact: true }).click();
  const result = await (await validation).json();
  expect(result.valid).toBe(false); expect(result.status).toBe("blocked");
  expect(result.blockers.join(" ")).toContain("执行账户或环境已改变");
  await expect(artifact.locator(".execution-artifact-status")).toContainText("校验未通过");
  await expect(page.locator(".confirm-action.primary")).toBeDisabled();
  await page.screenshot({ path: test.info().outputPath("account-changed.png"), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.getByRole("button", { name: "刷新预览", exact: true }).scrollIntoViewIfNeeded();
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(390);
  await page.screenshot({ path: test.info().outputPath("account-changed-mobile.png") });
  const rebuilt = page.waitForResponse(r => r.url().endsWith("/preview") && r.ok());
  await page.getByRole("button", { name: "刷新预览", exact: true }).click();
  const current = await (await rebuilt).json();
  expect(current.ticket.ticketId).not.toBe(preview.ticket.ticketId);
  expect(current.executionBinding.accountEpoch).toBe(problem.details.current.accountEpoch);
  await expect(artifact.locator(".execution-artifact-status")).toContainText("待校验");
  expect(confirms).toBe(1);
  expect(errors).toEqual([]); expect(unexpected).toEqual([]);
});
