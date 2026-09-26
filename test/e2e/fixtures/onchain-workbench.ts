import { type Page, type WebSocketRoute } from "@playwright/test";
import { API, NOW, setup as setupBase } from "./opportunity-workbench";
import { cyclePlan, cycleRun, progressCycle } from "./cross-chain-cycle";

export { API, NOW, WEB } from "./opportunity-workbench";

function marketConfig() {
  return { enabled: true, chain: "solana", provider: "jupiter_swap_v2", poolOrRoute: "jupiter-keyless",
      baseToken: "SOL", quoteToken: "USDC", baseIdentityResolved: true, quoteIdentityResolved: true,
      baseMint: "So11111111111111111111111111111111111111112",
      quoteMint: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v", baseDecimals: 9, quoteDecimals: 6,
      baseAmountRaw: "1000000000", quoteAmountRaw: "100000000", walletAddress: "",
      cexVenue: "binance", cexSymbol: "SOL/USDC", cexTakerFeeBps: 10, gasUsd: 0.01,
      slippageBps: 10, minLiquidityUsd: 100, maxAgeMs: 10000, rpc: { mode: "provider_managed" },
      spreadAlert: { enabled: false, mode: "verified_net", minNetSpreadBps: 20, minRawSpreadBps: 20, cooldownMs: 300000 } };
}

function mergeConfigPatch(config: ReturnType<typeof marketConfig>, patch: Record<string, unknown>) {
  const defined = (value: Record<string, unknown>) => Object.fromEntries(Object.entries(value).filter(([, field]) => field != null));
  const { source, spreadAlert, dexComparison, crossChain, ...fields } = patch;
  Object.assign(config, defined(fields));
  if (source) {
    const { provider, rpcMode } = defined(source as Record<string, unknown>);
    if (provider) config.provider = String(provider);
    if (rpcMode) config.rpc.mode = String(rpcMode);
  }
  for (const [key, value] of Object.entries({ spreadAlert, dexComparison, crossChain })) {
    if (value) Object.assign(config, { [key]: { ...Reflect.get(config, key), ...defined(value as Record<string, unknown>) } });
  }
}

export function batchItem(itemId = "fixture-peer", symbol = "ETH") {
  return { itemId, config: { ...marketConfig(), chain: "base", provider: "cow_protocol",
    baseToken: symbol, baseMint: "0x0000000000000000000000000000000000000001",
    quoteMint: "0x0000000000000000000000000000000000000002", baseDecimals: 18,
    cexSymbol: `${symbol}/USDC`, baseAmountRaw: "1000000000000000000" }, quality: "fresh",
    bestDirection: "buy_cex_sell_onchain", bestGrossSpreadBps: 100, bestNetSpreadBps: 79,
    observableNotionalUsd: 100, quoteObservedAtMs: NOW, onchainFreshnessMs: 10, onchainLatencyMs: 100,
    quoteIntervalMs: 4500, cexSource: "ws_push", cexFreshnessMs: 10, cexObservedAtMs: NOW,
    providerConfigured: true, providerProblem: null, degradationReasons: ["fixture: monitoring only"], observedAtMs: NOW };
}

export function snapshot(at = NOW) {
  const directions = ["buy_onchain_sell_cex", "buy_cex_sell_onchain"];
  return {
    config: marketConfig(),
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
    observedAtMs: at, batch: { items: [] as ReturnType<typeof batchItem>[], maxItems: 12, estimatedSweepMs: 0, projectionIntervalMs: 250, observedAtMs: at },
  };
}

