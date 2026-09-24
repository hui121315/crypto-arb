import { type Page, type WebSocketRoute } from "@playwright/test";
import { setup as setupShell, API, WEB, NOW } from "./opportunity-workbench";
export { API, WEB, NOW };

export function runtime() {
  return { config: { enabled: false, paused: false, environment: "paper", strategyKind: "perp_cross",
    canonicalSymbols: [], capitalUsd: 10, leverage: 1, minOneCycleNetBps: 10, minDepthUsd: 10,
    maxConcurrentRuns: 1, cooldownSecs: 60 }, state: "disabled", activeRunCount: 0,
    recentDecisions: [] as any[], lastDecision: null as any, updatedAtMs: NOW };
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
      } else if (message.type === "ping") socket.send(JSON.stringify({ type: "pong" }));
    });
    socket.onClose(() => sockets.delete(socket));
  });
  await page.route(`${API}/api/**`, async (route) => {
    const request = route.request();
    const path = new URL(request.url()).pathname;
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
