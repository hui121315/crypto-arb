import { expect, test, type Page } from "@playwright/test";
import { submissionFixture, reviewAndSubmit } from "./fixtures/execution-submission";
import { setup as automationFixture, receipt, runtime } from "./fixtures/automation-workbench";
import { setup, API, NOW } from "./fixtures/opportunity-workbench";

function runHref(module: string, run: any) {
  return `#${module}?${new URLSearchParams({ run: run.runId, ticket: run.ticketId, opp: run.opportunityId })}`;
}

function positionsFor(run: any) {
  return ["long", "short"].map((side) => {
    const leg = run[`${side}Leg`], peer = run[side === "long" ? "shortLeg" : "longLeg"];
    return { venue: leg.exchange, symbol: leg.symbol, side, quantity: 1, entryPrice: 100, markPrice: 101,
      origin: "execution_ledger", leverage: 1, unrealizedPnlUsd: 1, marginUsd: 100,
      liquidationPrice: 30, liquidationDistancePct: 70, severity: "ok", fundingRateVerified: true,
      nextFundingMs: NOW + 3600000, fundingRate8h: 0.0001, secondsUntilFunding: 3600,
      pairedWith: peer.exchange, pairEvidence: { source: "execution_run", runId: run.runId, ticketId: run.ticketId,
        opportunityId: run.opportunityId, venue: leg.exchange, symbol: leg.symbol, side,
        partnerVenue: peer.exchange, partnerSymbol: peer.symbol, partnerSide: side === "long" ? "short" : "long",
        legFilledQuantity: 1, partnerFilledQuantity: 1, matchedNotionalUsd: 100, updatedAtMs: NOW } };
  });
}

async function portfolioFixture(page: Page, rows: () => any[]) {
  const envelope = await (await page.request.get(`${API}/api/trading/portfolio/snapshot`)).json();
  const requests: string[] = [];
  await page.route(`${API}/api/trading/portfolio/nav-history**`, (route) =>
    route.fulfill({ json: { count: 0, rows: [], source: "isolated-fixture", observedAtMs: NOW } }));
  await page.route(`${API}/api/trading/portfolio/snapshot`, (route) => {
    requests.push(route.request().method());
    const body = structuredClone(envelope), snapshot = body.snapshot;
    Object.assign(snapshot, { positions: rows(), serverNowMs: NOW, snapshotVersion: "handoff-fixture", degraded: false, problems: [], recentCloseRuns: [] });
    Object.assign(snapshot.accountState.positions, { status: "fresh", rows: [], problems: [], fieldQuality: [], rowHealth: [], observedAtMs: NOW });
    snapshot.accountState.fieldQuality = [];
    body.observedAtMs = NOW;
    return route.fulfill({ json: body });
  });
  return { requests };
}

