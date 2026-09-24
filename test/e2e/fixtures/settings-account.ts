import { type Page } from "@playwright/test";
import { API, NOW } from "./opportunity-workbench";
import { settingsFixture } from "./settings-workbench";

export async function settingsAccountFixture(page: Page, tab = "credentials") {
  const base = await settingsFixture(page, tab);
  const seed = async (path: string) => (await page.request.get(`${API}${path}`)).json();
  const credentials = await seed("/api/exchanges/credentials");
  const adapters = await seed("/api/trading/adapters");
  const status = await seed("/api/trading/status");
  const health = await seed("/api/system/health");
  const actions = await seed("/api/trading/action-runs");
  const first = credentials.venues[0];
  first.fields = first.fields.slice(0, 3).map((field: any) => ({ ...field, configured: true }));
  credentials.venues.push({ ...structuredClone(first), venue: "binance", label: "Binance" });
  actions.data = [{ id: "old-save", kind: "venue_credentials_update", status: "succeeded", actor: "fixture",
    target: "binance", requestId: "old-request", idempotencyKey: "old-key", message: "old credential save",
    startedAtMs: NOW - 100, updatedAtMs: NOW - 10 }];
  const live = adapters.options.find((row: any) => row.environment === "live");
  live.enabled = true; live.credentialsAvailable = true; live.disabledReason = null;
  const calls: { key: string; body: any; idempotency?: string }[] = [];
  const failures = new Set<string>(), holds = new Set<string>();
  const releases = new Map<string, () => void>();
  const paths = new Map<string, any>([
    ["/api/exchanges/credentials", credentials], ["/api/trading/adapters", adapters],
    ["/api/trading/status", status], ["/api/system/health", health], ["/api/trading/action-runs", actions],
  ]);
  await page.route(/\/api\/(exchanges\/credentials|trading\/(adapters|status|action-runs)|system\/health)/, async (route) => {
    const request = route.request(), url = new URL(request.url());
    if (url.origin !== API) return route.fallback();
    const key = `${request.method()} ${url.pathname}`;
    const body = request.method() === "POST" ? request.postDataJSON() : null;
    calls.push({ key, body, idempotency: request.headers()["idempotency-key"] });
    const failed = failures.has(key), snapshot = structuredClone(paths.get(url.pathname));
    if (holds.delete(key)) await new Promise<void>((resolve) => releases.set(key, resolve));
    if (failed) return route.fulfill({ status: 503, json: { error: { code: "FIXTURE_SAVE_UNCONFIRMED", message: "fixture request failed", source: "fixture.settings" } } });
    if (request.method() === "GET") return snapshot ? route.fulfill({ json: snapshot }) : route.fallback();
    if (key === "POST /api/exchanges/credentials") {
      const row = credentials.venues.find((row: any) => row.venue === body.venue);
      for (const field of row.fields) if (body.fields.some((f: any) => f.key === field.key)) field.configured = true;
      return route.fulfill({ json: { venue: row.venue, label: row.label, configuredCount: row.fields.filter((f: any) => f.configured).length,
        fieldCount: row.fields.length, message: "fixture saved", secretStorage: credentials.secretStorage, requestId: "fixture-saved" } });
    }
    if (key === "POST /api/exchanges/credentials/clear" || key === "POST /api/exchanges/credentials/migrate") {
      const row = credentials.venues.find((row: any) => row.venue === body.venue);
      const clear = url.pathname.endsWith("/clear");
      if (clear) for (const field of row.fields) if (body.fields.includes(field.key)) field.configured = false;
      return route.fulfill({ json: { venue: row.venue, label: row.label, operation: clear ? "clear" : "migrate",
        affectedFields: body.fields ?? [], missingFields: [], message: "fixture maintained", secretStorage: credentials.secretStorage } });
    }
    if (key === "POST /api/trading/adapters/select") {
      const selected = adapters.options.find((row: any) => row.id === body.adapterId);
      if (!selected) return route.fulfill({ status: 400, json: { error: { code: "INVALID_ADAPTER", message: "unknown fixture adapter" } } });
      adapters.current = selected.id; adapters.currentEnvironment = selected.environment;
      status.adapter = selected.id; status.environment = selected.environment;
      status.risk.liveTradingEnabled = selected.environment === "live";
      return route.fulfill({ json: status });
    }
    return route.fallback();
  });
  return { ...base, calls, credentials, actions, adapters, status,
    failAccount: (key: string, fail = true) => fail ? failures.add(key) : failures.delete(key),
    holdAccount: (key: string) => { releases.delete(key); holds.add(key); },
    releaseAccount: (key: string) => { if (releases.has(key)) releases.get(key)!(); else holds.delete(key); },
  };
}
