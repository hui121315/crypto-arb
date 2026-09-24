import { type Page, type WebSocketRoute } from "@playwright/test";
import { setup as setupShell, API, WEB, NOW } from "./opportunity-workbench";
export { API, WEB, NOW };

export function runtime() {
  return { config: { enabled: false, paused: false, environment: "paper", strategyKind: "perp_cross",
    canonicalSymbols: [], capitalUsd: 10, leverage: 1, minOneCycleNetBps: 10, minDepthUsd: 10,
    maxConcurrentRuns: 1, cooldownSecs: 60 }, state: "disabled", activeRunCount: 0,
    recentDecisions: [] as any[], lastDecision: null as any, updatedAtMs: NOW };
}

export function receipt(id = "fixture-run-0") {
  const leg = (role: string) => ({ role, exchange: role === "long" ? "binance" : "bitget", symbol: "SOL",
    orderIds: [`${id}-${role}`], state: "accepted", targetQuantity: 1, targetNotionalUsd: 10, finalitySource: "adapter_ack" });
  return { run: { runId: id, ticketId: `ticket-${id}`, opportunityId: `opp-${id}`, state: "second_leg_submitted",
    longLeg: leg("long"), shortLeg: leg("short"), netExposureUsd: 0, statusReason: "fixture: waiting for fills",
    createdAtMs: NOW, updatedAtMs: NOW }, closeRuns: [] as any[], closeRunTotal: 0, observedAtMs: NOW };
}

export function closeReceipt(run: ReturnType<typeof receipt>["run"], id = "fixture-close-0") {
  const leg = (side: string) => ({ venue: side === "long" ? "binance" : "bitget", symbol: "SOL", side, status: "accepted",
    quantity: 1, markPrice: 10, notionalUsd: 10, finalitySource: "adapter_ack",
    pairEvidence: { source: "execution_run", runId: run.runId, ticketId: run.ticketId, opportunityId: run.opportunityId,
      venue: side === "long" ? "binance" : "bitget", symbol: "SOL", side,
      partnerVenue: side === "long" ? "bitget" : "binance", partnerSymbol: "SOL", partnerSide: side === "long" ? "short" : "long",
      legFilledQuantity: 1, partnerFilledQuantity: 1, matchedNotionalUsd: 10, updatedAtMs: NOW } });
  return { id, scope: "pair", status: "submitted", snapshotVersion: "fixture", expectedLegCount: 2,
    legs: [leg("long"), leg("short")], submittedOrderCount: 2, failedLegCount: 0, nakedExposureUsd: 0,
    message: "fixture: close accepted", startedAtMs: NOW, updatedAtMs: NOW };
}

