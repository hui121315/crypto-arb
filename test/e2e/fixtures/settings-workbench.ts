import { type Page } from "@playwright/test";
import { API, NOW, setup } from "./opportunity-workbench";

export async function settingsFixture(page: Page, initialTab = "webhook") {
  const base = await setup(page);
  const requests: { method: string; path: string; body: any }[] = [];
  const failures = new Set<string>();
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
  await page.route(/\/api\/(webhook\/|system\/market-subscriptions)/, async (route) => {
    const url = new URL(route.request().url());
    if (url.origin !== API) return route.fallback();
    const method = route.request().method(), path = url.pathname, key = `${method} ${path}`;
    const body = method === "GET" ? null : route.request().postDataJSON();
    requests.push({ method, path, body });
    const failed = failures.has(key);
    const read = structuredClone(path.includes("webhook") ? webhook : market);
    if (holds.delete(key)) await new Promise<void>((resolve) => releases.set(key, resolve));
    if (failed) return route.fulfill({ status: 503, json: { error: { code: "SETTINGS_FIXTURE_UNAVAILABLE",
      message: "fixture request unavailable", source: "fixture.settings" } } });
    if (method === "GET") return route.fulfill({ json: read });
    if (path === "/api/webhook/config" && method === "PATCH") {
      for (const key of ["enabled", "provider", "eventKinds", "timeoutMs", "maxAttempts", "baseBackoffMs", "queueCapacity"])
        if (body[key] != null) (webhook.config as any)[key] = body[key];
      if (body.url != null) { webhook.config.urlConfigured = !!body.url; webhook.config.url = body.url ? "https://example.com/***" : ""; }
      if (body.clearSecret) webhook.config.secretConfigured = false;
      if (body.secret) webhook.config.secretConfigured = true;
      webhook.updatedAtMs++;
      return route.fulfill({ json: webhook });
    }
    if (path === "/api/webhook/test" && method === "POST") return route.fulfill({ json: { eventId: "fixture-test", queued: true } });
    if (path === "/api/system/market-subscriptions/config" && method === "PATCH") {
      const row = market.venues.find((row) => row.venue === body.venue)!;
      for (const key of ["spotEnabled", "perpEnabled", "fundingEnabled"]) if (body[key] != null) (row as any)[key] = body[key];
      market.updatedAtMs++;
      return route.fulfill({ json: market });
    }
    return route.abort();
  });
  return { ...base, requests, webhook, market, emit,
    fail: (key: string, value = true) => value ? failures.add(key) : failures.delete(key),
    hold: (key: string) => { releases.delete(key); holds.add(key); },
    release: (key: string) => { if (releases.has(key)) releases.get(key)!(); else holds.delete(key); },
  };
}
