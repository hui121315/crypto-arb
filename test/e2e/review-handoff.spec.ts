import { expect, test, type Page } from "@playwright/test";
import { reviewFixture } from "./fixtures/review-workbench";
import { receipt, closeReceipt, runtime } from "./fixtures/automation-workbench";
import { API, NOW } from "./fixtures/opportunity-workbench";

const run = receipt("review/run &1").run;
const href = (id = run.runId) => `#review?${new URLSearchParams({ run: id, ticket: run.ticketId, opp: run.opportunityId })}`;

async function setup(page: Page) {
  const f = await reviewFixture(page);
  const reads: URLSearchParams[] = [];
  let failed = false, empty = false, hold: string | undefined, release: (() => void) | undefined;
  await page.route(`${API}/api/review/executed**`, async (route) => {
    const query = new URL(route.request().url()).searchParams;
    if (!query.has("runId") && !query.has("closeRunId")) return route.fallback();
    reads.push(query);
    const capturedFailure = failed;
    const rows = empty || query.get("runId") === "missing" ? [] : [{ ...f.trade, id: "associated-trade", symbol: "SOL" }];
    const body = f.envelope(rows);
    Object.assign(body, { days: 365, rowCount: rows.length });
    Object.assign(body.page, { totalRows: rows.length, hasMore: false, nextCursor: null, lastCursor: null });
    if (hold && query.get("runId") === hold) { hold = undefined; await new Promise<void>((resolve) => { release = resolve; }); }
    return capturedFailure
      ? route.fulfill({ status: 503, json: { error: { code: "SCOPED_REVIEW_UNAVAILABLE", message: "fixture scoped read failed" } } })
      : route.fulfill({ json: body });
  });
  return { ...f, scopedReads: reads, fail: (value: boolean) => { failed = value; }, empty: () => { empty = true; },
    hold: (id: string) => { hold = id; }, release: () => release?.() };
}