export async function setup(page: Page) {
  const shell = await setupShell(page);
  const trading = await (await page.request.get(`${API}/api/trading/status`)).json();
  trading.risk.autoProfitClose = { enabled: true, minNetProfitUsd: 0.125, minRoiBps: 10,
    stopLossEnabled: false, maxNetLossUsd: 2, maxLossRoiBps: 100, liquidationGuardEnabled: false, liquidationExitDistancePct: 8,
    exitBufferBps: 5, confirmationSamples: 3, cooldownSecs: 60 };
  let current = runtime();
  let failedRead = false;
  let failedWrite = false;
  let holdWrite = false;
  let holdRead = false;
  let releaseWrite: (() => void) | undefined;
  let releaseRead: (() => void) | undefined;
  const sockets = new Set<WebSocketRoute>();
  const executionSockets = new Set<WebSocketRoute>();
  const receipts = new Map<string, ReturnType<typeof receipt>>();
  let failedReceipt = false;
  let holdReceipt: string | undefined;
  let releaseReceipt: (() => void) | undefined;
  const requests: { path: string; method: string; body?: any }[] = [];
  const webhook = { config: { enabled: true, provider: "generic", url: "[configured]", urlConfigured: true,
    secretConfigured: true, eventKinds: ["opportunity", "automation_decision"], timeoutMs: 5000, maxAttempts: 3, baseBackoffMs: 500, queueCapacity: 128 },
    queueDepth: 0, deliveredTotal: 0, failedTotal: 0, droppedTotal: 0, recentDeliveries: [], updatedAtMs: NOW };
  await page.routeWebSocket(/.*/, (socket) => {
    if (!socket.url().startsWith(API.replace("http:", "ws:"))) return socket.close();
    socket.onMessage((raw) => {
      const message = JSON.parse(raw.toString());
      if (message.type === "subscribe") {
        socket.send(JSON.stringify({ type: "ack", subscribed: message.channels }));
        if (message.channels.includes("automation")) sockets.add(socket);
        if (message.channels.includes("execution")) executionSockets.add(socket);
      } else if (message.type === "ping") socket.send(JSON.stringify({ type: "pong" }));
    });
    socket.onClose(() => { sockets.delete(socket); executionSockets.delete(socket); });
  });
  await page.route(`${API}/api/**`, async (route) => {
    const request = route.request();
    const path = new URL(request.url()).pathname;
    if (path.startsWith("/api/automation/execution-runs/")) {
      requests.push({ path, method: request.method() });
      const id = decodeURIComponent(path.split("/").at(-1)!);
      const captured = structuredClone(receipts.get(id));
      if (holdReceipt === id) await new Promise<void>((resolve) => { releaseReceipt = resolve; });
      return failedReceipt || !captured
        ? route.fulfill({ status: captured ? 503 : 404, json: { code: "FIXTURE_RECEIPT_UNAVAILABLE", message: "fixture: receipt unavailable" } })
        : route.fulfill({ json: captured });
    }
    if (!["/api/automation/status", "/api/automation/config", "/api/automation/control",
      "/api/trading/status", "/api/trading/risk-config", "/api/webhook/status", "/api/webhook/test"].includes(path)) return route.fallback();
    const body = request.method() === "GET" ? undefined : request.postDataJSON();
    requests.push({ path, method: request.method(), body });
    const fail = (message: string) => route.fulfill({ status: 503, json: { error: { code: "FIXTURE_UNAVAILABLE", message } } });
    if (path === "/api/automation/status") {
      const captured = structuredClone(current);
      if (holdRead) await new Promise<void>((resolve) => { releaseRead = resolve; });
      return failedRead ? fail("fixture: automation unavailable") : route.fulfill({ json: captured });
    }
    if (path === "/api/trading/status") return route.fulfill({ json: trading });
    if (path === "/api/webhook/status") return route.fulfill({ json: webhook });
    if (holdWrite) await new Promise<void>((resolve) => { releaseWrite = resolve; });
    if (failedWrite) return fail("fixture: save rejected");
    if (path === "/api/webhook/test") return route.fulfill({ json: { queued: true, eventId: "fixture-test" } });
    if (path === "/api/trading/risk-config") {
      Object.assign(trading.risk.autoProfitClose, Object.fromEntries(Object.entries(body.autoProfitClose).filter(([, value]) => value != null)));
      return route.fulfill({ json: trading });
    }
    if (path === "/api/automation/config") Object.assign(current.config, Object.fromEntries(Object.entries(body).filter(([, value]) => value != null)));
    else if (body.action === "emergency_stop") Object.assign(current.config, { enabled: false, paused: true });
    else current.config.paused = body.action === "pause";
    current.state = !current.config.enabled ? "disabled" : current.config.paused ? "paused" : "watching";
    current.updatedAtMs++;
    return route.fulfill({ json: current });
  });
  const emit = () => sockets.forEach((socket) => socket.send(JSON.stringify({ type: "message", channel: "automation", payload: current })));
  return { ...shell, requests, webhook,
    setReceipt: (value: ReturnType<typeof receipt>) => { receipts.set(value.run.runId, structuredClone(value)); },
    execution: (payload: any) => executionSockets.forEach((socket) => socket.send(JSON.stringify({ type: "message", channel: "execution", payload }))),
    failReceipt: (value = true) => { failedReceipt = value; },
    holdReceipt: (id: string) => { holdReceipt = id; },
    releaseReceipt: () => { holdReceipt = undefined; releaseReceipt?.(); },
    tick: () => { current.updatedAtMs++; emit(); },
    setStatus: (next: ReturnType<typeof runtime>) => { current = next; emit(); },
    failRead: (value = true) => { failedRead = value; },
    failWrite: (value = true) => { failedWrite = value; },
    holdWrite: () => { holdWrite = true; },
    releaseWrite: () => { holdWrite = false; releaseWrite?.(); },
    holdRead: () => { holdRead = true; },
    releaseRead: () => { holdRead = false; releaseRead?.(); },
    streamFailure: () => sockets.forEach((socket) => socket.send(JSON.stringify({ type: "error", channel: "automation",
      error: { code: "FIXTURE_STREAM_LOST", message: "fixture: automation stream lost" } }))),
  };
}