export function executionPlan(at = NOW, direction = "buy_cex_sell_onchain") {
  const buyOnchain = direction === "buy_onchain_sell_cex";
  return {
    buildId: "fixture-build", direction, provider: "jupiter_swap_v2", chain: "solana",
    walletAddress: "fixture-wallet", inputToken: buyOnchain ? "USDC" : "SOL", outputToken: buyOnchain ? "SOL" : "USDC",
    inputAmountRaw: buyOnchain ? "100000000" : "1000000000", outputAmountRaw: buyOnchain ? "1000000000" : "101000000",
    chainTransaction: { kind: "solana_versioned", transaction_base64: "fixture-not-a-transaction", request_id: "fixture", router: "fixture", mode: "fixture" },
    cexOrder: { venue: "binance", nativeSymbol: "SOLUSDC", clientOrderId: "fixture-order", side: buyOnchain ? "sell" : "buy",
      baseQuantity: 1, referencePrice: 100, estimatedQuoteAmount: 100,
      instrumentSpec: { venue: "binance", nativeSymbol: "SOLUSDC", canonicalSymbol: "SOL/USDC", displaySymbol: "SOL/USDC",
        assetClass: "crypto", listingStatus: "trading", source: "official_endpoint", checkedAtMs: at },
      sizingPlan: { targetNotionalUsd: 100, referencePrice: 100, contractSize: 1, qtyStep: 0.001,
        roundedContracts: 1, roundedBaseQty: 1, actualNotionalUsd: 100, roundingDeltaUsd: 0 } },
    estimatedNetProfitUsd: 0.79, estimatedNetSpreadBps: 79, quoteObservedAtMs: at, cexObservedAtMs: at,
    builtAtMs: at, validUntilMs: at + 20_000, officialDocsUrl: "https://example.test/fixture", buildReady: true, submitReady: true, blockers: [],
  };
}

export function executionRun(status = "executing", buildId = "fixture-build", at = NOW) {
  return { runId: `run-${buildId}`, buildId, status, cexOrderId: "fixture-order",
    cexOrderState: null, cexFilledQuantity: status === "completed" ? 1 : null,
    chainTransactionId: status === "completed" ? "fixture-chain-receipt" : null,
    compensationOrderId: null, legs: [], recoveryActions: [], replenishmentCosts: [], approvalCosts: [],
    estimatedNetProfitUsd: 0.79, remainingExposureUsd: 0, quantityReconciled: false, accounting: null,
    message: status === "completed" ? "fixture: 双腿终态已回报，结算待核对" : "fixture: 请求已接收，等待双腿终态",
    problem: null, startedAtMs: NOW, updatedAtMs: at };
}

export function replenishmentPlan(at = NOW) {
  return { planId: "fixture-restock-plan", direction: "buy_cex_sell_onchain", status: "ready_for_authorization",
    builtAtMs: at, validUntilMs: at + 60_000, requiresLiveAuthorization: true, submitReady: true, blockers: [],
    transferCostUsd: 0.1, postTransferNetProfitUsd: null,
    legs: [{ direction: "deposit_to_cex", venue: "binance", asset: "USDC", chain: "solana", assetDecimals: 6,
      transferAmount: 12.5, transferAmountExact: "12.5", economics: {},
      networkEvidence: { network: "SOL", transferStatus: "ready" }, destination: { address: "fixture-address", status: "verified" } }] };
}

export function replenishmentRun(at = NOW) {
  return { runId: "fixture-restock-run", plan: replenishmentPlan(at), idempotencyKey: "fixture-key",
    status: "authorized_awaiting_submit", authorization: { actor: "fixture", authorizedAtMs: at,
      validUntilMs: at + 60_000, confirmationVersion: "fixture" }, transfers: [], createdAtMs: at,
    updatedAtMs: at, nextAction: "等待提交原资金动作", problem: null };
}

function crossChainPlan(at = NOW) {
  return { buildId: "fixture-cross-plan", provider: "lifi", sourceChain: "solana", peerChain: "base", legs: [],
    initialQuoteAmountRaw: "1000000", finalQuoteAmountRaw: "1010000", quoteObservedAtMs: at, builtAtMs: at,
    validUntilMs: at + 60_000, atomic: false, monitorOnly: false, previewReady: true, submitReady: true,
    quoteUsdValuation: { asset: "USDC", venue: "kraken", symbol: "USDC/USD", source: "ws_push", usdBid: 1, usdAsk: 1, observedAtMs: at } };
}

