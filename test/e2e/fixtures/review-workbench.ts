import { type Page } from "@playwright/test";
import { setup, API, NOW } from "./opportunity-workbench";

export async function reviewFixture(page: Page) {
  const f = await setup(page);
  const seed = await (await page.request.get(`${API}/e2e-large-tables/api/review/executed?limit=1`)).json();
  const missedSeed = await (await page.request.get(`${API}/e2e-large-tables/api/review/missed?limit=1`)).json();
  const quality = await (await page.request.get(`${API}/api/trading/venues/quality`)).json();
  quality.rows[0].operationHealth = [{ venue: "binance", operation: "positions_read", status: "ok",
    source: "fixture.quality", message: "fixture version 1", observedAtMs: NOW }];
  quality.operationCount = 1;
  const trade = { ...seed.rows[0], id: "review-1", symbol: "BTC", openedAtMs: NOW - 120000, closedAtMs: NOW - 60000 };
  const perf = { kind: "perp_cross", sampleWindowDays: 30, totalTrades30d: 2, trades30d: 2,
    actualTrades30d: 0, estimatedTrades30d: 2, sampleStatus: "partial_evidence", hitRatePct: 0,
    avgPnlPerTradeUsd: 0, sharpe30d: 0, sortino30d: 0, maxDrawdownPct: 0, grossPnl30dUsd: 0,
    netPnl30dUsd: 0, estimatedNetPnl30dUsd: 4, avgHoldingHours: 1 };
  const envelope = (rows: any[], offset = 0, more = false) => ({ ...structuredClone(seed), rows, rowCount: rows.length,
    generatedAtMs: NOW, status: "fresh", ledgerStatus: "ledger_backed", problems: [],
    page: { ...seed.page, startOffset: offset, returnedCount: rows.length, totalRows: 2,
      limit: 1, hasMore: more, hasNextPage: more, nextCursor: more ? "page-2" : null,
      previousCursor: null, lastCursor: more ? "page-2" : null, snapshotId: "review-fixture" } });
  let snapshot = { executed: envelope([trade], 0, true), strategyPerformance: envelope([perf]), generatedAtMs: NOW };
  let failPage = false, holdPage = false, failRuntime = false, holdRuntime = false;
  let releasePage: (() => void) | undefined, releaseRuntime: (() => void) | undefined;
  const reads: string[] = [];
  const problem = { error: { code: "REVIEW_FIXTURE_UNAVAILABLE", message: "fixture read failed", source: "fixture.review" } };
  await page.route("**/api/trading/venues/quality", async (route) => {
    if (new URL(route.request().url()).origin !== API) return route.fallback();
    reads.push("/api/trading/venues/quality");
    return route.fulfill({ json: quality });
  });
  await page.addInitScript(() => localStorage.setItem("crossline.review.activeTab", JSON.stringify("executed")));
  await page.route("**/api/review/**", async (route) => {
    const url = new URL(route.request().url());
    if (url.origin !== API) return route.fallback();
    reads.push(url.pathname + url.search);
    if (url.pathname.endsWith("/runtime")) {
      const response = structuredClone(snapshot), failed = failRuntime;
      if (holdRuntime) { holdRuntime = false; await new Promise<void>((resolve) => { releaseRuntime = resolve; }); }
      return failed ? route.fulfill({ status: 503, json: problem }) : route.fulfill({ json: response });
    }
    if (url.pathname.endsWith("/executed")) {
      const failed = failPage;
      if (holdPage) { holdPage = false; await new Promise<void>((resolve) => { releasePage = resolve; }); }
      return failed ? route.fulfill({ status: 503, json: problem })
        : route.fulfill({ json: envelope([{ ...trade, id: "review-2", symbol: "ETH" }], 1) });
    }
    if (url.pathname.endsWith("/missed")) return route.fulfill({ json: {
      ...envelope([{ ...missedSeed.rows[0], detectedAtMs: NOW - 45000 }]), source: "missed_opportunity_store",
    } });
    return route.fallback();
  });
  const emit = () => f.channelSockets.get("review")?.forEach((socket) => socket.send(JSON.stringify({ type: "message", channel: "review", payload: snapshot })));
  return { ...f, reads, trade, perf, envelope,
    qualityMessage: (value: string) => { quality.rows[0].operationHealth[0].message = value; },
    snapshot: () => structuredClone(snapshot),
    emit: (next = snapshot) => { snapshot = structuredClone(next); emit(); },
    failPage: (value: boolean) => { failPage = value; }, holdPage: () => { holdPage = true; }, releasePage: () => releasePage?.(),
    failRuntime: (value: boolean) => { failRuntime = value; }, holdRuntime: () => { holdRuntime = true; }, releaseRuntime: () => releaseRuntime?.(),
  };
}
