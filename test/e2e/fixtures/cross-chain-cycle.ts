import { NOW } from "./opportunity-workbench";

export function cyclePlan() {
  const routes = [
    ["source_swap", "solana", "solana", "USDC", "SOL", "100000000", "1000000000", 6, 9],
    ["outbound_bridge", "solana", "base", "SOL", "WSOL", "1000000000", "995000000000000000", 9, 18],
    ["target_swap", "base", "base", "WSOL", "USDC", "995000000000000000", "104000000", 18, 6],
    ["return_bridge", "base", "solana", "USDC", "USDC", "104000000", "103000000", 6, 6],
  ] as const;
  return { buildId: "fixture-cycle-plan", provider: "lifi", sourceChain: "solana", peerChain: "base",
    legs: routes.map(([kind, fromChain, toChain, fromAsset, toAsset, inputAmountRaw, expectedOutputAmountRaw, inputDecimals, outputDecimals], index) => ({
      position: index + 1, kind, provider: "fixture", fromChain, toChain, fromAsset, toAsset,
      fromToken: `fixture-${fromChain}-${fromAsset}`, toToken: `fixture-${toChain}-${toAsset}`,
      inputAmountRaw, expectedOutputAmountRaw, minimumOutputAmountRaw: expectedOutputAmountRaw,
      inputDecimals, outputDecimals, feeUsd: 0.25, gasUsd: 0.01, estimatedDurationSeconds: 30,
      routeId: `fixture-route-${index}`, routeTools: ["fixture"], officialDocsUrl: "https://example.test/cross-chain", observedAtMs: NOW,
    })),
    initialQuoteAmountRaw: "100000000", finalQuoteAmountRaw: "103000000", bridgeFeeUsd: 1, gasUsd: 0.04,
    netReturnBps: 196, quoteObservedAtMs: NOW, builtAtMs: NOW, validUntilMs: NOW + 60_000,
    atomic: false, monitorOnly: false, previewReady: true, submitReady: true, blockers: [], warnings: [],
    quoteUsdValuation: { asset: "USDC", venue: "kraken", symbol: "USDC/USD", source: "ws_push", usdBid: 1, usdAsk: 1, observedAtMs: NOW } };
}

export function cycleRun(key = "onchain-cross-chain-fixture-cycle-plan") {
  const build = cyclePlan();
  return { runId: "fixture-cycle-run", build, idempotencyKey: key, status: "authorized_awaiting_submit",
    authorization: { actor: "fixture", authorizedAtMs: NOW, validUntilMs: NOW + 60_000, confirmationVersion: "fixture" },
    activePosition: null as number | null,
    legs: build.legs.map((leg) => ({ position: leg.position, kind: leg.kind, clientActionId: `fixture-action-${leg.position}`,
      status: "requote_required", attempts: 0, plannedInputAmountRaw: leg.inputAmountRaw,
      submittedInputAmountRaw: null as string | null, actualInputAmountRaw: null as string | null,
      actualOutputAmountRaw: null as string | null, sourceTransactionId: null as string | null,
      sourceSubmittedAtMs: null as number | null, destinationTransactionId: null as string | null,
      lastCheckedAtMs: null as number | null, evidenceSource: null as string | null, problem: null as string | null })),
    createdAtMs: NOW, updatedAtMs: NOW, nextAction: "确认第 1 步后再继续", problem: null as string | null };
}

export function progressCycle(run: ReturnType<typeof cycleRun>, position: number, stage: "submitted" | "source_confirmed" | "completed" | "paused") {
  const leg = run.legs[position - 1];
  const contract = run.build.legs[position - 1];
  leg.status = stage;
  leg.attempts = 1;
  leg.submittedInputAmountRaw = contract.inputAmountRaw;
  leg.sourceTransactionId = `fixture-source-tx-${position}`;
  leg.sourceSubmittedAtMs = NOW;
  leg.lastCheckedAtMs = ++run.updatedAtMs;
  run.activePosition = position;
  run.status = stage === "source_confirmed" ? "awaiting_destination_evidence" : stage === "paused" ? "paused" : "awaiting_source_finality";
  run.nextAction = stage === "source_confirmed" ? "源链已确认，等待目标链真实到账" : "正在核对原交易，不重复提交";
  if (stage === "completed") {
    leg.actualInputAmountRaw = contract.inputAmountRaw;
    leg.actualOutputAmountRaw = contract.expectedOutputAmountRaw;
    if (position === 2 || position === 4) leg.destinationTransactionId = `fixture-destination-tx-${position}`;
    run.activePosition = null;
    run.status = position === 4 ? "completed" : "running";
    run.nextAction = position === 4 ? "资产路径已完成，等待费用与收支核对" : `确认第 ${position + 1} 步后再继续`;
  }
}
