import { expect, type Page } from "@playwright/test";
import { executionFixture, openExecution } from "./execution-workbench";
import { API, NOW } from "./opportunity-workbench";

export async function submissionFixture(page: Page) {
  const f = await executionFixture(page);
  const runsSeed = await (await page.request.get(`${API}/api/trading/execution-runs`)).json();
  const actionSeed = await (await page.request.get(`${API}/api/trading/action-runs`)).json();
  let rows: any[] = [];
  let actionRows: any[] = [];
  let records: any[] = [];
  let mode = "ack";
  let holdConfirm = false;
  let releaseConfirm: (() => void) | undefined;
  let holdRead = false;
  let releaseRead: (() => void) | undefined;
  let holdCancel = false;
  let releaseCancel: (() => void) | undefined;
  let failedCancel = "";
  const confirms: any[] = [];
  const cancels: string[] = [];
  const reads: string[] = [];
  const envelope = (values: any[]) => ({ ...runsSeed, rows: values,
    page: { ...runsSeed.page, returnedCount: values.length, totalRows: values.length },
    observedAtMs: NOW + 10, problems: [], status: "fresh" });
  const emit = (channel: string, payload: any) => f.channelSockets.get(channel)?.forEach((socket) => socket.send(JSON.stringify({ type: "message", channel, payload })));
  const makeRun = (request = confirms.at(-1), state = "second_leg_submitted", at = NOW + 10) => {
    const filled = state === "hedged" || state === "closed";
    const leg = (role: string, venue: string) => ({ role, exchange: venue, symbol: "BTC", orderIds: [`${request.idempotencyKey}-${role}`],
      state: filled ? "filled" : "accepted", targetQuantity: 1, filledQuantity: filled ? 1 : null,
      targetNotionalUsd: 100, filledNotionalUsd: filled ? 100 : null, filledFee: filled ? 0.05 : null,
      finalitySource: filled ? "private_ws" : "adapter_ack", confirmedFilledAtMs: filled ? at : null });
    return { runId: `run-${request.idempotencyKey}`, ticketId: request.ticketId,
      opportunityId: "fixture-perp_cross-BTC", state, longLeg: leg("long", "hyperliquid:km"), shortLeg: leg("short", "kucoin"),
      netExposureUsd: 0, recoveryAction: filled ? null : "cancel_open_orders",
      statusReason: filled ? "双腿成交已确认" : "双腿已受理，等待交易所成交", createdAtMs: NOW, updatedAtMs: at };
  };
  const makeOrder = (id: string, state: string, at: number) => ({
    intent: { id, source: "arbitrage_preview", strategy: "perp_cross", mode: "dry_run",
      exchange: id.endsWith("long") ? "hyperliquid:km" : "kucoin", symbol: "BTC", side: id.endsWith("long") ? "buy" : "sell",
      orderType: "limit", quantity: 1, price: 100, reduceOnly: false, timeInForce: "ioc", postOnly: false,
      marginMode: "cross", leverage: 1, clientOrderId: id, createdAtMs: NOW },
    state, lastUpdateSource: "private_ws", updatedAtMs: at, message: `fixture ${state}`, filledQuantity: state === "filled" ? 1 : null,
  });
  await page.route("**/api/**", async (route) => {
    const url = new URL(route.request().url());
    if (url.origin !== API) return route.fallback();
    if (url.pathname.endsWith("/confirm")) {
      const request = route.request().postDataJSON();
      confirms.push(request);
      const run = makeRun(request);
      if (holdConfirm) await new Promise<void>((resolve) => { releaseConfirm = resolve; });
      if (mode === "timeout" || mode === "reject") return route.fulfill({ status: mode === "timeout" ? 504 : 400,
        json: { error: { code: mode === "timeout" ? "TIMEOUT" : "HEDGE_PRE_TRADE_REJECTED", message: mode === "timeout" ? "fixture response lost" : "fixture rejected before order" } } });
      if (!rows.length) rows = [run];
      return route.fulfill({ json: { status: "submitted", idempotencyKey: request.idempotencyKey,
        context: { opportunityId: run.opportunityId, idempotencyKey: request.idempotencyKey, ticketId: request.ticketId, environment: "paper" },
        executionRun: mode === "wrong" ? { ...run, ticketId: "wrong-ticket" } : run } });
    }
    if (url.pathname === "/api/trading/execution-runs") {
      reads.push(url.search);
      const snapshot = structuredClone(rows);
      if (holdRead) { holdRead = false; await new Promise<void>((resolve) => { releaseRead = resolve; }); }
      return route.fulfill({ json: envelope(snapshot) });
    }
    if (url.pathname === "/api/trading/orders") return route.fulfill({ json: envelope(records) });
    if (url.pathname === "/api/trading/action-runs") return route.fulfill({ json: { ...actionSeed, data: actionRows } });
    if (url.pathname.endsWith("/cancel")) {
      const id = decodeURIComponent(url.pathname.split("/").at(-2)!);
      cancels.push(id);
      if (holdCancel) { holdCancel = false; await new Promise<void>((resolve) => { releaseCancel = resolve; }); }
      if (id.endsWith(failedCancel) && failedCancel) return route.fulfill({ status: 504, json: { error: { code: "TIMEOUT", message: "fixture cancel outcome unknown" } } });
      const record = makeOrder(id, "cancelled", NOW + 20);
      records = [...records.filter((row) => row.intent.id !== id), record];
      return route.fulfill({ json: record });
    }
    return route.fallback();
  });
  return { ...f, confirms, cancels, reads, makeRun,
    setMode: (value: string) => { mode = value; },
    holdConfirm: () => { holdConfirm = true; }, releaseConfirm: () => { holdConfirm = false; releaseConfirm?.(); },
    holdRead: () => { holdRead = true; }, releaseRead: () => releaseRead?.(),
    holdCancel: () => { holdCancel = true; }, releaseCancel: () => releaseCancel?.(),
    failCancel: (leg: string) => { failedCancel = leg; },
    setRuns: (values: any[]) => { rows = structuredClone(values); },
    setActions: (values: any[]) => { actionRows = structuredClone(values); },
    emitRun: (run: any) => { rows = [structuredClone(run)]; emit("execution", { event: "execution_run_updated", executionRun: run, timestampMs: run.updatedAtMs }); },
    emitOrder: (id: string, state: string, at: number) => {
      const record = makeOrder(id, state, at);
      records = [...records.filter((row) => row.intent.id !== id), record];
      emit("orders", { event: "order_updated", record, timestampMs: at });
    },
  };
}

export async function reviewAndSubmit(page: Page) {
  await openExecution(page);
  await page.getByRole("button", { name: "校验票据" }).click();
  await page.locator(".execution-artifact").getByRole("checkbox").check();
  await expect(page.locator(".confirm-action.primary")).toBeEnabled();
  await page.locator(".confirm-action.primary").click();
}