export function crossChainRun(at = NOW) {
  const asset = { chain: "base", wallet: "fixture-wallet", asset: { symbol: "USDC", address: "fixture-token", decimals: 6 }, amountExact: "12.5" };
  return { runId: "fixture-cross-run", build: crossChainPlan(at), idempotencyKey: "fixture-cross-key", status: "paused",
    authorization: { actor: "fixture", authorizedAtMs: at - 120_000, validUntilMs: at - 60_000, confirmationVersion: "fixture" },
    legs: [], activePosition: null, createdAtMs: at - 120_000, updatedAtMs: at, nextAction: "fixture: 原路径停止，核对剩余资金", problem: null,
    accounting: { status: "pending_receipts", flows: [], netAssets: [], problems: [], disposition: {
      sourceRunId: "fixture-cross-run", receiptsObservedAtMs: at, originalCapital: asset,
      remainingAssets: [{ change: asset, action: "quote_swap" }], blockers: [], submitReady: false, requiresLiveAuthorization: true } } };
}

export function recoveryPlan(at = NOW) {
  const input = crossChainRun(at).accounting.disposition.originalCapital;
  return { planId: "fixture-recovery-plan", status: "awaiting_authorization", createdAtMs: at, updatedAtMs: at,
    preview: { planId: "fixture-recovery-plan", sourceRunId: "fixture-cross-run", sourceRunUpdatedAtMs: at, assetIndex: 0,
      input, target: input, inputAmountRaw: "12500000", balanceAmountRaw: "20000000", balanceCheckedAtMs: at,
      routeId: "fixture-route", provider: "lifi", minimumOutputAmountRaw: "12400000", expectedOutputAmountRaw: "12450000",
      feeUsd: 0.01, gasUsd: 0.01, validUntilMs: at + 20_000, blockers: [], quoteReady: true,
      submitReady: false, requiresLiveAuthorization: true, officialDocsUrl: "https://docs.li.fi" } };
}