test("filled execution opens only its paired positions, preserves whole-account risk and restores the route on reload", async ({ page }) => {
  const f = await submissionFixture(page);
  let rows: any[] = [];
  await portfolioFixture(page, () => rows);
  await page.addInitScript(() => localStorage.setItem("crossline.positions.query", JSON.stringify("unrelated-old-query")));
  await reviewAndSubmit(page);
  const run = f.makeRun(undefined, "hedged", NOW + 30);
  rows = [...positionsFor(run), ...positionsFor({ ...run, runId: "unrelated-run", ticketId: "unrelated-ticket", longLeg: { ...run.longLeg, exchange: "binance" }, shortLeg: { ...run.shortLeg, exchange: "okx" } })];
  f.emitRun(run);
  const link = page.getByRole("link", { name: "去持仓平仓", exact: true });
  await expect(link).toHaveAttribute("href", runHref("positions", run));
  await link.click();
  const scope = page.locator(".positions-run-scope");
  await expect(scope).toContainText(run.runId);
  const table = page.locator(".positions-table");
  await expect(table.locator(".row-close-button")).toHaveCount(2);
  await expect(table).not.toContainText("binance");
  await expect(page.getByPlaceholder("搜索场所、标的、配对")).toHaveValue("");
  await expect(scope).toContainText("风险摘要仍为全账户");
  const risk = await page.locator(".positions-risk-command").textContent();
  await page.reload();
  await expect(table.locator(".row-close-button")).toHaveCount(2);
  await page.screenshot({ path: test.info().outputPath("handoff-desktop.png"), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(scope.getByRole("link", { name: "查看全部持仓" })).toBeVisible();
  await scope.getByRole("link", { name: "查看全部持仓" }).click({ trial: true });
  await page.screenshot({ path: test.info().outputPath("handoff-mobile.png"), fullPage: true });
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(390);
  await scope.getByRole("link", { name: "查看全部持仓" }).click();
  await expect(table.locator(".row-close-button")).toHaveCount(4);
  expect(await page.locator(".positions-risk-command").textContent()).toBe(risk);
  expect(f.confirms).toHaveLength(1); expect(f.cancels).toHaveLength(0);
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});

test("missing exact pair evidence never falls back to another same-symbol position", async ({ page }) => {
  const f = await setup(page);
  const run = receipt().run;
  const wrongTicket = positionsFor({ ...run, ticketId: "another-ticket" });
  const unpaired = { ...wrongTicket[0], pairEvidence: null, pairedWith: null };
  await portfolioFixture(page, () => [...wrongTicket, unpaired]);
  await page.goto(`/${runHref("positions", run)}`);
  await expect(page.locator(".positions-run-scope")).toContainText("尚未找到明确关联的持仓");
  await expect(page.locator(".positions-table")).toBeHidden();
  await expect(page.locator(".positions-run-scope")).toContainText("不代表没有持仓或已平仓");
  await page.getByRole("link", { name: "查看全部持仓" }).click();
  await expect(page.locator(".positions-table")).toBeVisible();
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});

test("automation opens an older exact run without mixing a cached newer execution or rebuilding a ticket", async ({ page }) => {
  const f = await automationFixture(page);
  const old = receipt("old/run &1"); old.run.updatedAtMs = NOW - 1000;
  const newer = receipt("newer-run");
  f.setReceipt(old);
  const status = runtime();
  status.lastDecision = { id: "decision", kind: "submitted", symbol: "SOL", reason: "fixture receipt", executionRunId: old.run.runId, occurredAtMs: NOW };
  status.recentDecisions = [status.lastDecision];
  f.setStatus(status);
  const seed = await (await page.request.get(`${API}/api/trading/execution-runs`)).json();
  const queries: URLSearchParams[] = [];
  await page.route(`${API}/api/trading/execution-runs**`, (route) => {
    const q = new URL(route.request().url()).searchParams;
    queries.push(q);
    const rows = [newer.run, old.run].filter((run) => run.runId === q.get("runId"));
    return route.fulfill({ json: { ...seed, rows, status: "fresh", problems: [], page: { ...seed.page, returnedCount: rows.length, totalRows: rows.length } } });
  });
  await page.goto(`/${runHref("execution", newer.run)}`);
  await expect(page.locator(".execution-runtime-disclosure")).toContainText(newer.run.runId);
  await page.locator('.module-tabs button[data-module="automation"]').click();
  await page.getByRole("tab", { name: "运行回执", exact: true }).click();
  const panel = page.getByRole("region", { name: "自动化运行回执", exact: true });
  await expect(panel.getByRole("link", { name: "关联持仓" })).toHaveAttribute("href", runHref("positions", old.run));
  await panel.getByRole("link", { name: "运行订单" }).click();
  await expect(page.locator(".execution-runtime-disclosure")).toContainText(old.run.runId);
  await expect(page.locator(".execution-runtime-disclosure")).not.toContainText(newer.run.runId);
  await expect(page.locator(".confirm-action.primary")).toHaveCount(0);
  expect(queries.at(-1)!.get("ticketId")).toBe(old.run.ticketId);
  expect(queries.at(-1)!.get("opportunityId")).toBe(old.run.opportunityId);
  await page.evaluate((hash) => { location.hash = hash; }, runHref("execution", { ...old.run, ticketId: "wrong-ticket" }));
  await expect(page.locator(".execution-idle")).toContainText("指定运行尚未读取成功");
  await expect(page.locator(".execution-runtime-disclosure")).toContainText("execution run seed returned no row");
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});

test("an unknown submission retains its original identity when another run link is opened", async ({ page }) => {
  const f = await submissionFixture(page);
  f.setMode("timeout");
  await reviewAndSubmit(page);
  await expect(page.getByRole("button", { name: "查询提交结果", exact: true })).toBeVisible();
  const other = receipt("not-the-pending-run").run;
  await page.evaluate((hash) => { location.hash = hash; }, runHref("execution", other));
  await expect(page.locator(".execution-page")).toContainText("继续显示原执行；未切换到其他运行记录");
  const before = f.reads.length;
  await page.getByRole("button", { name: "查询提交结果", exact: true }).click();
  await expect.poll(() => f.reads.length).toBeGreaterThan(before);
  const query = new URLSearchParams(f.reads.at(-1));
  expect(query.get("runId")).toBe(`run-${f.confirms[0].idempotencyKey}`);
  expect(f.confirms).toHaveLength(1);
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});
