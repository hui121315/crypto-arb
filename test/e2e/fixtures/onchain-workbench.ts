import { type Page, type WebSocketRoute } from "@playwright/test";
import { API, NOW, setup as setupBase } from "./opportunity-workbench";

export { API, NOW, WEB } from "./opportunity-workbench";

export function snapshot(at = NOW) {
  const directions = ["buy_onchain_sell_cex", "buy_cex_sell_onchain"];
  return {
    config: { enabled: true, chain: "solana", provider: "jupiter_swap_v2", poolOrRoute: "jupiter-keyless",
      baseToken: "SOL", quoteToken: "USDC", baseIdentityResolved: true, quoteIdentityResolved: true,
      baseMint: "So11111111111111111111111111111111111111112",
      quoteMint: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v", baseDecimals: 9, quoteDecimals: 6,
      baseAmountRaw: "1000000000", quoteAmountRaw: "100000000", walletAddress: "",
      cexVenue: "binance", cexSymbol: "SOL/USDC", cexTakerFeeBps: 10, gasUsd: 0.01,
      slippageBps: 10, minLiquidityUsd: 100, maxAgeMs: 10000, rpc: { mode: "provider_managed" },
      spreadAlert: { enabled: false, mode: "verified_net", minNetSpreadBps: 20, minRawSpreadBps: 20, cooldownMs: 300000 } },
    quality: "fresh", comparisons: directions.map((direction) => ({ direction, onchainPrice: 100, cexPrice: 101,
      grossSpreadBps: 100, cexFeeBps: 10, quoteConversionFeeBps: 0, slippageBps: 10, gasUsd: 0.01,
      gasBps: 1, totalCostBps: 21, netSpreadBps: 79, observableNotionalUsd: 100, executable: false })),
    quoteEvidence: [], quoteObservedAtMs: at, onchainFreshnessMs: 10, onchainLatencyMs: 100,
    quoteIntervalMs: 4500, cexSource: "ws_push", cexFreshnessMs: 10, cexObservedAtMs: at,
    projectionIntervalMs: 100, providerConfigured: true, providerProblem: null,
    rpcStatus: { mode: "provider_managed", configured: false, ready: true,
      officialDocsUrl: "https://ethereum.org/developers/docs/apis/json-rpc/" },
    cexSymbolSource: "configured", degradationReasons: [], readOnly: true,
    executionReadiness: { walletAddressConfigured: true, chainSubmissionReady: true, cexLiveModeReady: true,
      globalBlockers: [], observedAtMs: at, directions: directions.map((direction) => ({ direction,
        inventory: [], buildReady: true, submitReady: false, blockers: [], cexInstrument: {
          venue: "binance", requestedSymbol: "SOL/USDC", nativeSymbol: "SOLUSDC", status: "ready",
          ready: true, source: "fixture", observedAtMs: at, problem: null } })) },
    observedAtMs: at, batch: { items: [], maxItems: 12, estimatedSweepMs: 0, projectionIntervalMs: 250, observedAtMs: at },
  };
}

export function executionPlan(at = NOW) {
  return {
    buildId: "fixture-build", direction: "buy_cex_sell_onchain", provider: "jupiter_swap_v2", chain: "solana",
    walletAddress: "fixture-wallet", inputToken: "SOL", outputToken: "USDC",
    inputAmountRaw: "1000000000", outputAmountRaw: "101000000",
    chainTransaction: { kind: "solana_versioned", transaction_base64: "fixture-not-a-transaction", request_id: "fixture", router: "fixture", mode: "fixture" },
    cexOrder: { venue: "binance", nativeSymbol: "SOLUSDC", clientOrderId: "fixture-order", side: "buy",
      baseQuantity: 1, referencePrice: 100, estimatedQuoteAmount: 100,
      instrumentSpec: { venue: "binance", nativeSymbol: "SOLUSDC", canonicalSymbol: "SOL/USDC", displaySymbol: "SOL/USDC",
        assetClass: "crypto", listingStatus: "trading", source: "official_endpoint", checkedAtMs: at },
      sizingPlan: { targetNotionalUsd: 100, referencePrice: 100, contractSize: 1, qtyStep: 0.001,
        roundedContracts: 1, roundedBaseQty: 1, actualNotionalUsd: 100, roundingDeltaUsd: 0 } },
    estimatedNetProfitUsd: 0.79, estimatedNetSpreadBps: 79, quoteObservedAtMs: at, cexObservedAtMs: at,
    builtAtMs: at, validUntilMs: at + 20_000, officialDocsUrl: "https://example.test/fixture", buildReady: true, submitReady: true, blockers: [],
  };
}

