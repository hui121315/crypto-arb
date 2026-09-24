import { type Page } from "@playwright/test";
import { settingsFixture } from "./settings-workbench";
import { API, NOW } from "./opportunity-workbench";

export async function evidenceRoutes(page: Page) {
  const seed = async (path: string) => (await page.request.get(`${API}${path}`)).json();
  const actions = await seed("/api/trading/action-runs");
  const health = await seed("/api/system/venue-operation-health");
  const credentials = await seed("/api/exchanges/credentials");
  const providers = { secretStorage: credentials.secretStorage, providers: [
    ["jupiter_swap_v2_keyed", "Jupiter", ["api_key"]], ["zeroex_swap_v2", "0x", ["api_key"]],
    ["okx_dex_v6", "OKX DEX", ["api_key", "secret_key", "passphrase"]], ["lifi", "LI.FI", ["api_key"]],
    ["solana_wallet_signer", "Solana", ["private_key"]], ["evm_wallet_signer", "EVM", ["private_key"]],
    ["backpack_stocks", "Backpack", ["api_key", "secret_key"]],
  ].map(([provider, label, keys]) => ({ provider, label, officialDocsUrl: "https://example.com/fixture",
    fields: (keys as string[]).map((key) => ({ key, label: key, envKey: `FIXTURE_${key.toUpperCase()}`,
      configured: true, secret: true, required: true, source: "runtime" })),
    missingFields: [], configuredCount: keys.length, fieldCount: keys.length, ready: true, note: "fixture only" })) };
  actions.data = Array.from({ length: 15 }, (_, i) => ({ id: `fixture-action-${i}`, kind: i === 0 ? "portfolio_close_pair" : "venue_credentials_update",
    status: "succeeded", actor: "fixture", target: `fixture-venue-${i}`, requestId: `request-${i}`, idempotencyKey: `key-${i}`,
    startedAtMs: NOW - i * 1000, updatedAtMs: NOW - i * 1000,
    message: `fixture receipt ${i}`, problem: null, result: i === 0 ? { status: "submitted" } : { configuredCount: 1 } }));
  const calls: { key: string; body: any }[] = [];
  const fails = new Set<string>(), holds = new Set<string>(), releases = new Map<string, () => void>();
  const paths = new Map<string, any>([["/api/onchain/credentials", providers], ["/api/trading/action-runs", actions], ["/api/system/venue-operation-health", health]]);
  await page.route(/\/api\/(onchain\/credentials|trading\/action-runs|system\/venue-operation-health|v1\/spot\/ticks)/, async (route) => {
    const request = route.request(), url = new URL(request.url());
    if (url.origin !== API) return route.fallback();
    const key = `${request.method()} ${url.pathname}`, body = request.method() === "POST" ? request.postDataJSON() : null;
    calls.push({ key, body });
    const failed = fails.has(key), snapshot = structuredClone(paths.get(url.pathname) ?? actions.data.find((row: any) => url.pathname.endsWith(`/${row.id}`)));
    if (holds.delete(key)) await new Promise<void>((resolve) => releases.set(key, resolve));
    if (failed) return route.fulfill({ status: 503, json: { error: { code: "EVIDENCE_FIXTURE_FAILED", message: "fixture evidence unavailable", source: "fixture.settings" } } });
    if (url.pathname === "/api/v1/spot/ticks") return route.fulfill({ status: 404, json: { error: { code: "SPOT_FIXTURE_DISABLED", message: "fixture spot endpoint disabled" } } });
    if (request.method() === "GET") return snapshot ? route.fulfill({ json: snapshot }) : route.fallback();
    if (url.pathname.startsWith("/api/onchain/credentials")) {
      const provider = providers.providers.find((row) => row.provider === body.provider)!;
      const clear = url.pathname.endsWith("/clear");
      for (const field of provider.fields) if (clear || body.fields.some((item: any) => item.key === field.key)) field.configured = !clear;
      provider.configuredCount = provider.fields.filter((field) => field.configured).length;
      provider.ready = provider.configuredCount === provider.fieldCount;
      provider.missingFields = provider.fields.filter((field) => !field.configured).map((field) => field.key) as never[];
      return route.fulfill({ json: { provider: provider.provider, label: provider.label, configuredCount: provider.configuredCount,
        fieldCount: provider.fieldCount, affectedFields: provider.fields.map((field) => field.key), missingFields: provider.missingFields,
        message: clear ? "fixture cleared" : "fixture saved", secretStorage: providers.secretStorage } });
    }
    return route.fallback();
  });
  return { calls, actions, health, providers,
    failEvidence: (key: string, value = true) => value ? fails.add(key) : fails.delete(key),
    holdEvidence: (key: string) => { releases.delete(key); holds.add(key); },
    releaseEvidence: (key: string) => { if (releases.has(key)) releases.get(key)!(); else holds.delete(key); },
  };
}

export async function settingsEvidenceFixture(page: Page, tab = "credentials") {
  const base = await settingsFixture(page, tab);
  await page.addInitScript(() => localStorage.setItem("crossline.settings.credentialTask", JSON.stringify("onchain-provider")));
  return { ...base, ...await evidenceRoutes(page) };
}