test("scoped review restores exact route, opens evidence and resists global WS replacement", async ({ page }) => {
  const f = await setup(page);
  await page.addInitScript(() => localStorage.setItem("crossline.review.activeTab", JSON.stringify("strategy")));
  await page.goto(`/${href()}`);
  await expect(page.locator(".review-record-scope")).toContainText(run.runId);
  await expect(page.locator(".review-selected-trade")).toContainText("SOL");
  const query = f.scopedReads.at(-1)!;
  expect(query.get("runId")).toBe(run.runId);
  expect(query.get("ticketId")).toBe(run.ticketId);
  expect(query.get("opportunityId")).toBe(run.opportunityId);
  expect(query.get("days")).toBe("365");
  const before = f.scopedReads.length;
  f.emit();
  await expect(page.locator(".review-executed-table tbody")).toContainText("SOL");
  expect(f.scopedReads.length).toBe(before);
  await page.reload();
  await expect(page.locator(".review-selected-trade")).toContainText("SOL");
  const summary = page.locator(".review-executed-summary");
  await expect.poll(async () => {
    const metrics = await summary.boundingBox();
    const table = await page.locator(".review-executed-workbench").boundingBox();
    return metrics && table ? table.y - (metrics.y + metrics.height) : -1;
  }).toBeGreaterThanOrEqual(0);
  expect(await summary.locator("strong").evaluateAll((values) => values.every((value) => {
    const box = value.getBoundingClientRect();
    return value.contains(document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2));
  }))).toBe(true);
  await page.screenshot({ path: test.info().outputPath("scoped-review-desktop.png"), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.getByRole("link", { name: "全部执行记录", exact: true }).click({ trial: true });
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(390);
  await page.screenshot({ path: test.info().outputPath("scoped-review-mobile.png"), fullPage: true });
  await page.getByRole("link", { name: "全部执行记录", exact: true }).click();
  await expect(page.locator(".review-record-scope")).toHaveCount(0);
  await expect(page.locator(".review-executed-table tbody")).toContainText("BTC");
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});

test("missing or failed scoped review never falls back to global trades and refresh retries exact scope", async ({ page }) => {
  const f = await setup(page);
  f.fail(true);
  await page.goto(`/${href()}`);
  await expect(page.locator(".review-state-disclosure")).toContainText("SCOPED_REVIEW_UNAVAILABLE");
  await expect(page.locator(".review-executed-table tbody")).not.toContainText("BTC");
  f.fail(false);
  await page.getByRole("button", { name: "刷新复盘记录" }).click();
  await expect(page.locator(".review-selected-trade")).toContainText("SOL");
  f.fail(true);
  await page.getByRole("button", { name: "刷新复盘记录" }).click();
  await expect(page.locator(".review-state-disclosure")).toContainText("显示上次快照");
  await expect(page.locator(".review-executed-table tbody")).toContainText("SOL");
  f.fail(false);
  await page.evaluate((hash) => { location.hash = hash; }, href("missing"));
  await expect(page.locator(".review-record-scope")).toContainText("未找到可核验的关联记录");
  await expect(page.locator(".review-selected-trade")).toHaveCount(0);
  await expect(page.locator(".review-executed-table tbody")).not.toContainText("SOL");
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});

test("leaving a slow scoped read cannot replace the restored global first page", async ({ page }) => {
  const f = await setup(page);
  f.hold(run.runId);
  await page.goto(`/${href()}`);
  await expect.poll(() => f.scopedReads.length).toBe(1);
  await page.getByRole("link", { name: "全部执行记录" }).click();
  await expect(page.locator(".review-executed-table tbody")).toContainText("BTC");
  const response = page.waitForResponse((response) => response.url().includes("runId="));
  f.release();
  await (await response).finished();
  await expect(page.locator(".review-executed-table tbody")).not.toContainText("SOL");
  await expect(page.getByRole("button", { name: "刷新复盘记录" })).toBeEnabled();
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});

test("position close history links the exact close ID, including reserved characters", async ({ page }) => {
  const f = await setup(page);
  const close = closeReceipt(run, "close/run &1");
  close.status = "succeeded";
  const envelope = await (await page.request.get(`${API}/api/trading/portfolio/snapshot`)).json();
  envelope.snapshot.recentCloseRuns = [close];
  await page.route(`${API}/api/trading/portfolio/snapshot`, (route) => route.fulfill({ json: envelope }));
  await page.route(`${API}/api/trading/portfolio/nav-history**`, (route) => route.fulfill({ json: { count: 0, rows: [], source: "fixture", observedAtMs: NOW } }));
  await page.goto("/#positions");
  await page.getByRole("tab", { name: "平仓", exact: true }).click();
  await page.locator(".position-history-record summary").click();
  await page.getByRole("link", { name: "关联复盘", exact: true }).click();
  await expect(page.locator(".review-record-scope")).toContainText(close.id);
  await expect(page.locator(".review-selected-trade")).toContainText("SOL");
  expect(f.scopedReads.at(-1)!.get("closeRunId")).toBe(close.id);
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});

test("automation receipt opens the same execution in review without any write", async ({ page }) => {
  const f = await setup(page);
  const status = runtime();
  status.lastDecision = { id: "decision", kind: "submitted", symbol: "SOL", reason: "fixture", executionRunId: run.runId, occurredAtMs: NOW };
  status.recentDecisions = [status.lastDecision];
  await page.route(`${API}/api/automation/status`, (route) => route.fulfill({ json: status }));
  await page.route(`${API}/api/automation/execution-runs/**`, (route) => route.fulfill({ json: receipt(run.runId) }));
  await page.goto("/#automation");
  await page.getByRole("tab", { name: "运行回执", exact: true }).click();
  const link = page.getByRole("region", { name: "自动化运行回执", exact: true }).getByRole("link", { name: "关联复盘", exact: true });
  await expect(link).toHaveAttribute("href", href());
  await link.click();
  await expect(page.locator(".review-selected-trade")).toContainText("SOL");
  expect(f.scopedReads.at(-1)!.get("runId")).toBe(run.runId);
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});