export async function setup(page: Page, options: { holdSeed?: boolean; failSeed?: boolean; holdBuild?: boolean } = {}) {
  const base = await setupBase(page);
  const sockets = new Set<WebSocketRoute>();
  const requests: string[] = [];
  let current = snapshot();
  let releaseSeed: (() => void) | undefined;
  let holdSave = false;
  let failSave = false;
  let releaseSave: (() => void) | undefined;
  let releaseBuild: (() => void) | undefined;
  await page.routeWebSocket(/.*/, (socket) => {
    if (!socket.url().startsWith(API.replace("http:", "ws:"))) return socket.close();
    socket.onMessage((raw) => {
      const message = JSON.parse(raw.toString());
      if (message.type === "subscribe") {
        socket.send(JSON.stringify({ type: "ack", subscribed: message.channels }));
        if (message.channels.includes("onchain")) sockets.add(socket);
      } else if (message.type === "ping") socket.send(JSON.stringify({ type: "pong" }));
    });
    socket.onClose(() => sockets.delete(socket));
  });
  await page.route(`${API}/api/onchain/**`, async (route) => {
    const path = new URL(route.request().url()).pathname;
    requests.push(`${route.request().method()} ${path}`);
    if (path === "/api/onchain/comparison") {
      const captured = structuredClone(current);
      if (options.holdSeed) await new Promise<void>((resolve) => { releaseSeed = resolve; });
      if (options.failSeed) return route.fulfill({ status: 503, json: { code: "FIXTURE_UNAVAILABLE", message: "fixture: snapshot unavailable" } });
      return route.fulfill({ json: captured });
    }
    if (path === "/api/onchain/comparison/config") {
      const patch = route.request().postDataJSON();
      if (holdSave) await new Promise<void>((resolve) => { releaseSave = resolve; });
      if (failSave) return route.fulfill({ status: 503, json: { code: "SAVE_FAILED", message: "fixture: configuration not saved" } });
      current = snapshot(current.observedAtMs + 10);
      Object.assign(current.config, Object.fromEntries(Object.entries(patch).filter(([, value]) => value !== null)));
      if (!current.config.enabled) current.quality = "disabled";
      return route.fulfill({ json: current });
    }
    if (path === "/api/onchain/comparison/refresh") return route.fulfill({ json: current });
    if (path === "/api/onchain/execution/build") {
      const plan = executionPlan(current.observedAtMs);
      if (options.holdBuild) await new Promise<void>((resolve) => { releaseBuild = resolve; });
      return route.fulfill({ json: plan });
    }
    if (path === "/api/onchain/cex-pairs") return route.fulfill({ json: { venue: "binance", baseToken: "SOL", problem: null,
      pairs: [{ venue: "binance", baseToken: "SOL", quoteToken: "USDC", cexSymbol: "SOL/USDC",
        nativeSymbol: "SOLUSDC", quality: "fresh", source: "ws_push", freshnessMs: 10, observedAtMs: NOW }] } });
    if (path.endsWith("/runs")) return route.fulfill({ json: { rows: [], recoveryProblem: null, costOwners: {}, observedAtMs: NOW } });
    if (path === "/api/onchain/credentials") return route.fulfill({ json: { providers: [] } });
    // Execution, token resolution and transfers never reach a real service in this fixture.
    return route.fulfill({ status: 409, json: { code: "ISOLATED_TEST", message: "fixture: action disabled" } });
  });
  const emit = (next: ReturnType<typeof snapshot>) => sockets.forEach((socket) => socket.send(JSON.stringify({
    type: "message", channel: "onchain", payload: next,
  })));
  return { ...base, sockets, requests, emit,
    failStream: () => sockets.forEach((socket) => socket.send(JSON.stringify({
      type: "error", channel: "onchain", code: "FIXTURE_OFFLINE", message: "fixture: onchain disconnected", retryAfterMs: 60000,
    }))),
    releaseSeed: () => { options.holdSeed = false; releaseSeed?.(); },
    holdSave: (fail = false) => { holdSave = true; failSave = fail; },
    releaseSave: () => { holdSave = false; releaseSave?.(); },
    releaseBuild: () => { options.holdBuild = false; releaseBuild?.(); },
    tick: () => { current = snapshot(current.observedAtMs + 1); emit(current); },
  };
}
