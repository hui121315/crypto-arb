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
  let scopedRows: any[] | undefined;
  await page.route(`${API}/api/review/executed**`, async (route) => {
    const query = new URL(route.request().url()).searchParams;
    if (!query.has("runId") && !query.has("closeRunId")) return route.fallback();
    reads.push(query);
    const capturedFailure = failed;
    const rows = empty || query.get("runId") === "missing" ? [] : (scopedRows ?? [{ ...f.trade, id: "associated-trade", symbol: "SOL" }]);
    const body = f.envelope(rows);
    Object.assign(body, { days: 365, rowCount: rows.length });
    Object.assign(body.page, { totalRows: rows.length, hasMore: false, nextCursor: null, lastCursor: null });
    if (hold && query.get("runId") === hold) { hold = undefined; await new Promise<void>((resolve) => { release = resolve; }); }
    return capturedFailure
      ? route.fulfill({ status: 503, json: { error: { code: "SCOPED_REVIEW_UNAVAILABLE", message: "fixture scoped read failed" } } })
      : route.fulfill({ json: body });
  });
  return { ...f, scopedReads: reads, fail: (value: boolean) => { failed = value; }, empty: () => { empty = true; },
    setScopedRows: (rows: any[]) => { scopedRows = rows; },
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

test("named destinations discard legacy query context and review links use only unambiguous record identities", async ({ page }) => {
  const f = await setup(page);
  const close = (id: string, runId: string, ticketId: string, opportunityId: string) => ({
    closeRunId: id, status: "succeeded", runId, ticketId, opportunityId,
    matchedNotionalUsd: 10, unwindStatus: null, compensationAttemptCount: 0, costReconciliation: null,
  });
  const one = close("close/one &1", "run/one &1", "ticket+1", "opp/1");
  const two = close("close-two", "run-two", "ticket-two", "opp-two");
  const row = { ...f.trade, id: "not-a-run", symbol: "SOL", evidence: {
    ...f.trade.evidence, ledgerEvents: [], closeRunEvidence: [one, one, two],
  } };
  f.setScopedRows([row]);
  const legacy = new URLSearchParams({ module: "review", run: "legacy-run", ticket: "legacy-ticket", opp: "legacy-opp", source: "stocks", record: "legacy-stock" });
  await page.goto(`/?${legacy}`);
  await expect(page.locator(".review-record-scope[role='status']")).toContainText("legacy-run");
  const query = new URLSearchParams({ close: one.closeRunId });
  await page.evaluate(hash => { location.hash = hash; }, `#review?${query}`);
  await expect(page.locator(".review-record-scope[role='status']")).toContainText(one.closeRunId);
  await expect.poll(() => f.scopedReads.at(-1)?.get("closeRunId")).toBe(one.closeRunId);
  for (const key of ["runId", "ticketId", "opportunityId"]) expect(f.scopedReads.at(-1)!.has(key)).toBe(false);
  const links = page.getByRole("navigation", { name: "原运行后续操作", exact: true });
  await expect(links).toHaveCount(2);
  const first = links.filter({ hasText: one.runId });
  for (const [name, module] of [["查看原执行", "execution"], ["关联持仓", "positions"]]) {
    await expect(first.getByRole("link", { name, exact: true })).toHaveAttribute("href",
      `#${module}?${new URLSearchParams({ run: one.runId, ticket: one.ticketId, opp: one.opportunityId })}`);
  }
  // Two different explicit opportunities cannot silently collapse to a broader run link.
  row.evidence.closeRunEvidence = [one, { ...one, opportunityId: "conflicting-opp" }, two];
  await page.getByRole("button", { name: "刷新复盘记录", exact: true }).click();
  await expect(links).toHaveCount(1);
  await expect(links).toContainText(two.runId);
  row.evidence.closeRunEvidence = [];
  await page.getByRole("button", { name: "刷新复盘记录", exact: true }).click();
  await expect(links).toHaveCount(0);
  await expect(page.locator(".review-selected-trade")).toContainText("SOL");
  await page.getByRole("link", { name: "全部执行记录", exact: true }).click();
  await expect(page.locator(".review-record-scope[role='status']")).toHaveCount(0);
  await expect(page.locator(".review-executed-table tbody")).toContainText("BTC");
  await page.getByRole("tab", { name: /链上 \/ 股票/ }).click();
  await expect(page.getByRole("combobox", { name: "收支记录来源", exact: true })).toHaveValue("all");
  await page.evaluate(() => { location.hash = "#positions"; });
  await expect(page.locator(".positions-layout")).toBeVisible();
  await expect(page.locator(".positions-run-scope")).toHaveCount(0);
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
  await expect(page.locator(".review-record-scope")).toContainText("未找到可核对的关联记录");
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
  await page.getByRole("tab", { name: "交易记录", exact: true }).click();
  const link = page.getByRole("region", { name: "自动化交易记录", exact: true }).getByRole("link", { name: "关联复盘", exact: true });
  await expect(link).toHaveAttribute("href", href());
  await link.click();
  await expect(page.locator(".review-selected-trade")).toContainText("SOL");
  expect(f.scopedReads.at(-1)!.get("runId")).toBe(run.runId);
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});