export async function setup(page: Page, options: { holdSeed?: boolean; failSeed?: boolean; holdBuild?: boolean;
  scenario?: "replenishment" | "cross_chain"; holdPlan?: boolean; authorizedRun?: boolean; failSubmit?: boolean; lostAuthorization?: boolean;
  crossRecovery?: boolean; savedRecovery?: boolean; holdRecovery?: boolean; simulateRecoveryMutation?: boolean;
  execution?: "ack" | "lost_reply" | "unknown" | "reject"; failExecutionRead?: boolean;
  crossCycle?: boolean; crossSubmitMode?: "unknown" | "reject"; crossAuthorizeLost?: boolean } = {}) {
  const base = await setupBase(page);
  const sockets = new Set<WebSocketRoute>();
  const requests: string[] = [];
  const scenarioSnapshot = (at = NOW) => {
    const value = snapshot(at);
    if (options.scenario === "replenishment") for (const row of value.executionReadiness.directions) {
      row.buildReady = false;
      Object.assign(row, { path: { kind: "direct_two_leg", availability: "replenishable", summary: "fixture: 可补仓", legs: [], replenishment: [] } });
    }
    if (options.scenario === "cross_chain") {
      Object.assign(value.config, { crossChain: { enabled: true, peerItemId: "fixture-peer", provider: "lifi", stablecoinRiskBps: 50 } });
      Object.assign(value, { crossChain: { provider: "lifi", peerItemId: "fixture-peer", peerChain: "base", quality: "fresh",
        legs: options.crossCycle ? cyclePlan().legs : [], atomic: false, previewReady: true, submitReady: true, quoteObservedAtMs: at, observedAtMs: at } });
      if (options.crossCycle) value.batch.items = [batchItem("fixture-peer", "SOL")];
    }
    return value;
  };
  let current = scenarioSnapshot();
  let restockRows = options.authorizedRun ? [replenishmentRun()] : [];
  let crossRows = options.crossRecovery ? [crossChainRun()] : [];
  let cycle: ReturnType<typeof cycleRun> | undefined;
  let recoveryPlans = options.savedRecovery ? [recoveryPlan()] : [];
  let executionRows: ReturnType<typeof executionRun>[] = [];
  let buildCount = 0;
  let holdExecutionRead = false;
  let releaseExecutionRead: (() => void) | undefined;
  let releaseRecovery: (() => void) | undefined;
  let failCrossRead = false;
  const seedWaiters: (() => void)[] = [];
  let holdSave = false;
  let failSave = false;
  let releaseSave: (() => void) | undefined;
  let releaseBuild: (() => void) | undefined;
  let releasePlan: (() => void) | undefined;
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
      if (options.holdSeed) await new Promise<void>((resolve) => { seedWaiters.push(resolve); });
      if (options.failSeed) return route.fulfill({ status: 503, json: { code: "FIXTURE_UNAVAILABLE", message: "fixture: snapshot unavailable" } });
      return route.fulfill({ json: captured });
    }
    if (path === "/api/onchain/comparison/config") {
      const patch = route.request().postDataJSON();
      if (holdSave) await new Promise<void>((resolve) => { releaseSave = resolve; });
      if (failSave) return route.fulfill({ status: 400, json: { code: "SAVE_REJECTED", message: "fixture: configuration not saved" } });
      const previous = current;
      current = scenarioSnapshot(current.observedAtMs + 10);
      current.config = previous.config;
      current.batch = previous.batch;
      mergeConfigPatch(current.config, patch);
      if (!current.config.enabled) current.quality = "disabled";
      return route.fulfill({ json: current });
    }
    if (path === "/api/onchain/comparison/refresh") return route.fulfill({ json: current });
    if (path === "/api/onchain/comparison/batch") {
      if (route.request().method() === "POST") {
        const item = batchItem(`fixture-added-${current.batch.items.length}`, current.config.baseToken);
        Object.assign(item.config, current.config);
        mergeConfigPatch(item.config, route.request().postDataJSON());
        current.batch.items.push(item);
        current.batch.observedAtMs += 1;
      }
      return route.fulfill({ json: current.batch });
    }
    if (path === "/api/onchain/comparison/batch/remove") {
      current.batch.items = current.batch.items.filter((item) => item.itemId !== route.request().postDataJSON().itemId);
      current.batch.observedAtMs += 1;
      return route.fulfill({ json: current.batch });
    }
    if (path === "/api/onchain/execution/build") {
      const plan = executionPlan(current.observedAtMs, route.request().postDataJSON().direction);
      if (options.execution) {
        buildCount += 1;
        plan.buildId = buildCount === 1 ? "fixture-build" : `fixture-build-${buildCount}`;
        Object.assign(plan, { provider: current.config.provider, chain: current.config.chain,
          inputToken: plan.direction === "buy_onchain_sell_cex" ? current.config.quoteToken : current.config.baseToken });
        Object.assign(plan.cexOrder, { venue: current.config.cexVenue, nativeSymbol: current.config.cexSymbol });
      }
      if (options.holdBuild) await new Promise<void>((resolve) => { releaseBuild = resolve; });
      return route.fulfill({ json: plan });
    }
    if (path === "/api/onchain/execution/runs") {
      const captured = structuredClone(executionRows);
      if (holdExecutionRead) await new Promise<void>((resolve) => { releaseExecutionRead = resolve; });
      if (options.failExecutionRead) return route.fulfill({ status: 503, json: { error: { code: "FIXTURE_OFFLINE", message: "fixture: execution records unavailable" } } });
      return route.fulfill({ json: { rows: captured, observedAtMs: NOW, recoveryProblem: null } });
    }
    if (path === "/api/onchain/execution/submit" && options.execution) {
      if (options.execution === "reject") return route.fulfill({ status: 409, json: { error: { code: "ONCHAIN_BUILD_EXPIRED", message: "fixture: plan expired" } } });
      const run = executionRun("executing", route.request().postDataJSON().buildId);
      if (options.execution !== "unknown") executionRows = [run, ...executionRows.filter((row) => row.buildId !== run.buildId)];
      if (options.execution !== "ack") return route.fulfill({ status: 504, json: { error: { code: "TIMEOUT", message: "fixture: execution reply missing" } } });
      return route.fulfill({ json: run });
    }
    if (path === "/api/onchain/replenishment/build" || path === "/api/onchain/cross-chain/build") {
      const plan = path.includes("replenishment") ? replenishmentPlan(current.observedAtMs) : options.crossCycle ? cyclePlan() : crossChainPlan(current.observedAtMs);
      if ("direction" in plan) plan.direction = route.request().postDataJSON().direction;
      if (options.holdPlan) await new Promise<void>((resolve) => { releasePlan = resolve; });
      return route.fulfill({ json: plan });
    }
    if (path === "/api/onchain/replenishment/runs") return route.fulfill({ json: { rows: restockRows, observedAtMs: NOW, recoveryProblem: null } });
    if (path === "/api/onchain/cross-chain/runs") {
      if (failCrossRead) return route.fulfill({ status: 503, json: { code: "FIXTURE_OFFLINE", message: "fixture: runs unavailable" } });
      return route.fulfill({ json: { rows: cycle ? [cycle, ...crossRows] : crossRows, recoveryPlans, observedAtMs: NOW, recoveryProblem: null } });
    }
    if (options.crossCycle && path === "/api/onchain/cross-chain/authorize") {
      cycle = cycleRun(route.request().postDataJSON().idempotencyKey);
      if (options.crossAuthorizeLost) return route.fulfill({ status: 504, json: { error: { code: "TIMEOUT", message: "fixture: authorization reply missing" } } });
      return route.fulfill({ json: cycle });
    }
    if (options.crossCycle && cycle && path === "/api/onchain/cross-chain/submit") {
      if (options.crossSubmitMode) return route.fulfill({ status: options.crossSubmitMode === "reject" ? 409 : 504,
        json: { error: { code: options.crossSubmitMode === "reject" ? "ONCHAIN_CROSS_CHAIN_PRE_TRADE_REJECTED" : "TIMEOUT", message: "fixture: step not acknowledged" } } });
      progressCycle(cycle, route.request().postDataJSON().expectedPosition, "submitted");
      return route.fulfill({ json: cycle });
    }
    if (options.crossCycle && cycle && path === "/api/onchain/cross-chain/recheck") {
      progressCycle(cycle, route.request().postDataJSON().expectedPosition, "source_confirmed");
      return route.fulfill({ json: cycle });
    }
    if (path === "/api/onchain/cross-chain/recovery/preview") {
      const request = route.request().postDataJSON();
      const preview = recoveryPlan().preview;
      preview.sourceRunUpdatedAtMs = request.expectedRunUpdatedAtMs;
      preview.input = { ...preview.input, amountExact: request.amountExact };
      preview.inputAmountRaw = (Number(request.amountExact) * 1_000_000).toFixed(0);
      preview.minimumOutputAmountRaw = (Number(request.amountExact) * 990_000).toFixed(0);
      preview.expectedOutputAmountRaw = (Number(request.amountExact) * 995_000).toFixed(0);
      if (options.holdRecovery) await new Promise<void>((resolve) => { releaseRecovery = resolve; });
      return route.fulfill({ json: preview });
    }
    if (options.simulateRecoveryMutation && (path === "/api/onchain/cross-chain/recovery/reserve" || path === "/api/onchain/cross-chain/recovery/cancel")) {
      const record = recoveryPlans.find((plan) => plan.planId === route.request().postDataJSON().planId);
      if (!record) return route.fulfill({ status: 404, json: { code: "FIXTURE_MISSING", message: "fixture: plan missing" } });
      record.status = path.endsWith("/reserve") ? "reserved" : "cancelled";
      record.updatedAtMs += 1;
      return route.fulfill({ json: record });
    }
    if (path === "/api/onchain/replenishment/authorize" && options.lostAuthorization) {
      const record = replenishmentRun();
      record.idempotencyKey = route.request().postDataJSON().idempotencyKey;
      restockRows = [record];
      return route.fulfill({ status: 504, json: { code: "TIMEOUT", message: "fixture: authorization reply missing" } });
    }
    if (path === "/api/onchain/replenishment/submit" && options.failSubmit)
      return route.fulfill({ status: 504, json: { code: "TIMEOUT", message: "fixture: submit reply missing" } });
    if (path === "/api/onchain/cex-pairs") {
      const query = new URL(route.request().url()).searchParams;
      const venue = query.get("venue") || "binance";
      const baseToken = query.get("baseToken") || "SOL";
      return route.fulfill({ json: { venue, baseToken, problem: null,
        pairs: [{ venue, baseToken, quoteToken: "USDC", cexSymbol: `${baseToken}/USDC`,
          nativeSymbol: `${baseToken}USDC`, quality: "fresh", source: "ws_push", freshnessMs: 10, observedAtMs: NOW }] } });
    }
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
    releaseSeed: () => { options.holdSeed = false; seedWaiters.splice(0).forEach((resolve) => resolve()); },
    holdSave: (fail = false) => { holdSave = true; failSave = fail; },
    releaseSave: () => { holdSave = false; releaseSave?.(); },
    holdBuild: () => { options.holdBuild = true; },
    releaseBuild: () => { options.holdBuild = false; releaseBuild?.(); },
    setExecutionRows: (rows: ReturnType<typeof executionRun>[]) => { executionRows = rows; },
    failExecutionRead: (fail = true) => { options.failExecutionRead = fail; },
    holdExecutionRead: () => { holdExecutionRead = true; },
    releaseExecutionRead: () => { holdExecutionRead = false; releaseExecutionRead?.(); },
    releasePlan: () => { options.holdPlan = false; releasePlan?.(); },
    holdPlan: () => { options.holdPlan = true; },
    setRestockRows: (rows: ReturnType<typeof replenishmentRun>[]) => { restockRows = rows; },
    setCrossRows: (rows: ReturnType<typeof crossChainRun>[]) => { crossRows = rows; },
    progressCycle: (position: number, stage: Parameters<typeof progressCycle>[2]) => { if (cycle) progressCycle(cycle, position, stage); },
    reviseEarlierCycleReceipt: () => { if (cycle) { cycle.updatedAtMs += 1; cycle.legs[0].evidenceSource = "fixture: late fee receipt"; } },
    crossSubmitMode: (mode?: "unknown" | "reject") => { options.crossSubmitMode = mode; },
    setRecoveryPlans: (plans: ReturnType<typeof recoveryPlan>[]) => { recoveryPlans = plans; },
    failCrossRead: (fail = true) => { failCrossRead = fail; },
    releaseRecovery: () => { options.holdRecovery = false; releaseRecovery?.(); },
    setBatchItems: (items: ReturnType<typeof batchItem>[]) => {
      current = { ...current, observedAtMs: current.observedAtMs + 1,
        batch: { ...current.batch, items: structuredClone(items), observedAtMs: current.observedAtMs + 1 } };
      emit(current);
    },
    tick: () => {
      const previous = current;
      current = scenarioSnapshot(previous.observedAtMs + 1);
      current.config = previous.config;
      current.batch = previous.batch;
      if (!current.config.enabled) current.quality = "disabled";
      emit(current);
    },
  };
}
