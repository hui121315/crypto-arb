import { type Page } from "@playwright/test";
import { API, NOW, setup } from "./opportunity-workbench";

export async function settingsFixture(page: Page, initialTab = "webhook") {
  const base = await setup(page);
  const requests: { method: string; path: string; body: any; requestId?: string; idempotencyKey?: string }[] = [];
  const failures = new Map<string, number>();
  const actions = await (await page.request.get(`${API}/api/trading/action-runs`)).json();
  actions.data = [];
  const holds = new Set<string>();
  const releases = new Map<string, () => void>();
  const webhook = {
    config: { enabled: true, provider: "bark", url: "https://example.com/***", urlConfigured: true,
      secretConfigured: false, eventKinds: ["opportunity", "automation_decision"],
      timeoutMs: 15000, maxAttempts: 3, baseBackoffMs: 500, queueCapacity: 128 },
    queueDepth: 0, deliveredTotal: 1, failedTotal: 0, droppedTotal: 0, updatedAtMs: NOW,
    recentDeliveries: [{ eventId: "fixture-delivery", kind: "test", provider: "bark", status: "delivered",
      attempts: 1, responseStatus: 200, applicationAck: "accepted", error: null, updatedAtMs: NOW }],
  };
  const market = { updatedAtMs: NOW,
    venues: ["binance", "kraken"].map((venue) => ({ venue, spotEnabled: true, perpEnabled: true, fundingEnabled: true })),
    runtime: ["binance", "kraken"].map((venue) => ({ venue,
      spot: { state: "live", rows: 10, source: "ws_push", observedAtMs: NOW },
      perp: { state: "warming", rows: 0 }, funding: { state: "warming", rows: 0 } })),
  };
  await page.addInitScript((tab) => localStorage.setItem("crossline.settings.activeTab", JSON.stringify(tab)), initialTab);
  const emit = () => base.channelSockets.get("webhook")?.forEach((socket) => socket.send(JSON.stringify({
    type: "message", channel: "webhook", payload: webhook,
  })));
  await page.route(/\/api\/(webhook\/|system\/market-subscriptions|trading\/action-runs)/, async (route) => {
    const url = new URL(route.request().url());
    if (url.origin !== API) return route.fallback();
    const method = route.request().method(), path = url.pathname, key = `${method} ${path}`;
    const body = method === "GET" ? null : route.request().postDataJSON();
    const requestId = route.request().headers()["x-request-id"], idempotencyKey = route.request().headers()["idempotency-key"];
    requests.push({ method, path, body, requestId, idempotencyKey });
    if (path.startsWith("/api/trading/action-runs")) {
      const data = path === "/api/trading/action-runs" ? actions : actions.data.find((row: any) => path.endsWith(`/${row.id}`));
      return data ? route.fulfill({ json: data }) : route.fulfill({ status: 404, json: { error: { code: "NOT_FOUND", message: "fixture run absent" } } });
    }
    const run = method === "PATCH" || (method === "POST" && path === "/api/webhook/test") ? { id: `fixture-config-${requests.length}`,
      kind: method === "POST" ? "webhook_test" : path.includes("webhook") ? "webhook_config_update" : "market_subscriptions_update",
      target: method === "POST" ? "webhook-test" : path.includes("webhook") ? "webhook-delivery" : body.venue, requestId, idempotencyKey,
      status: "accepted", actor: "fixture", message: "fixture accepted", startedAtMs: NOW, updatedAtMs: NOW,
      result: null as any, problem: null as any } : null;
    if (run) actions.data.unshift(run);
    const complete = (result: any) => {
      if (run) Object.assign(run, { status: "succeeded", message: "fixture confirmed", result: structuredClone(result) });
      return route.fulfill({ json: result });
    };
    const failed = failures.get(key);
    const read = structuredClone(path.includes("webhook") ? webhook : market);
    if (holds.delete(key)) await new Promise<void>((resolve) => releases.set(key, resolve));
    if (failed) {
      const problem = { code: "SETTINGS_FIXTURE_UNAVAILABLE", message: "fixture request unavailable", source: "fixture.settings", status: failed };
      if (run) Object.assign(run, { status: "failed", problem });
      return route.fulfill({ status: failed, json: { error: problem } });
    }
    if (method === "GET") return route.fulfill({ json: read });
    if (path === "/api/webhook/config" && method === "PATCH") {
      for (const key of ["enabled", "provider", "eventKinds", "timeoutMs", "maxAttempts", "baseBackoffMs", "queueCapacity"])
        if (body[key] != null) (webhook.config as any)[key] = body[key];
      if (body.url != null) { webhook.config.urlConfigured = !!body.url; webhook.config.url = body.url ? "https://example.com/***" : ""; }
      if (body.clearSecret) webhook.config.secretConfigured = false;
      if (body.secret) webhook.config.secretConfigured = true;
      webhook.updatedAtMs++;
      return complete(webhook);
    }
    if (path === "/api/webhook/test" && method === "POST") return complete({ eventId: `evt-webhook-test-${run!.id}`, queued: true, requestId, actionRunId: run!.id, idempotencyKey });
    if (path === "/api/system/market-subscriptions/config" && method === "PATCH") {
      const row = market.venues.find((row) => row.venue === body.venue)!;
      for (const key of ["spotEnabled", "perpEnabled", "fundingEnabled"]) if (body[key] != null) (row as any)[key] = body[key];
      market.updatedAtMs++;
      return complete(market);
    }
    return route.abort();
  });
  return { ...base, requests, webhook, market, emit, actions,
    fail: (key: string, value = true, status = 503) => value ? failures.set(key, status) : failures.delete(key),
    hold: (key: string) => { releases.delete(key); holds.add(key); },
    release: (key: string) => { if (releases.has(key)) releases.get(key)!(); else holds.delete(key); },
  };
}
