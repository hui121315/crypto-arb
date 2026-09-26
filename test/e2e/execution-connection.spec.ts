import { expect, test, type Page } from "@playwright/test";
import { submissionFixture, reviewAndSubmit, confirmRecoveryKey } from "./fixtures/execution-submission";
import { API, NOW } from "./fixtures/opportunity-workbench";

const original = "isolated-fixture-token";
const other = "different-fixture-login";
async function login(page: Page, token: string) {
  await page.getByRole("button", { name: "切换到设置", exact: true }).click();
  await page.getByRole("tab", { name: "诊断", exact: true }).click();
  await page.getByRole("tab", { name: "连接", exact: true }).click();
  await page.locator(".settings-api-token-task input").fill(token);
  await page.getByRole("button", { name: "保存 Token", exact: true }).click();
}
const execution = (page: Page) => page.getByRole("button", { name: /^切换到对冲执行/ }).click();
const saved = (page: Page, key = confirmRecoveryKey()) => page.evaluate(key => localStorage.getItem(key), key);

test("connection switch isolates pending submit, late receipts, and interrupted cancel across A-B-A", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const f = await submissionFixture(page);
  const requests: { path: string; auth: string; method: string }[] = [];
  await page.route("**/api/**", route => {
    const request = route.request(), url = new URL(request.url());
    const auth = request.headers().authorization ?? "";
    requests.push({ path: url.pathname + url.search, auth, method: request.method() });
    if (auth === `Bearer ${other}` && ["/api/trading/orders", "/api/trading/execution-runs"].includes(url.pathname)) {
      return route.fulfill({ json: { status: "fresh", rows: [], problems: [], observedAtMs: NOW,
        page: { returnedCount: 0, totalRows: 0, pageSize: 50, hasNextPage: false } } });
    }
    return route.fallback();
  });
  f.holdConfirm();
  await reviewAndSubmit(page);
  await expect.poll(() => f.confirms.length).toBe(1);
  const pending = await saved(page);
  expect(pending).not.toBeNull();
  expect(pending).not.toContain(original);
  await login(page, other);
  const response = page.waitForResponse("**/confirm");
  f.releaseConfirm();
  await (await response).finished();
  await execution(page);
  const blocked = page.locator(".execution-recovery");
  await expect(blocked).toContainText("连接已改变");
  await expect(page.getByTestId("top-status-bar")).toContainText("连接已改变，待刷新");
  await expect(page.getByText(/execution run seed returned no row/)).toHaveCount(0);
  await expect(page.locator(".execution-ticket, .execution-order-queue")).toHaveCount(0);
  expect(await saved(page)).toBe(pending);
  expect(await saved(page, confirmRecoveryKey(other))).toBeNull();
  await page.screenshot({ path: test.info().outputPath("connection-desktop.png"), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  const reload = page.getByRole("button", { name: "刷新当前连接", exact: true });
  await reload.scrollIntoViewIfNeeded();
  expect(await reload.evaluate(el => {
    const r = el.getBoundingClientRect();
    return el.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2));
  })).toBe(true);
  expect(await blocked.evaluate(el => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
  await page.screenshot({ path: test.info().outputPath("connection-mobile.png") });
  await page.setViewportSize({ width: 1440, height: 900 });
  await login(page, original);
  await execution(page);
  await expect(blocked).toContainText("连接已改变");
  await reload.click();
  await expect(page.locator(".execution-page")).toContainText("等待交易所成交");
  await expect.poll(() => saved(page)).toBeNull();
  expect(f.confirms).toHaveLength(1);

  // Complete the original run before explicitly submitting a distinct second ticket.
  f.emitRun(f.makeRun(undefined, "closed", NOW + 30));
  await expect(page.locator(".execution-page")).toContainText("执行已收口");
  await page.getByRole("button", { name: "切换到期货套利", exact: true }).click();
  await page.getByRole("button", { name: "构建新双腿", exact: true }).click();
  f.setRuns([]);
  await page.getByRole("button", { name: "校验票据" }).click();
  await page.locator(".execution-artifact").getByRole("checkbox").check();
  await page.locator(".confirm-action.primary").click();
  await expect(page.locator(".execution-actionbar")).toContainText("等待交易所成交");
  expect(f.confirms).toHaveLength(2);
  f.holdCancel();
  await page.getByRole("button", { name: "撤单", exact: true }).click();
  await expect.poll(() => f.cancels.length).toBe(1);
  const first = f.cancels[0];
  await login(page, other);
  f.releaseCancel();
  await execution(page);
  await expect(blocked).toContainText("连接已改变");
  await reload.click();
  await expect(page.locator(".execution-idle")).toBeVisible();
  await expect(page.getByRole("region", { name: "原撤单核对" })).toHaveCount(0);
  expect(await page.locator(".execution-page").innerText()).not.toContain("原提交结果待核对");
  expect(requests.filter(r => r.auth === `Bearer ${other}` && r.path.includes("execution-runs?")).length).toBe(0);
  expect(f.cancels).toEqual([first]);
  await login(page, original);
  await execution(page);
  await reload.click();
  await expect(page.getByRole("region", { name: "原撤单核对" })).toContainText("1 笔未发送");
  expect(f.cancels).toEqual([first]);
  expect(requests.filter(r => /\/(confirm|cancel)$/.test(r.path)).every(r => r.auth === `Bearer ${original}`)).toBe(true);
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});

test("storage denial sends no order and legacy recovery binds only an exact authenticated run", async ({ page }) => {
  const f = await submissionFixture(page);
  f.setMode("timeout");
  await page.addInitScript(() => {
    const set = Storage.prototype.setItem;
    Storage.prototype.setItem = function(key, value) {
      if (key.startsWith("crossline.execution.pendingConfirm.v2:") && !sessionStorage.getItem("allow-submit-storage"))
        throw new DOMException("fixture storage denied");
      return set.call(this, key, value);
    };
  });
  await reviewAndSubmit(page);
  await expect(page.locator(".execution-actionbar")).toContainText("无法保存原提交记录");
  expect(f.confirms).toHaveLength(0);
  await page.evaluate(() => sessionStorage.setItem("allow-submit-storage", "yes"));
  await page.reload();
  await reviewAndSubmit(page);
  await expect(page.locator(".execution-actionbar")).toContainText("提交结果待核对");
  const raw = await saved(page);
  const legacyKey = `crossline.execution.pendingConfirm:${API}`;
  await page.evaluate(({ key, legacyKey, raw }) => {
    localStorage.setItem(legacyKey, raw!); localStorage.removeItem(key);
  }, { key: confirmRecoveryKey(), legacyKey, raw });
  f.setRuns([{ ...f.makeRun(), ticketId: "wrong-ticket" }]);
  await page.reload();
  const legacy = page.locator(".execution-legacy-recovery");
  await expect(legacy).toContainText("旧版提交待核对");
  await legacy.getByRole("button", { name: "核对旧版提交", exact: true }).click();
  await expect(legacy).toContainText("当前登录未取得匹配的原运行记录");
  expect(await saved(page, legacyKey)).toBe(raw);
  expect(await saved(page)).toBeNull();
  f.setRuns([f.makeRun(undefined, "hedged", NOW + 30)]);
  await legacy.getByRole("button", { name: "核对旧版提交", exact: true }).click();
  await expect(legacy).toHaveCount(0);
  await expect(page.locator(".execution-page")).toContainText("双腿成交已确认");
  expect(await saved(page, legacyKey)).toBeNull();
  expect(await saved(page)).toBeNull();
  expect(f.confirms).toHaveLength(1);
  expect(f.cancels).toHaveLength(0);
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});
