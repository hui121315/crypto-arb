import { expect, test, type Page, type WebSocketRoute } from "@playwright/test";
import { readFileSync } from "node:fs";

const fixture = JSON.parse(readFileSync(new URL("../../shared-types/fixtures/stocks_plan_build.json", import.meta.url), "utf8"));
const peerFixture = JSON.parse(readFileSync(new URL("../../shared-types/fixtures/stocks_peer_settlement.json", import.meta.url), "utf8"));
const fundingFixture = JSON.parse(readFileSync(new URL("../../shared-types/fixtures/stocks_funding_transfer_conflict.json", import.meta.url), "utf8"));
const scanFixture = JSON.parse(readFileSync(new URL("../../shared-types/fixtures/stocks_funding_deposit_scan.json", import.meta.url), "utf8"));
const sizingFixture = JSON.parse(readFileSync(new URL("../../shared-types/fixtures/stocks_exchange_conversion_sizing.json", import.meta.url), "utf8"));
const recoveryFixture = JSON.parse(readFileSync(new URL("../../shared-types/fixtures/stocks_exchange_conversion_recovery.json", import.meta.url), "utf8"));
const costFixture = JSON.parse(readFileSync(new URL("../../shared-types/fixtures/stocks_conversion_costs.json", import.meta.url), "utf8"));
const restockFixture = JSON.parse(readFileSync(new URL("../../shared-types/fixtures/stocks_restock.json", import.meta.url), "utf8"));
const CONVERSION_NOW = Math.max(sizingFixture.sizing.checkedAtMs, sizingFixture.saved.observedAtMs,
  sizingFixture.saved.exchangeConversions[0].updatedAtMs) + 10;

// Captured from stock_plan_build_one_pass with mock inventory, cost RPC and a real local journal.
// All application HTTP/WS are intercepted. No account, signer or webhook can be reached.
const API = "http://127.0.0.1:18997";
const NOW = fixture.observedAtMs;
const WALLET = fixture.plans[0].request.walletAddress;

async function showSection(page: Page, name: "行情与提醒" | "跨所对比" | "库存与成本" | "执行记录", openRecords = true) {
  await page.getByRole("navigation", { name: "股票详情视图" }).getByRole("button", { name, exact: true }).click();
  // Financial scenarios explicitly open a record; layout scenarios inspect the collapsed default.
  if (name === "执行记录" && openRecords) {
    for (const region of await page.locator(".stock-plans, .stock-peer-plans, .stock-funding-plans").all()) {
      if (!await region.isVisible() || await region.locator("article").count()) continue;
      const first = region.getByRole("table").getByRole("button", { name: /^查看 / }).first();
      if (await first.count()) await first.click();
    }
  }
}

async function setup(page: Page, mode: "bp" | "peer" | "quote_state" | "funding" | "funding_scan" | "conversion" | "conversion_recovery" | "conversion_costs" | "restock" = "bp") {
  let buildReply: Promise<void> | undefined;
  let loseBuildReply = false;
  let mismatchBuildReply = false;
  let mismatchPreflight = false;
  let preflightReply: Promise<void> | undefined;
  let heldAction: { path: string; reply: Promise<void>; fail: boolean } | undefined;
  let mismatchStablecoin = false;
  const reads: { path: string; token?: string }[] = [];
  const buildActions: any[] = [];
  let quoteReply: Promise<void> | undefined;
  let rejectPeerBuild = false;
  let losePeerBuild = false;
  let mismatchPeerBuild = false;
  let failPeerRead = false;
  let failStablecoinRead = false;
  let stablecoinReadReply: Promise<void> | undefined;
  let failWatch = false;
  const restock = mode === "restock";
  const costs = mode === "conversion_costs";
    const peer = mode === "peer";
  const recovery = mode === "conversion_recovery";
  const conversion = mode === "conversion" || recovery;
  const pagination = mode === "funding_scan";
  const funding = mode === "funding" || pagination;
  const market = structuredClone(restock ? restockFixture.before : costs ? costFixture.reserved : recovery ? recoveryFixture.pending : conversion ? sizingFixture.saved : pagination ? scanFixture.pending : funding ? fundingFixture : peer ? peerFixture : fixture);
  if (!restock) market.plans = [];
  if (mode === "quote_state") {
    market.peer = structuredClone(peerFixture.peerPlans[0].terms.basis.peer);
    market.peerPlans = [];
    market.peer.instrument.checkedAtMs = NOW;
    for (const q of [market.peer.quote, market.peer.quoteConversion]) {
      q.sourceAtMs = NOW; q.receivedAtMs = NOW;
    }
  }
  if (costs) market.claimedConversionCostIds = [];
  if (conversion && !recovery) market.exchangeConversions = [];
  const now = restock ? restockFixture.ready.preflight.checkedAtMs : costs ? costFixture.reserved.observedAtMs : recovery ? recoveryFixture.completed.observedAtMs + 10 : conversion ? CONVERSION_NOW : funding ? Math.max(market.observedAtMs, market.fundingPlans[0].updatedAtMs) + 6000
    : peer ? peerFixture.observedAtMs : NOW;
  // The Rust account fixture uses i64::MAX for its unrelated calendar. Give the
  // browser a finite fixture window instead of rounding it outside i64 via JS.
  if ((funding || conversion || costs) && market.tradingRoute) market.tradingRoute.validUntilMs = now + 60_000;
  const finiteWindows = (value: any) => {
    if (!value || typeof value !== "object") return;
    for (const [key, child] of Object.entries(value)) {
      if (key === "validUntilMs" && typeof child === "number" && child > Number.MAX_SAFE_INTEGER) value[key] = now + 60_000;
      else finiteWindows(child);
    }
  };
  if (restock) finiteWindows(market);
  let revision = now;
  const unarchive = () => {
    market.peerPlans = structuredClone(peerFixture.peerPlans);
    const p = market.peerPlans[0];
    p.phase = "submission_unknown";
    p.revision = p.settlement.sourceRevision;
    delete p.settlement;
    market.peerAccounting = [];
  };
  if (peer) unarchive();
  const writes: { path: string; body: any }[] = [];
  const errors: string[] = [];
  const sockets = new Set<WebSocketRoute>();
  const snapshot = () => ({ ...market, observedAtMs: ++revision });
  const publish = () => {
    const frame = JSON.stringify({ type: "message", channel: "stocks", payload: snapshot() });
    for (const socket of sockets) socket.send(frame);
  };
  await page.clock.setFixedTime(now);
  await page.addInitScript((api) => {
    (Error as any).stackTraceLimit = 80;
    localStorage.setItem("api_base", JSON.stringify(api));
    localStorage.setItem("api_auth_token", JSON.stringify("isolated-fixture-token"));
  }, API);
  page.on("pageerror", (error) => errors.push(error.stack ?? error.message));
  page.on("console", (message) => {
    if (message.type() === "error" && /panicked at|already borrowed|disposed/i.test(message.text())) errors.push(message.text());
  });
  await page.routeWebSocket(/.*/, (socket) => {
    if (!socket.url().startsWith(API.replace("http:", "ws:"))) return socket.close();
    socket.onMessage((raw) => {
      const msg = JSON.parse(raw.toString());
      if (msg.type === "subscribe") {
        socket.send(JSON.stringify({ type: "ack", subscribed: msg.channels }));
        if (msg.channels.includes("stocks")) { sockets.add(socket); publish(); }
      } else if (msg.type === "ping") socket.send(JSON.stringify({ type: "pong" }));
    });
    socket.onClose(() => sockets.delete(socket));
  });
  await page.route("**/*", async (route) => {
    const req = route.request();
    const url = new URL(req.url());
    if (!req.url().startsWith(API)) {
      if (url.hostname === "127.0.0.1" && !url.pathname.startsWith("/api/")) return route.continue();
      return route.abort();
    }
    const path = url.pathname;
    const json = async (body: any, status = 200) => {
      if (heldAction?.path === path && req.method() === "POST") {
        const held = heldAction; heldAction = undefined;
        body = structuredClone(body);
        await held.reply;
        if (held.fail) return route.fulfill({ status: 504, json: {
          error: { code: "OLD_SOURCE_TIMEOUT", message: "old source failed after account switch", status: 504 } } });
        if (body.observedAtMs) body.observedAtMs = ++revision;
      }
      return route.fulfill({ status, json: body });
    };
    if (req.method() === "GET") reads.push({ path, token: req.headers().authorization });
    if (req.method() === "OPTIONS") return route.fulfill({ status: 204 });
    if (path === "/api/auth/ws-ticket") return json({ ticket: "fixture", expiresAtMs: now + 60_000 });
    if (path === "/api/stocks/catalog") return json({ rows: [market.security ?? peerFixture.peerPlans[0].terms.basis.security], observedAtMs: now });
    if (path === "/api/stocks/peer-markets") return json({ request: Object.fromEntries(url.searchParams), rows: [], matched: 0, registryCount: 0 });
    if (path === "/api/stocks" && req.method() === "GET") return json(snapshot());
    if (path === "/api/stocks/peer/plans" && req.method() === "GET") {
      if (failPeerRead) { failPeerRead = false; return json({ error: { code: "FIXTURE_READ_FAILED", message: "当前双边计划读取失败", status: 503 } }, 503); }
      return json(snapshot());
    }
    if (path === "/api/stocks/funding/stablecoin-plans" && req.method() === "GET") {
      const waiting=stablecoinReadReply; stablecoinReadReply=undefined;
      if (waiting) await waiting;
      if (failStablecoinRead) { failStablecoinRead=false; return json({ error: {
        code: "FIXTURE_STABLECOIN_READ_FAILED", message: "兑换记录服务暂时不可达", status: 503 } }, 503); }
      return json(snapshot());
    }
    if (path === "/api/trading/action-runs") return json({ status: "ready", data: buildActions,
      problems: [], source: "fixture", observedAtMs: now });
    if (path.startsWith("/api/trading/action-runs/")) return json(buildActions.find(a => a.id === decodeURIComponent(path.split("/").at(-1)!)));
    if (req.method() === "POST" && path.startsWith("/api/stocks/")) {
      const body = req.postDataJSON();
      writes.push({ path, body });
      if (path === "/api/stocks/watch") {
        if (failWatch) { failWatch=false; return json({ error: { code:"FIXTURE_WATCH_FAILED", message:"股票详情关闭失败", status:503 } },503); }
        expect(body.asset === null || body.asset === fixture.security.asset).toBe(true);
        market.security=body.asset ? structuredClone(fixture.security) : null;
        market.comparison=null; market.preflight=null; market.peer=null;
        market.connected=false; market.books=[]; market.reference=null; market.chainCosts=[];
        market.monitor={ ...market.monitor,enabled:false,request:null,phase:"disabled" };
        return json(snapshot());
      }
      if (path === "/api/stocks/funding/stablecoin-preview") {
        const request = { ...body, walletAddress: mismatchStablecoin ? "wrong-wallet" : body.walletAddress };
        mismatchStablecoin = false;
        return json({ request, quote: { ...fixture.comparison.buy, inputRaw: "10000000", outputRaw: "10000000", minimumOutputRaw: "9990000",
          requestedAtMs: now, receivedAtMs: now },
          wallet: { owner: body.walletAddress, mint: fixture.comparison.buy.inputMint, stockRaw: "20000000", usdcRaw: "0",
            solLamports: "10000000", checkedAtMs: now, problems: [] },
          cost: null, checkedAtMs: now, validUntilMs: now + 10_000, minimumUsdc: "9.99", nativeCostUsdc: null,
          afterNativeCostUsdc: null, shortfallUsdc: "0", inputSufficient: true, blockers: ["isolated preview; no executable transaction"] });
      }
      if (mode === "quote_state" && path === "/api/stocks/quote") {
        if (quoteReply) { await quoteReply; quoteReply = undefined; }
        market.comparison.budgetUsdc = body.budgetUsdc;
        market.comparison.keyed = body.keyed;
        return json(snapshot());
      }
      if (mode === "quote_state" && path === "/api/stocks/peer/plans") {
        expect(body).toMatchObject({ asset: "MU.US", selection: market.peer.selection,
          direction: "sell", walletAddress: WALLET, inputRaw: "16000", keyed: false });
        if (rejectPeerBuild) {
          rejectPeerBuild = false;
          return json({ error: { code: "STOCK_PEER_INVENTORY", message: "双边库存不足，请重新检查库存", status: 409 } }, 409);
        }
        const p = structuredClone(peerFixture.peerPlans[0]);
        p.request = body;
        p.phase = "reserved"; p.revision = 1; p.updatedAtMs = now;
        p.terms.createdAtMs = now; p.terms.marketValidUntilMs = now + 5_000;
        p.terms.reservedUntilMs = now + 60_000;
        p.cexOrder = null; p.chainSubmission = null; p.settlement = null;
        p.recoveries = []; p.conversions = []; p.nativeTopups = []; p.inventoryOrders = [];
        market.peerPlans = [p]; market.peerAccounting = [];
        const receipt = { planId: p.planId, request: structuredClone(body), phase: p.phase, observedAtMs: ++revision };
        buildActions.push({ id: `peer-build-${buildActions.length + 1}`, kind: "stock_peer_plan_build", target: body.requestId,
          status: "succeeded", actor: "fixture", message: "reserved", startedAtMs: now, updatedAtMs: now,
          requestId: req.headers()["x-request-id"], idempotencyKey: req.headers()["idempotency-key"], result: receipt });
        if (losePeerBuild) { losePeerBuild = false; return route.abort(); }
        if (mismatchPeerBuild) { mismatchPeerBuild = false; return json({ ...receipt, request: { ...body, inputRaw: "999" } }); }
        return json(receipt);
      }
      if (mode === "quote_state" && path === "/api/stocks/peer/plans/cancel") {
        const p = market.peerPlans[0];
        expect(body).toEqual({ planId: p.planId, revision: p.revision });
        p.phase = "cancelled"; p.revision++;
        return json(snapshot());
      }
      if (restock && path === "/api/stocks/preflight") {
        const p = market.plans[0];
        expect(body).toEqual({ asset: p.request.asset, walletAddress: p.request.walletAddress, sourcePlan: { planId: p.planId, revision: p.revision } });
        market.preflight = structuredClone(restockFixture.ready.preflight);
        return json(snapshot());
      }
      if (!restock && !recovery && path === "/api/stocks/preflight") {
        const result = { ...snapshot(), preflight: { ...(market.preflight ?? fixture.preflight),
          asset: body.asset, walletAddress: mismatchPreflight ? "wrong-wallet" : body.walletAddress } };
        mismatchPreflight = false;
        const waiting = preflightReply; preflightReply = undefined;
        if (waiting) await waiting;
        // Deliberately newer: source/selection checks, not just timestamps, must reject this.
        return json({ ...result, observedAtMs: ++revision });
      }
      if (restock && path === "/api/stocks/funding/plans") {
        const p = structuredClone(restockFixture.reserved.fundingPlans[0]);
        expect(body).toEqual({ ...p.request, requestId: body.requestId });
        p.request.requestId = body.requestId;
        finiteWindows(p);
        market.fundingPlans = [p];
        return json(snapshot());
      }
      if (restock && path === "/api/stocks/funding/plans/cancel") {
        const p = market.fundingPlans[0];
        expect(body).toEqual({ planId: p.planId, revision: p.revision });
        p.phase = "cancelled"; p.revision++; p.updatedAtMs = now + 1;
        return json(snapshot());
      }
      if (recovery && path === "/api/stocks/preflight") {
        expect(body).toEqual({ asset: "MU.US", walletAddress: WALLET });
        market.preflight = structuredClone(fixture.preflight);
        market.preflight.checkedAtMs = now;
        market.preflight.validUntilMs = now + 5_000;
        market.preflight.walletAddress = body.walletAddress;
        market.preflight.directions = [];
        market.preflight.funding = [];
        market.preflight.problems = ["隔离样本：新余额已读取，未创建后续资金计划"];
        return json(snapshot());
      }
      if (conversion && path === "/api/stocks/funding/exchange-conversions/size") {
        expect(body).toEqual({ minimumUsdc: "9.98" });
        return json(sizingFixture.sizing);
      }
      if (conversion && path === "/api/stocks/funding/exchange-conversions") {
        expect(body).toMatchObject({ inputUsdt: "10", minimumUsdc: "9.98" });
        market.exchangeConversions = structuredClone(sizingFixture.saved.exchangeConversions);
        market.exchangeConversions[0].request.requestId = body.requestId;
        return json(snapshot());
      }
      if (conversion && path === "/api/stocks/funding/exchange-conversions/cancel") {
        const p = market.exchangeConversions[0];
        expect(body).toEqual({ planId: p.planId, revision: p.revision });
        p.cancelledAtMs = now + 1;
        p.updatedAtMs = now + 1;
        p.revision++;
        return json(snapshot());
      }
      if (funding && path === "/api/stocks/funding/plans/recheck") {
        expect(body).toEqual({ planId: market.fundingPlans[0].planId });
        if (pagination) {
          Object.assign(market, structuredClone(scanFixture.completed));
          if (market.tradingRoute) market.tradingRoute.validUntilMs = now + 60_000;
          return json(snapshot());
        }
        market.fundingPlans[0].revision++;
        market.fundingPlans[0].transfer.problem = "已补充原交易入账回复；此前处理结果冲突仍需人工核对，未释放占用";
        return json(snapshot());
      }
      if (peer && path === "/api/stocks/peer/plans/settle") {
        expect(body).toEqual({ planId: market.peerPlans[0].planId, revision: market.peerPlans[0].revision });
        market.peerPlans = structuredClone(peerFixture.peerPlans);
        const plan = market.peerPlans[0];
        plan.revision = body.revision + 1;
        plan.settlement.sourceRevision = body.revision;
        plan.settlement.accounting.sourceRevision = body.revision;
        market.peerAccounting = [structuredClone(plan.settlement.accounting)];
        return json(snapshot());
      }
      if (path === "/api/stocks/plans/build") {
        if (buildReply) { await buildReply; buildReply = undefined; }
        expect(body).toMatchObject({ asset: "MU.US", direction: "buy", walletAddress: WALLET, inputRaw: "10000000", keyed: false });
        const plan = structuredClone(costs ? costFixture.reserved.plans[0] : fixture.plans[0]);
        if (costs) {
          plan.terms.route.validUntilMs = now + 60_000;
          expect(body.conversionCostIds).toEqual([market.exchangeConversions[0].planId]);
          market.claimedConversionCostIds = [...body.conversionCostIds];
        }
        plan.request.requestId = body.requestId;
        plan.request.build = body;
        market.plans = [plan, ...market.plans.filter((p: any) => p.planId !== plan.planId)];
        const receipt = { planId: plan.planId, request: body, phase: plan.phase, observedAtMs: ++revision };
        buildActions.push({ id: `build-${buildActions.length + 1}`, kind: "stock_plan_build", target: body.requestId,
          status: "succeeded", actor: "fixture", message: "reserved", startedAtMs: now, updatedAtMs: now,
          requestId: req.headers()["x-request-id"], idempotencyKey: req.headers()["idempotency-key"], result: receipt });
        if (loseBuildReply) { loseBuildReply = false; return route.abort(); }
        if (mismatchBuildReply) { mismatchBuildReply = false; return json({ ...receipt, request: { ...body, asset: "WRONG.US" } }); }
        return json(receipt);
      }
      if (path === "/api/stocks/plans/cancel") {
        const plan = market.plans.find((p: any) => p.planId === body.planId);
        expect(plan).toBeDefined();
        plan.phase = "cancelled";
        plan.revision += 1;
        if (costs) market.claimedConversionCostIds = [];
        return json(snapshot());
      }
      throw new Error(`Unexpected stock mutation: ${path}`);
    }
    return json({ error: { code: "FIXTURE_NOT_CONFIGURED", message: "isolated fixture", status: 404 } }, 404);
  });
  return { writes, reads, errors, publish,
    failStablecoinRead: () => { failStablecoinRead=true; },
    holdStablecoinRead: () => { let release!: () => void; stablecoinReadReply=new Promise<void>(resolve => release=resolve); return release; },
    failWatch: () => { failWatch=true; },
    savedStablecoin: () => {
      const conversion={ asset: "MU.US",walletAddress:WALLET,inputUsdt:"10",targetUsdc:"9.9",keyed:false };
      const usdt="Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB";
      market.stablecoinPlans=[{ planId:"isolated-stablecoin-recovery",revision:1,updatedAtMs:now,
        phase:"submission_unknown",request:{ requestId:"stablecoin-original",conversion,previewAtMs:now,transactionFingerprint:"isolated-no-signature" },
        preview:{ request:conversion,quote:{ ...fixture.comparison.buy,inputMint:usdt,outputMint:fixture.comparison.buy.inputMint,inputRaw:"10000000",outputRaw:"10000000",minimumOutputRaw:"9900000" },
          wallet:{owner:WALLET,mint:usdt,stockRaw:"10000000",usdcRaw:"0",solLamports:"10000000",checkedAtMs:now,problems:[]},
          cost:null,checkedAtMs:now,validUntilMs:now+30_000,minimumUsdc:"9.9",nativeCostUsdc:null,afterNativeCostUsdc:null,
          shortfallUsdc:"0",inputSufficient:true,blockers:[] },nativeTopups:[],submission:{ submittedAtMs:now,
            walletSignature:"isolated-original-signature",transactionId:null,providerTransactionId:null,providerAcknowledged:false,
            receipt:null,recheckAttempts:0,nextRecheckAtMs:0,searchBefore:null,problem:"等待原交易处理结果" } }];
      publish();
    },
    historyRows: (archivedOnly = false) => {
      const archived = structuredClone(fixture.plans[0]);
      archived.planId = "bp-history-cancelled"; archived.phase = "cancelled";
      archived.terms.createdAtMs = now - 3000;
      const reserved = structuredClone(fixture.plans[0]);
      reserved.planId = "bp-history-reserved"; reserved.terms.reservedUntilMs = now + 60_000;
      reserved.terms.createdAtMs = now - 1000;
      const unknown = structuredClone(fixture.plans[0]);
      unknown.planId = "bp-history-unresolved"; unknown.phase = "submission_unknown";
      unknown.terms.createdAtMs = now - 2000;
      market.plans = archivedOnly ? [archived] : [archived, reserved, unknown];
      market.peerPlans = []; market.fundingPlans = [];
      if (!archivedOnly) {
        const oldPeer = structuredClone(peerFixture.peerPlans[0]);
        oldPeer.planId = "peer-history-settled";
        const pendingPeer = structuredClone(oldPeer);
        pendingPeer.planId = "peer-history-unresolved"; pendingPeer.phase = "submission_unknown";
        pendingPeer.settlement = null;
        market.peerPlans = [oldPeer, pendingPeer];
        const pendingFunding = structuredClone(fundingFixture.fundingPlans[0]);
        pendingFunding.planId = "funding-history-conflict";
        const oldFunding = structuredClone(pendingFunding);
        oldFunding.planId = "funding-history-cancelled"; oldFunding.phase = "cancelled";
        oldFunding.transfer = null; oldFunding.withdrawal = null; oldFunding.followup = null;
        market.fundingPlans = [oldFunding, pendingFunding];
      }
      publish();
    },
    rejectPeerBuild: () => { rejectPeerBuild = true; },
    losePeerBuild: () => { losePeerBuild = true; },
    mismatchPeerBuild: () => { mismatchPeerBuild = true; },
    failPeerRead: () => { failPeerRead = true; },
    cancelPeerBuild: () => { market.peerPlans[0].phase = "cancelled"; market.peerPlans[0].revision++; },
    planState: (state: "submitted" | "peer_reserved" | "peer_expired" | "funding" | "conversion" | "damaged") => {
      market.planProblem = null; market.peerPlanProblem = null;
      market.plans = []; market.peerPlans = []; market.fundingPlans = [];
      market.stablecoinPlans = []; market.exchangeConversions = [];
      if (state === "submitted") {
        market.plans = structuredClone(fixture.plans); // Reserved row must not hide a submitted peer trade.
        unarchive();
        market.peerPlans[0].terms.reservedUntilMs = now - 1;
      } else if (state === "peer_reserved" || state === "peer_expired") {
        unarchive();
        market.peerPlans[0].phase = "reserved";
        market.peerPlans[0].terms.reservedUntilMs = state === "peer_expired" ? now - 1 : now + 60_000;
        market.peerPlans[0].cexOrder = null; market.peerPlans[0].chainSubmission = null;
      } else if (state === "funding") market.fundingPlans = structuredClone(fundingFixture.fundingPlans);
      else if (state === "conversion") market.exchangeConversions = structuredClone(recoveryFixture.pending.exchangeConversions);
      else market.peerPlanProblem = "双边计划账本读取失败";
      publish();
    },
    holdAction: (path: string, fail = false) => {
      let release!: () => void;
      heldAction = { path, fail, reply: new Promise<void>(resolve => release = resolve) };
      return release;
    },
    mismatchStablecoin: () => { mismatchStablecoin = true; },
    newAccount: () => {
      market.security = { ...(market.security ?? fixture.security), asset: "SNDK.US", ticker: "SNDK", name: "Sandisk" };
      market.comparison = null; market.preflight = null;
      market.plans = []; market.peerPlans = []; market.fundingPlans = []; market.stablecoinPlans = []; market.exchangeConversions = [];
      publish();
    },
    buildActions,
    mismatchInventory: () => { mismatchPreflight = true; },
    holdInventory: () => { let release!: () => void; preflightReply = new Promise<void>(resolve => release = resolve); return release; },
    loseBuild: () => { loseBuildReply = true; },
    mismatchBuild: () => { mismatchBuildReply = true; },
    cancelSavedBuild: () => { market.plans[0].phase = "cancelled"; market.plans[0].revision++; },
    otherStock: () => { market.security = { ...market.security, asset: "SNDK.US", ticker: "SNDK", name: "Sandisk" };
      market.comparison = null; market.preflight = null; publish(); },
    holdQuote: () => {
      let release!: () => void;
      quoteReply = new Promise<void>((resolve) => { release = resolve; });
      return release;
    },
    holdBuild: () => {
      let release!: () => void;
      buildReply = new Promise<void>((resolve) => { release = resolve; });
      return release;
    },
    damage: () => { market.planProblem = "fixture journal damaged"; publish(); },
    costConflict: () => {
      market.exchangeConversions[0].order.evidenceConflict = true;
      market.exchangeConversions[0].revision++;
      publish();
    },
    conversionReceipt: (state: "pending" | "missing" | "partial" | "fee_mismatch" | "completed") => {
      market.exchangeConversions = structuredClone(recoveryFixture[state].exchangeConversions);
      // Different test scenarios may share a captured revision. Every pushed replacement
      // gets its own revision, as production journal updates do.
      market.exchangeConversions[0].revision = ++revision;
      market.preflight = null;
      publish();
    },
    conversionFunding: (target: "Solana" | "Backpack", sourceKnown = true, reserveShortfall = false) => {
      market.preflight = structuredClone(fixture.preflight);
      market.preflight.checkedAtMs = now;
      market.preflight.validUntilMs = now + 30_000;
      market.preflight.directions = [];
      market.preflight.funding = [{ direction: "buy", needs: [{
        asset: "USDC", target, source: target === "Solana" ? "Backpack" : "Solana",
        required: target === "Solana" ? reserveShortfall ? "5.98" : "10.98" : "9.98", available: "0",
        shortfall: target === "Solana" ? reserveShortfall ? "5.98" : "10.98" : "9.98",
        sourceAvailable: sourceKnown ? reserveShortfall ? "5" : "10" : null,
        sourceSpare: sourceKnown ? reserveShortfall ? "0" : "2" : null, sourceTradeReserve: "8",
        conservativeSourceBudget: reserveShortfall ? "6.98" : "11.98", sourceSufficient: sourceKnown ? false : null,
        token: null, metadataAtMs: null, blockers: ["来源不足，先核算 USDT 兑换；充提仍须单独交易检查"],
      }] }];
      publish();
    },
    missingFee: () => { market.peerPlans[0].cexOrder.fills[0].fees = null; market.peerPlans[0].revision++; publish(); },
    restorePeer: () => { unarchive(); market.peerPlans[0].revision += 2; publish(); },
    fundingUnavailable: () => {
      market.fundingPlans[0].revision++;
      market.fundingPlans[0].transfer.problem = "Backpack 原入账历史查询失败，保留占用；没有转账重试";
      publish();
    },
  };
}

test("BP closing stock keeps account records and failed recovery reachable", async ({ page }) => {
  const f=await setup(page,"conversion_recovery");
  f.failStablecoinRead();
  await page.goto("/#stocks");
  await showSection(page,"库存与成本");
  const stablecoin=page.getByRole("region",{name:"稳定币兑换记录",exact:true});
  await expect(stablecoin.getByRole("alert")).toContainText("兑换记录服务暂时不可达");
  await page.getByRole("button",{name:"关闭详情",exact:true}).click();
  await expect(page.locator(".stock-heading")).toContainText("账户与记录");
  await expect(page.locator(".stock-account-state")).toContainText("资金待核对");
  const batch=page.getByRole("region",{name:"批量链上监控",exact:true});
  await expect(batch).toBeHidden();
  await page.getByRole("navigation",{name:"股票详情视图"}).getByRole("button",{name:"市场监控",exact:true}).click();
  await expect(batch.getByLabel("批量询价金额",{exact:true})).toBeVisible();
  await page.getByRole("button",{name:"查看待处理资金",exact:true}).click();
  await expect(page.getByRole("navigation",{name:"股票详情视图"}).getByRole("button",{name:"执行记录",exact:true})).toHaveAttribute("aria-pressed","true");
  await page.getByRole("navigation",{name:"股票详情视图"}).getByRole("button",{name:"库存与成本",exact:true}).click();
  const tabs=page.getByRole("navigation",{name:"股票详情视图"});
  await expect(tabs.getByRole("button",{name:"库存与成本",exact:true})).toHaveAttribute("aria-pressed","true");
  await expect(tabs.getByRole("button",{name:"行情与提醒",exact:true})).toBeHidden();
  const conversion=page.getByRole("region",{name:"Backpack 账户兑换",exact:true});
  await expect(conversion.getByLabel("账户兑换待核对收支")).toBeVisible();
  await expect(conversion.getByRole("button",{name:"生成兑换计划",exact:true})).toBeHidden();
  await expect(stablecoin.getByRole("alert")).toContainText("兑换记录服务暂时不可达");
  const release=f.holdStablecoinRead();
  await stablecoin.getByRole("button",{name:"刷新兑换记录",exact:true}).click();
  await expect(stablecoin.getByRole("button",{name:"读取中…",exact:true})).toBeDisabled();
  await expect(stablecoin).toContainText("正在读取已保存兑换记录");
  release();
  await expect(stablecoin).toContainText("暂无已保存兑换记录");
  await expect(stablecoin.getByRole("alert")).toHaveCount(0);
  f.savedStablecoin();
  await expect(stablecoin.getByLabel("已保存兑换计划",{exact:true})).toContainText("已提交 · 待核对到账");
  await expect(stablecoin.getByRole("button",{name:"提交兑换",exact:true})).toHaveCount(0);
  await expect(stablecoin.getByRole("button",{name:"核对原交易",exact:true})).toBeEnabled();
  for (const width of [1440,390]) {
    await page.setViewportSize({width,height:1000});
    await page.evaluate(()=>window.scrollTo({top:0,behavior:"instant"}));
    expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
    await expect(tabs.getByRole("button",{name:"库存与成本",exact:true})).toBeInViewport();
    await expect(stablecoin.getByRole("button",{name:"核对原交易",exact:true})).toBeInViewport();
    await page.screenshot({path:test.info().outputPath(`bp-account-recovery-${width}.png`)});
  }
  await page.reload();
  await expect(page.locator(".stock-heading")).toContainText("账户与记录");
  await expect(tabs.getByRole("button",{name:"执行记录",exact:true})).toHaveAttribute("aria-pressed","true");
  await showSection(page,"库存与成本");
  await expect(stablecoin.getByLabel("已保存兑换计划",{exact:true})).toContainText("已提交 · 待核对到账");
  await expect(conversion.getByLabel("账户兑换待核对收支")).toBeVisible();
  expect(f.writes.map(w=>w.path)).toEqual(["/api/stocks/watch"]);
  expect(f.errors).toEqual([]);
});

test("BP close failure keeps detail while successful close and reselect preserve records", async ({ page }) => {
  const f=await setup(page);
  f.historyRows();
  await page.goto("/#stocks");
  const tabs=page.getByRole("navigation",{name:"股票详情视图"});
  await expect(tabs.getByRole("button",{name:"行情与提醒",exact:true})).toHaveAttribute("aria-pressed","true");
  f.failWatch();
  await page.getByRole("button",{name:"关闭详情",exact:true}).click();
  await expect(page.getByRole("alert").filter({hasText:"股票详情关闭失败"})).toBeVisible();
  await expect(page.locator(".stock-heading h2")).toHaveText("MU");
  await expect(tabs.getByRole("button",{name:"行情与提醒",exact:true})).toHaveAttribute("aria-pressed","true");
  await page.getByRole("button",{name:"关闭详情",exact:true}).click();
  await expect(tabs.getByRole("button",{name:"执行记录",exact:true})).toHaveAttribute("aria-pressed","true");
  const plans=page.getByRole("region",{name:"股票执行计划",exact:true});
  await expect(plans.getByRole("table").locator("tbody tr")).toHaveCount(3);
  await expect(page.locator(".stock-account-state")).toHaveText("提交待核对");
  await page.getByRole("button",{name:"选择股票",exact:true}).click();
  await page.locator(".stock-security").filter({hasText:"MU"}).click();
  await expect(page.locator(".stock-heading h2")).toHaveText("MU");
  await page.getByRole("button",{name:"完成选择",exact:true}).click();
  await expect(tabs.getByRole("button",{name:"行情与提醒",exact:true})).toHaveAttribute("aria-pressed","true");
  await showSection(page,"执行记录");
  await expect(plans.getByRole("table").locator("tbody tr")).toHaveCount(3);
  expect(f.writes.map(w=>w.path)).toEqual(["/api/stocks/watch","/api/stocks/watch","/api/stocks/watch"]);
  expect(f.writes.map(w=>w.body.asset)).toEqual([null,null,fixture.security.asset]);
  expect(f.errors).toEqual([]);
});

test("BP history uses aligned selectable records without hiding unresolved funds", async ({ page }) => {
  const f = await setup(page);
  f.historyRows();
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto("/#stocks");
  await showSection(page, "执行记录", false);
  const core = page.getByRole("region", { name: "股票执行计划", exact: true });
  const peer = page.getByRole("region", { name: "Kraken 双边计划记录", exact: true });
  const funding = page.getByRole("region", { name: "股票补库计划", exact: true });
  const table = core.getByRole("table", { name: "股票执行记录列表" });
  await expect(core.locator("article")).toHaveCount(0);
  await expect(peer.locator("article")).toHaveCount(0);
  await expect(funding.locator("article")).toHaveCount(0);
  await expect(table.locator("tbody tr").first()).toContainText("提交待核对 · 保留占用");
  await expect(core.locator(".stock-record-toolbar")).toContainText("3 / 3 笔 · 2 笔占用资金");
  await expect(peer.locator(".stock-record-toolbar")).toContainText("2 / 2 笔 · 1 笔占用资金");
  await expect(peer.getByRole("table")).toContainText("Kraken");
  await expect(peer.getByRole("table")).not.toContainText("Backpack");
  await expect(funding.getByRole("table")).toContainText("处理结果冲突 · 保留占用");
  expect(f.writes).toEqual([]);
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 1000 });
    await core.scrollIntoViewIfNeeded();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    const action = table.getByRole("button", { name: "查看 bp-history-cancelled", exact: true });
    await action.scrollIntoViewIfNeeded();
    expect(await action.evaluate(el => { const r = el.getBoundingClientRect(); return el.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2)); })).toBe(true);
    const overflows = await core.locator(".stock-record-browser td, .stock-record-toolbar input").evaluateAll(els => els.filter(e => {
      const r = e.getBoundingClientRect(); return r.width && (r.left < -1 || r.right > innerWidth + 1);
    }).map(e => e.textContent));
    expect(overflows).toEqual([]);
    await page.screenshot({ path: test.info().outputPath(`bp-history-${width}.png`) });
    for (const [name, region] of [["peer", peer], ["funding", funding]] as const) {
      await region.locator(".stock-record-browser").scrollIntoViewIfNeeded();
      expect(await region.locator(".stock-record-browser").evaluate(el => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
      await page.screenshot({ path: test.info().outputPath(`bp-history-${name}-${width}.png`) });
    }
  }
  await table.getByRole("button", { name: "查看 bp-history-cancelled", exact: true }).focus();
  await page.keyboard.press("Enter");
  await expect(core.locator("article")).toHaveAttribute("id", "stock-plan-bp-history-cancelled");
  await expect(core.locator("article").getByRole("button", { name: "取消预留", exact: true })).toBeDisabled();
  await expect(table.getByRole("button", { name: "收起 bp-history-cancelled", exact: true })).toHaveAttribute("aria-expanded", "true");
  f.publish();
  await expect(core.locator("article")).toHaveAttribute("id", "stock-plan-bp-history-cancelled");
  await core.getByRole("button", { name: "收起详情", exact: true }).click();
  await expect(core.locator("article")).toHaveCount(0);
  await expect(table.getByRole("button", { name: "查看 bp-history-cancelled", exact: true })).toBeFocused();
  f.publish();
  await expect(core.locator("article")).toHaveCount(0);
  const search = core.getByRole("searchbox", { name: "搜索股票执行记录列表" });
  await search.fill("no-such-plan");
  await expect(core.locator("article")).toHaveCount(0);
  await expect(core).toContainText("没有匹配的记录");
  await expect(core.locator(".stock-record-toolbar")).toContainText("0 / 3 笔 · 2 笔占用资金");
  await search.fill("reserved");
  await expect(core.locator("article")).toHaveCount(0);
  const reservedToggle = await table.getByRole("button", { name: "查看 bp-history-reserved", exact: true }).elementHandle();
  await table.getByRole("button", { name: "查看 bp-history-reserved", exact: true }).click();
  await expect(core.locator("article")).toHaveAttribute("id", "stock-plan-bp-history-reserved");
  await core.locator("article").getByRole("button", { name: "取消预留", exact: true }).click();
  await expect(core.locator("article")).toContainText("已取消");
  await expect(core.locator(".stock-record-toolbar")).toContainText("1 笔占用资金");
  expect(await reservedToggle!.evaluate(el => el.isConnected)).toBe(true);
  await expect(table.locator("tbody tr")).toContainText("已取消");
  await search.fill("");
  await peer.getByRole("button", { name: "查看 peer-history-settled", exact: true }).click();
  await expect(peer.locator("article")).toContainText("已结算 · 预留已释放");
  await expect(peer.getByRole("table")).toContainText("提交待核对 · 保留占用");
  await peer.getByRole("button", { name: "收起 peer-history-settled", exact: true }).click();
  await expect(peer.locator("article")).toHaveCount(0);
  await funding.getByRole("button", { name: "查看 funding-history-cancelled", exact: true }).click();
  await expect(funding.locator("article")).toContainText("已取消 · 未转账");
  await expect(funding.getByRole("button", { name: "取消补库预留", exact: true })).toBeDisabled();
  await funding.getByRole("button", { name: "收起详情", exact: true }).click();
  await expect(funding.locator("article")).toHaveCount(0);
  await expect(funding.getByRole("button", { name: "查看 funding-history-cancelled", exact: true })).toBeFocused();
  expect(f.writes.map(w => w.path)).toEqual(["/api/stocks/plans/cancel"]);
  expect(f.writes[0].body.planId).toBe("bp-history-reserved");
  expect(f.errors).toEqual([]);
});

test("BP new plan selects its exact record after browsing archived history", async ({ page }) => {
  const f = await setup(page);
  f.historyRows(true);
  await page.goto("/#stocks");
  await showSection(page, "执行记录", false);
  const core = page.getByRole("region", { name: "股票执行计划", exact: true });
  await core.getByRole("searchbox").fill("cancelled");
  await expect(core.locator("article")).toHaveCount(0);
  await core.getByRole("button", { name: "查看 bp-history-cancelled", exact: true }).click();
  await expect(core.locator("article")).toHaveAttribute("id", "stock-plan-bp-history-cancelled");
  await showSection(page, "库存与成本");
  const inventory = page.getByRole("region", { name: "股票库存与成本交易检查" });
  await inventory.getByLabel("股票套利 Solana 钱包地址").fill(WALLET);
  await inventory.getByRole("button", { name: "构建并预留", exact: true }).first().click();
  await expect(core).toBeVisible();
  await expect(core.getByRole("searchbox")).toHaveValue("");
  await expect(core.locator("article")).toHaveAttribute("id", `stock-plan-${fixture.plans[0].planId}`);
  await expect(core.getByRole("table").locator("tbody tr")).toHaveCount(2);
  await expect(core.locator("article")).toContainText("已预留 · 未下单");
  await page.reload();
  await showSection(page, "执行记录", false);
  await expect(core.locator("article")).toHaveCount(0);
  await expect(core.getByRole("table")).toContainText("已预留 · 未下单");
  await core.getByRole("button", { name: `查看 ${fixture.plans[0].planId}`, exact: true }).click();
  await expect(core.locator("article")).toHaveAttribute("id", `stock-plan-${fixture.plans[0].planId}`);
  expect(f.writes.map(w => w.path)).toEqual(["/api/stocks/plans/build"]);
  expect(f.errors).toEqual([]);
});

test("BP archived trade refreshes real inventory then reserves and restores linked funding without sending", async ({ page }) => {
  const f = await setup(page, "restock");
  await page.setViewportSize({ width: 1440, height: 1100 });
  await page.goto("/#stocks");
  await showSection(page, "执行记录");
  await page.getByRole("button", { name: "检查下一笔库存", exact: true }).click();
  const report = page.getByLabel("下一笔库存复查", { exact: true });
  await expect(report).toContainText("当前余额复查");
  await expect(report).toContainText("Solana · USDC 缺");
  await expect(report.getByText("已知费用后差额 / USDC", { exact: true })).toHaveCount(0);
  for (const width of [1440, 390, 320]) {
    await page.setViewportSize({ width, height: 1100 });
    await report.scrollIntoViewIfNeeded();
    const overflow = await report.locator("strong, span, button, summary, dt, dd").evaluateAll((els) => els.filter((e) => {
      const r = e.getBoundingClientRect(); return r.width > 0 && (r.left < -1 || r.right > window.innerWidth + 1);
    }).map((e) => e.textContent));
    expect(overflow).toEqual([]);
    await page.screenshot({ path: test.info().outputPath(`stock-restock-${width}.png`) });
  }
  await report.getByRole("button", { name: "保存补库计划", exact: true }).click();
  await showSection(page, "执行记录");
  const plans = page.getByRole("region", { name: "股票补库计划", exact: true });
  await expect(plans).toContainText("已预留 · 未转账");
  await plans.getByText("补库凭据", { exact: true }).click();
  await expect(plans).toContainText("来源归档交易");
  await page.reload();
  await showSection(page, "执行记录");
  await expect(plans).toContainText("已预留 · 未转账");
  await plans.getByRole("button", { name: "取消补库预留", exact: true }).click();
  await expect(plans).toContainText("已取消 · 未转账");
  expect(f.writes.map((r) => r.path)).toEqual(["/api/stocks/preflight", "/api/stocks/funding/plans", "/api/stocks/funding/plans/cancel"]);
  expect(f.errors).toEqual([]);
});

test("BP restock wallet changes and stale inventory disable funding", async ({ page }) => {
  const f = await setup(page, "restock");
  await page.goto("/#stocks");
  await showSection(page, "执行记录");
  await page.getByRole("button", { name: "检查下一笔库存", exact: true }).click();
  const report = page.getByLabel("下一笔库存复查", { exact: true });
  const save = report.getByRole("button", { name: "保存补库计划", exact: true });
  await expect(save).toBeEnabled();
  const wallet = page.getByRole("textbox", { name: "股票套利 Solana 钱包地址", exact: true });
  await wallet.fill("different-wallet");
  await expect(save).toBeDisabled();
  await expect(report).toContainText("库存复查已失效");
  await wallet.fill(restockFixture.ready.preflight.walletAddress);
  await expect(save).toBeEnabled();
  await page.clock.setFixedTime(restockFixture.ready.preflight.validUntilMs + 1);
  await expect(save).toBeDisabled();
  expect(f.writes.map((r) => r.path)).toEqual(["/api/stocks/preflight"]);
  expect(f.errors).toEqual([]);
});

test("BP conversion cash remains visible on budget failure and completion refreshes inventory without funds", async ({ page }) => {
  const f = await setup(page, "conversion_recovery");
  await page.goto("/#stocks");
  await showSection(page, "库存与成本");
  const panel = page.getByRole("region", { name: "Backpack 账户兑换", exact: true });
  const actual = panel.getByLabel("账户兑换实际收支", { exact: true });
  const next = panel.getByRole("button", { name: "重新检查库存与补库", exact: true });
  await expect(panel).toContainText("净入账未知");
  await expect(actual).toHaveCount(0);
  f.conversionReceipt("missing");
  await expect(panel.getByLabel("账户兑换待核对收支")).toContainText("9.997");
  await expect(actual).toHaveCount(0);
  await expect(next).toHaveCount(0);
  f.conversionReceipt("partial");
  await expect(panel).toContainText("收支已核对 · 未满足原计划");
  await expect(actual).toContainText("4.9935015");
  await expect(actual).toContainText("-5");
  await expect(panel).toContainText("仍保留资金占用");
  await expect(next).toHaveCount(0);
  f.conversionReceipt("fee_mismatch");
  await expect(actual).toContainText("实际 SOL 变化");
  await expect(actual).toContainText("-0.000001");
  await expect(next).toHaveCount(0);
  for (const width of [1440, 390, 320]) {
    await page.setViewportSize({ width, height: 900 });
    await panel.scrollIntoViewIfNeeded();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    const receipt = panel.getByLabel("已保存 Backpack 兑换");
    expect(await receipt.evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
    await panel.screenshot({ path: test.info().outputPath(`stock-conversion-accounting-${width}.png`) });
  }
  f.conversionReceipt("completed");
  await expect(panel).toContainText("兑换已完成 · 实际费用已核对");
  await expect(actual).toContainText("9.987003");
  await expect(next).toBeDisabled();
  await page.getByLabel("股票套利 Solana 钱包地址", { exact: true }).fill(WALLET);
  await expect(next).toBeEnabled();
  await next.click();
  await expect(page.getByRole("region", { name: "股票库存与成本交易检查" })).toContainText("新余额已读取");
  await expect(actual).toContainText("9.987003");
  await page.reload();
  await showSection(page, "库存与成本");
  await expect(actual).toContainText("9.987003");
  await expect(next).toBeDisabled();
  expect(f.writes.map((r) => r.path)).toEqual(["/api/stocks/preflight"]);
  expect(f.errors).toEqual([]);
});

test("BP conversion fees bind once to a stock plan and survive reload without funds", async ({ page }) => {
  const f = await setup(page, "conversion_costs");
  await page.goto("/#stocks");
  await page.getByLabel("链买预算 USDC", { exact: true }).fill("10");
  await showSection(page, "库存与成本");
  const inventory = page.getByRole("region", { name: "股票库存与成本交易检查" });
  await inventory.getByLabel("股票套利 Solana 钱包地址").fill(WALLET);
  const costs = inventory.locator(".stock-conversion-costs");
  await costs.locator("summary").click();
  const selection = costs.getByRole("checkbox").first();
  await expect(selection).toBeEnabled();
  await selection.check();
  await expect(costs.locator("summary")).toContainText("0.009997 USDC");
  const preview = inventory.locator(".stock-direction").first();
  await expect(preview).toContainText(costFixture.reserved.plans[0].terms.afterKnownCostsUsdc);
  for (const width of [1440, 390, 320]) {
    await page.setViewportSize({width, height: 950});
    await costs.scrollIntoViewIfNeeded();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    const label = costs.locator("label").first();
    expect(await label.evaluate(el => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
    await costs.screenshot({path: test.info().outputPath(`stock-cost-selection-${width}.png`)});
  }
  await preview.getByRole("button", {name: "构建并预留", exact:true}).click();
  await expect(costs.getByRole("checkbox").first()).toBeDisabled();
  await showSection(page, "执行记录");
  const history = page.getByRole("region", {name: "股票执行计划", exact: true});
  await history.getByText("计划凭据", {exact:true}).click();
  await expect(history).toContainText("已计入前期换币手续费 / USDC");
  await expect(history).toContainText("0.009997");
  await page.reload();
  await showSection(page, "执行记录");
  await expect(history.locator(".stock-plan-record")).toHaveCount(1);
  await showSection(page, "库存与成本");
  await costs.locator("summary").click();
  await expect(costs.getByRole("checkbox").first()).toBeDisabled();
  await showSection(page, "执行记录");
  await history.getByRole("button", {name:"取消预留",exact:true}).click();
  await showSection(page, "库存与成本");
  await expect(costs.getByRole("checkbox").first()).toBeEnabled();
  expect(f.writes.map(w => w.path)).toEqual(["/api/stocks/plans/build", "/api/stocks/plans/cancel"]);
  expect(f.errors).toEqual([]);
});

test("BP changed conversion receipt blocks stock execution without a submit request", async ({ page }) => {
  const f = await setup(page, "conversion_costs");
  await page.goto("/#stocks");
  await page.getByLabel("链买预算 USDC",{exact:true}).fill("10");
  await showSection(page, "库存与成本");
  const inventory = page.getByRole("region", {name:"股票库存与成本交易检查"});
  await inventory.getByLabel("股票套利 Solana 钱包地址").fill(WALLET);
  const costs = inventory.locator(".stock-conversion-costs");
  await costs.locator("summary").click();
  await costs.getByRole("checkbox").first().check();
  await inventory.getByRole("button",{name:"构建并预留",exact:true}).first().click();
  await showSection(page, "执行记录");
  const history=page.getByRole("region",{name:"股票执行计划",exact:true});
  await history.getByText("执行两腿",{exact:true}).click();
  f.costConflict();
  await expect(history.getByRole("alert")).toContainText("前期费用待核对");
  await expect(history.getByRole("button",{name:"提交两腿",exact:true})).toBeDisabled();
  await expect(history.getByLabel("确认本次实盘资金操作")).toBeDisabled();
  expect(f.writes.map(w=>w.path)).toEqual(["/api/stocks/plans/build"]);
  expect(f.errors).toEqual([]);
});

test("BP detail layout keeps both directions aligned and preserves quote guards", async ({ page }) => {
  const f = await setup(page, "quote_state");
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/#stocks");
  await showSection(page, "行情与提醒");
  const comparison = page.locator(".stock-comparison");
  const directions = comparison.locator(".stock-direction");
  const gross = comparison.locator(".stock-direction-values dd").first();
  const budget = page.getByLabel("链买预算 USDC", { exact: true });
  await expect(directions).toHaveCount(2);
  await expect(gross).not.toHaveText("—");
  await expect(page.getByRole("region", { name: "其他交易所股票对比" })).toBeHidden();
  for (const width of [1440, 1024, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await page.evaluate(() => window.scrollTo({ top: 0, behavior: "instant" }));
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
    const heading = await page.locator(".stock-heading").boundingBox();
    const tabs = await page.getByRole("navigation", { name: "股票详情视图" }).boundingBox();
    expect(heading!.y + heading!.height).toBeLessThanOrEqual(tabs!.y + 1);
    await expect(budget).toBeInViewport();
    if (width > 800) {
      await expect(comparison.locator(".stock-direction-columns")).toBeVisible();
      await expect(directions.last()).toBeInViewport();
      for (let i = 0; i < 4; i++) {
        const a = await directions.first().locator("dd").nth(i).boundingBox();
        const b = await directions.last().locator("dd").nth(i).boundingBox();
        expect(Math.abs(a!.x + a!.width - b!.x - b!.width)).toBeLessThan(1);
      }
    }
    await page.screenshot({ path: test.info().outputPath(`bp-detail-aligned-${width}.png`) });
  }
  // The peer form has its own tab; changing views must not discard the quote draft.
  await showSection(page, "跨所对比");
  await expect(comparison).toBeHidden();
  const peerPanel = page.getByRole("region", { name: "其他交易所股票对比" });
  await expect(peerPanel).toBeVisible();
  await expect(peerPanel).toContainText("当前对比 · kraken · MUx/USD");
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await page.evaluate(() => window.scrollTo({ top: 0, behavior: "instant" }));
    await expect(page.getByLabel("股票对比交易所", { exact: true })).toBeInViewport();
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
    await page.screenshot({ path: test.info().outputPath(`bp-peer-workspace-${width}.png`) });
  }
  await showSection(page, "行情与提醒");
  const reasons = directions.first().locator(".stock-direction-reasons");
  await reasons.locator("summary").click();
  await expect(reasons.locator("li").first()).toBeVisible();
  const metadata = comparison.locator(".stock-quote-meta");
  await metadata.locator("summary").click();
  await expect(metadata.getByRole("link", { name: "发行方 1:1 兑换资料" })).toBeVisible();
  await budget.fill("11");
  await expect(gross).toHaveText("—");
  await expect(reasons).toHaveAttribute("open", "");
  await expect(metadata).toHaveAttribute("open", "");
  await expect(reasons.locator("li").first()).toBeVisible();
  await expect(page.locator(".stock-context-summary")).toContainText("参数已更改");
  await showSection(page, "库存与成本");
  const inventory = page.getByRole("region", { name: "股票库存与成本交易检查" });
  await inventory.getByLabel("股票套利 Solana 钱包地址").fill(WALLET);
  const build = inventory.getByRole("button", { name: "构建并预留", exact: true }).first();
  await expect(build).toBeDisabled();
  await budget.fill("10");
  await expect(build).toBeEnabled();
  expect(f.writes).toEqual([]);
  expect(f.errors).toEqual([]);
});

test("BP quote edits and late replies cannot reserve a stale peer quantity", async ({ page }) => {
  const f = await setup(page, "quote_state");
  await page.goto("/#stocks");
  await showSection(page, "行情与提醒");
  const summary = page.locator(".stock-main .stock-summary").first();
  const budget = page.getByLabel("链买预算 USDC", { exact: true });
  const source = page.getByLabel("Jupiter 接入", { exact: true });
  const builder = page.getByRole("region", { name: "Kraken 双边计划构建" });
  const save = builder.getByRole("button", { name: "保存双边计划", exact: true }).first();
  const check = page.getByRole("button", { name: "验证股票卖单", exact: true });
  const bpGross = page.locator(".stock-comparison .stock-direction-values dd").first();
  const peerGross = page.locator(".stock-peers > .stock-directions .stock-direction-values dd").nth(1);
  await showSection(page, "跨所对比");
  await builder.locator("input").fill(WALLET);
  await expect(summary).toContainText("双向报价 · 仅观察");
  await expect(save).toBeEnabled();
  await expect(check).toBeEnabled();
  await expect(bpGross).not.toHaveText("—");
  await expect(peerGross).not.toHaveText("—");

  await budget.fill("11");
  await expect(summary).toContainText("参数已更改");
  await expect(page.locator(".stock-comparison .stock-quote-meta")).toContainText("上次报价 10 USDC");
  await expect(bpGross).toHaveText("—");
  await expect(peerGross).toHaveText("—");
  await expect(save).toBeDisabled();
  await expect(check).toBeDisabled();
  await showSection(page, "库存与成本");
  const bpBuild = page.getByRole("region", { name: "股票库存与成本交易检查" })
    .getByRole("button", { name: "构建并预留", exact: true }).first();
  await expect(bpBuild).toBeDisabled();
  await budget.fill("10.00");
  await expect(bpBuild).toBeEnabled();
  await showSection(page, "跨所对比");
  await source.selectOption("keyed");
  await expect(save).toBeDisabled();
  await source.selectOption("public");
  await expect(save).toBeEnabled();

  const release = f.holdQuote();
  await page.getByRole("button", { name: "更新询价", exact: true }).click();
  await expect(summary).toContainText("正在更新询价");
  await expect(save).toBeDisabled();
  await budget.fill("11");
  release();
  await expect(summary).toContainText("参数已更改");
  await expect(budget).toHaveValue("11");
  await expect(save).toBeDisabled();
  await expect(bpGross).toHaveText("—");
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await page.locator(".stock-quote-controls").scrollIntoViewIfNeeded();
    expect(await page.locator(".stock-quote-controls").evaluate(el => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
    await page.screenshot({ path: test.info().outputPath(`bp-quote-draft-${width}.png`) });
  }

  await budget.fill("10");
  await expect(save).toBeEnabled();
  await page.clock.setFixedTime(NOW + 11_000);
  await expect(summary).toContainText("报价已过期");
  await expect(save).toBeDisabled();
  await expect(check).toBeDisabled();
  // BP build refreshes evidence; expiry must not prevent requesting that refresh.
  await showSection(page, "库存与成本");
  await expect(bpBuild).toBeEnabled();
  expect(f.writes.map(w => w.path)).toEqual(["/api/stocks/quote"]);
  expect(f.errors).toEqual([]);
});

test("BP build and cancel update the actual WASM controls and survive reload", async ({ page }) => {
  const f = await setup(page);
  await page.goto("/#stocks");
  await showSection(page, "库存与成本");
  const inventory = page.getByRole("region", { name: "股票库存与成本交易检查" });
  const build = inventory.getByRole("button", { name: "构建并预留", exact: true }).first();
  const summary = page.locator(".stock-main .stock-summary").first();
  await expect(build).toBeDisabled();
  await expect(inventory.locator(".stock-build-status").first()).toContainText("先填写 Solana 钱包地址");
  await inventory.getByLabel("股票套利 Solana 钱包地址").fill(WALLET);
  await showSection(page, "行情与提醒");
  await page.getByLabel("链买预算 USDC", { exact: true }).fill("10.00");
  await showSection(page, "库存与成本");
  await expect(build).toBeEnabled();
  await expect(summary).toContainText("已完成交易检查 · 未预留");
  await expect(inventory.locator(".stock-chain-cost").first()).toHaveAttribute("data-current", "true");
  await showSection(page, "行情与提醒");
  await page.getByLabel("链买预算 USDC", { exact: true }).fill("11");
  await showSection(page, "库存与成本");
  await expect(build).toBeDisabled();
  await expect(summary).toContainText("交易检查需更新");
  await expect(inventory.locator(".stock-chain-cost").first()).toHaveAttribute("data-current", "false");
  await expect(inventory.locator(".stock-build-status").first()).toContainText("先更新询价");
  await showSection(page, "行情与提醒");
  await page.getByLabel("链买预算 USDC", { exact: true }).fill("10.00");
  await page.getByLabel("Jupiter 接入", { exact: true }).selectOption("keyed");
  await showSection(page, "库存与成本");
  await expect(build).toBeDisabled();
  await expect(summary).toContainText("交易检查需更新");
  await showSection(page, "行情与提醒");
  await page.getByLabel("Jupiter 接入", { exact: true }).selectOption("public");
  await showSection(page, "库存与成本");
  await build.click();
  const history = page.getByRole("region", { name: "股票执行计划", exact: true });
  await expect(history).toBeVisible();
  await expect(page.getByRole("navigation", { name: "股票详情视图" }).getByRole("button", { name: "执行记录", exact: true })).toHaveAttribute("aria-pressed", "true");
  await expect(history.locator(".stock-plan-record")).toHaveCount(1);
  await expect(page.getByRole("status").filter({ hasText: "计划已保存并预留" })).toBeVisible();
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    const plan = history.locator(".stock-plan-record").first();
    await plan.scrollIntoViewIfNeeded();
    expect(await plan.evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
    await page.screenshot({ path: test.info().outputPath(`stock-plan-navigation-${width}.png`) });
  }
  await page.setViewportSize({ width: 1440, height: 900 });
  await showSection(page, "库存与成本");
  await expect(summary).toContainText("已预留 · 未下单");
  await expect(build).toBeDisabled();
  await expect(inventory.locator(".stock-build-status").first()).toContainText("已有股票计划占用资金");
  await showSection(page, "执行记录");
  await expect(history.locator(".stock-plan-record")).toHaveCount(1);
  await page.reload();
  await expect(summary).toContainText("已预留 · 未下单");
  await showSection(page, "执行记录");
  await expect(history.locator(".stock-plan-record")).toHaveCount(1);
  await history.getByRole("button", { name: "取消预留", exact: true }).click();
  await expect(history.locator(".stock-plan-phase")).toHaveText("已取消");
  await showSection(page, "库存与成本");
  await inventory.getByLabel("股票套利 Solana 钱包地址").fill(WALLET);
  await expect(build).toBeEnabled();
  for (const width of [1440, 390, 320]) {
    await page.setViewportSize({ width, height: 900 });
    await inventory.evaluate((el) => el.scrollIntoView({ block: "start" }));
    const wallet = inventory.getByLabel("股票套利 Solana 钱包地址");
    expect(await wallet.evaluate((el) => {
      const r = el.getBoundingClientRect();
      const header = document.querySelector(".mod-topbar")!.getBoundingClientRect();
      return r.top >= header.bottom && document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2) === el;
    })).toBe(true);
    expect(await inventory.evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
    expect(await page.locator(".module-tabs").evaluate((el) => {
      const nav = el.getBoundingClientRect();
      const status = document.querySelector(".topbar-status-frame")!.getBoundingClientRect();
      return Array.from(el.querySelectorAll("button")).every((button) => {
        const r = button.getBoundingClientRect();
        const hit = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
        return r.bottom <= status.top + 1 && r.left >= nav.left && r.right <= nav.right + 1 && !!hit && button.contains(hit);
      });
    })).toBe(true);
    await page.screenshot({ path: test.info().outputPath(`bp-build-preflight-${width}.png`) });
  }
  await page.reload();
  await showSection(page, "执行记录");
  await expect(history.locator(".stock-plan-phase")).toHaveText("已取消");
  expect(f.writes.map((w) => w.path)).toEqual(["/api/stocks/plans/build", "/api/stocks/plans/cancel"]);
  expect(f.errors).toEqual([]);
});

test("BP late build receipt does not redirect a user who moved to another view", async ({ page }) => {
  const f = await setup(page);
  await page.goto("/#stocks");
  await showSection(page, "库存与成本");
  const inventory = page.getByRole("region", { name: "股票库存与成本交易检查" });
  await inventory.getByLabel("股票套利 Solana 钱包地址").fill(WALLET);
  const release = f.holdBuild();
  await inventory.getByRole("button", { name: "构建并预留", exact: true }).first().click();
  const details = page.getByRole("navigation", { name: "股票详情视图" }).getByRole("button", { name: "合约资料", exact: true });
  await details.click();
  release();
  await expect(page.getByRole("status").filter({ hasText: "计划已保存并预留" })).toBeVisible();
  await expect(page.getByRole("alert").filter({ hasText: "计划已保存并预留" })).toHaveCount(0);
  await expect(details).toHaveAttribute("aria-pressed", "true");
  await showSection(page, "执行记录");
  await expect(page.getByRole("region", { name: "股票执行计划" }).locator(".stock-plan-record")).toHaveCount(1);
  expect(f.writes.map(w => w.path)).toEqual(["/api/stocks/plans/build"]);
  expect(f.errors).toEqual([]);
});

test("BP lost or mismatched build receipts recover the original operation without reserving again", async ({ page }) => {
  const f = await setup(page);
  await page.goto("/#stocks");
  const inventory = page.getByRole("region", { name: "股票库存与成本交易检查" });
  const build = inventory.getByRole("button", { name: "构建并预留", exact: true }).first();
  const recovery = page.getByRole("alert", { name: "设置操作待核对" });
  const history = page.getByRole("region", { name: "股票执行计划", exact: true });
  await showSection(page, "库存与成本");
  await inventory.getByLabel("股票套利 Solana 钱包地址").fill(WALLET);
  f.mismatchInventory();
  await inventory.getByRole("button", { name: "检查库存与成本", exact: true }).click();
  await expect(page.getByRole("alert").filter({ hasText: "库存交易检查回复与当前股票或钱包不一致" })).toBeVisible();
  f.loseBuild(); await build.click();
  await expect(recovery).toContainText("构建股票计划结果待核对");
  await expect(build).toBeDisabled();
  await page.reload();
  await expect(recovery).toContainText("构建股票计划结果待核对");
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await recovery.scrollIntoViewIfNeeded();
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
    await page.screenshot({ path: test.info().outputPath(`bp-plan-recovery-${width}.png`) });
  }
  await page.setViewportSize({ width: 1440, height: 900 });
  // A later cancellation must not be overwritten by the original reserved receipt.
  f.cancelSavedBuild();
  await recovery.getByRole("button", { name: "核对上次操作", exact: true }).click();
  await expect(recovery).toBeHidden();
  await showSection(page, "执行记录");
  await expect(history.locator(".stock-plan-phase")).toHaveText("已取消");
  expect(f.writes.filter(w => w.path.endsWith("/build"))).toHaveLength(1);
  await showSection(page, "库存与成本");
  await inventory.getByLabel("股票套利 Solana 钱包地址").fill(WALLET);
  f.mismatchBuild(); await build.click();
  await expect(recovery).toContainText("结果待核对");
  await expect(page.getByRole("status").filter({ hasText: "计划已保存并预留" })).toHaveCount(0);
  const originalIds = f.buildActions.map(a => a.requestId);
  expect(new Set(originalIds).size).toBe(2);
  f.otherStock();
  await expect(page.locator(".stock-heading h2")).toHaveText("SNDK");
  await recovery.getByRole("button", { name: "核对上次操作", exact: true }).click();
  await expect(recovery).toBeHidden();
  await expect(page.locator(".stock-heading h2")).toHaveText("SNDK");
  await showSection(page, "执行记录");
  await expect(history.locator(".stock-plan-record")).toHaveCount(1);
  await showSection(page, "库存与成本");
  const release = f.holdInventory();
  await inventory.getByRole("button", { name: "检查库存与成本", exact: true }).click();
  await page.getByRole("button", { name: "切换到设置", exact: true }).click();
  await page.getByRole("tab", { name: "诊断", exact: true }).click();
  await page.getByRole("tab", { name: "连接", exact: true }).click();
  await page.locator(".settings-api-token-task input").fill("isolated-other-token");
  await page.getByRole("button", { name: "保存 Token", exact: true }).click();
  await page.getByRole("button", { name: "切换到股票套利", exact: true }).click();
  const oldRead = page.waitForResponse("**/api/stocks/preflight");
  release(); await (await oldRead).finished();
  await showSection(page, "库存与成本");
  await expect(inventory).toContainText("尚未检查交易");
  await expect(inventory.getByLabel("股票套利 Solana 钱包地址")).toHaveValue("");
  expect(f.writes.map(w => w.path)).toEqual(["/api/stocks/preflight", "/api/stocks/plans/build", "/api/stocks/plans/build", "/api/stocks/preflight"]);
  expect(f.errors).toEqual([]);
});

test("BP original plan completion preserves a newly selected stock", async ({ page }) => {
  const f = await setup(page);
  await page.goto("/#stocks");
  await showSection(page, "库存与成本");
  await page.getByLabel("股票套利 Solana 钱包地址").fill(WALLET);
  await page.getByRole("button", { name: "构建并预留", exact: true }).first().click();
  const history = page.getByRole("region", { name: "股票执行计划", exact: true });
  await expect(history).toBeVisible();
  const path = "/api/stocks/plans/cancel";
  const release = f.holdAction(path);
  const sent = page.waitForRequest(`${API}${path}`);
  await history.getByRole("button", { name: "取消预留", exact: true }).click();
  await sent;
  f.otherStock();
  await expect(page.locator(".stock-heading h2")).toHaveText("SNDK");
  const response = page.waitForResponse(`${API}${path}`);
  const current = page.waitForResponse(`${API}/api/stocks/peer/plans`);
  release(); await (await response).finished(); await (await current).finished();
  await expect(page.locator(".stock-heading h2")).toHaveText("SNDK");
  await showSection(page, "执行记录");
  await expect(history.locator(".stock-plan-phase")).toHaveText("已取消");
  expect(f.writes.map(w => w.path)).toEqual(["/api/stocks/plans/build", path]);
  expect(f.errors).toEqual([]);
});

test("BP financial replies stay with their original connection across account changes", async ({ browser }) => {
  for (const scenario of ["funding", "conversion", "peer", "stablecoin"] as const) {
    await test.step(scenario, async () => {
      const page = await browser.newPage({ baseURL: test.info().project.use.baseURL });
      try {
        const f = await setup(page, scenario === "stablecoin" ? "bp" : scenario);
        await page.goto("/#stocks");
        let path: string;
        let action;
        if (scenario === "funding") {
          await showSection(page, "执行记录");
          path = "/api/stocks/funding/plans/recheck";
          action = page.getByRole("button", { name: "核对原转账与 Backpack 入账", exact: true });
          await action.click();
          await expect(page.getByRole("region", { name: "股票补库计划", exact: true })).toContainText("已补充原交易入账回复");
        } else if (scenario === "peer") {
          await showSection(page, "执行记录");
          path = "/api/stocks/peer/plans/settle";
          action = page.getByRole("button", { name: "结算并释放预留", exact: true });
          await action.click();
          await expect(page.locator(".stock-peer-plan-record").first()).toContainText("已归档");
          f.restorePeer();
        } else if (scenario === "conversion") {
          await showSection(page, "库存与成本");
          const panel = page.getByRole("region", { name: "Backpack 账户兑换", exact: true });
          await panel.getByLabel("Backpack 最低到账 USDC", { exact: true }).fill("9.98");
          await panel.getByRole("button", { name: "计算所需 USDT", exact: true }).click();
          await expect(panel.getByLabel("Backpack 兑换投入 USDT", { exact: true })).toHaveValue("10");
          await panel.getByRole("button", { name: "生成兑换计划", exact: true }).click();
          path = "/api/stocks/funding/exchange-conversions/cancel";
          action = panel.getByRole("button", { name: "取消预留", exact: true });
        } else {
          await showSection(page, "库存与成本");
          await page.getByLabel("股票套利 Solana 钱包地址").fill(WALLET);
          await page.getByLabel("兑换投入 USDT", { exact: true }).fill("10");
          await page.getByLabel("希望补入 USDC", { exact: true }).fill("9.98");
          action = page.getByRole("button", { name: "试算兑换", exact: true });
          path = "/api/stocks/funding/stablecoin-preview";
          await action.click();
          await expect(page.locator(".stock-stablecoin-result")).toContainText("9.99");
          f.mismatchStablecoin(); await action.click();
          await expect(page.getByRole("alert").filter({ hasText: "兑换试算回复与当前参数不一致" })).toBeVisible();
          await expect(page.locator(".stock-stablecoin-result")).toHaveCount(0);
        }
        const release = f.holdAction(path, scenario === "conversion");
        const sent = page.waitForRequest(`${API}${path}`);
        await action.click();
        expect((await sent).headers().authorization).toBe("Bearer isolated-fixture-token");
        f.newAccount();
        await page.getByRole("button", { name: "切换到设置", exact: true }).click();
        await page.getByRole("tab", { name: "诊断", exact: true }).click();
        await page.getByRole("tab", { name: "连接", exact: true }).click();
        await page.locator(".settings-api-token-task input").fill("isolated-other-token");
        await page.getByRole("button", { name: "保存 Token", exact: true }).click();
        await page.getByRole("button", { name: "切换到股票套利", exact: true }).click();
        await expect(page.locator(".stock-heading h2")).toHaveText("SNDK");
        await expect.poll(() => f.reads.some(r => r.path.endsWith("/stablecoin-plans") && r.token === "Bearer isolated-other-token")).toBe(true);
        const before = f.reads.filter(r => r.path.endsWith("/stablecoin-plans") || r.path.endsWith("/peer/plans")).length;
        const response = page.waitForResponse(`${API}${path}`);
        release(); await (await response).finished();
        await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
        await expect(page.locator(".stock-heading h2")).toHaveText("SNDK");
        await showSection(page, "执行记录");
        await expect(page.locator(".stock-plan-record, .stock-peer-plan-record, .stock-funding-plan-record")).toHaveCount(0);
        await showSection(page, "库存与成本");
        await expect(page.getByLabel("股票套利 Solana 钱包地址")).toHaveValue("");
        await expect(page.locator(".stock-stablecoin-result")).toHaveCount(0);
        await expect(page.getByText("old source failed after account switch", { exact: true })).toHaveCount(0);
        expect(f.reads.filter(r => r.path.endsWith("/stablecoin-plans") || r.path.endsWith("/peer/plans")).length).toBe(before);
        expect(f.errors).toEqual([]);
      } finally { await page.close(); }
    });
  }
});

test("BP expired evidence can be refreshed by build, while damaged journals block new reservations", async ({ page }) => {
  const f = await setup(page);
  await page.clock.setFixedTime(NOW + 11_000);
  await page.goto("/#stocks");
  await showSection(page, "库存与成本");
  const inventory = page.getByRole("region", { name: "股票库存与成本交易检查" });
  await inventory.getByLabel("股票套利 Solana 钱包地址").fill(WALLET);
  const build = inventory.getByRole("button", { name: "构建并预留", exact: true }).first();
  await expect(page.locator(".stock-readiness > strong")).toHaveText("交易检查需更新");
  await expect(build).toBeEnabled();
  f.damage();
  await expect(build).toBeDisabled();
  await expect(inventory.locator(".stock-build-status").first()).toContainText("资金记录存在问题");
  expect(f.writes).toEqual([]);
  expect(f.errors).toEqual([]);
});

test("BP peer builder shows local failure and saved plan navigation without changing views", async ({ page }) => {
  const f = await setup(page, "quote_state");
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto("/#stocks");
  await showSection(page, "跨所对比");
  const builder = page.getByRole("region", { name: "Kraken 双边计划构建", exact: true });
  const save = builder.getByRole("button", { name: "保存双边计划", exact: true }).last();
  await builder.getByRole("textbox", { name: "Solana 钱包", exact: true }).fill(WALLET);
  await expect(save).toBeEnabled();
  f.rejectPeerBuild();
  await save.click();
  await expect(builder.getByRole("alert")).toContainText("双边库存不足");
  await expect(save).toBeEnabled();
  const release = f.holdAction("/api/stocks/peer/plans");
  await save.click();
  await expect(builder.getByRole("button", { name: "正在核对计划…", exact: true }).first()).toBeDisabled();
  await expect(builder.getByRole("alert")).toHaveCount(0);
  const details = page.getByRole("navigation", { name: "股票详情视图" }).getByRole("button", { name: "合约资料", exact: true });
  await details.click();
  release();
  const summary = page.locator(".stock-context-summary");
  await expect(summary).toContainText("已预留 · 未下单");
  await expect(details).toHaveAttribute("aria-pressed", "true");
  await showSection(page, "跨所对比");
  await expect(builder.getByRole("status")).toHaveText("双边计划已保存并预留，尚未下单");
  await expect(save).toHaveCount(0);
  const records = builder.getByRole("button", { name: "查看双边计划记录", exact: true });
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 1000 });
    await records.scrollIntoViewIfNeeded();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    expect(await records.evaluate((el) => {
      const r = el.getBoundingClientRect();
      const hit = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
      return !!hit && el.contains(hit);
    })).toBe(true);
    await page.screenshot({ path: test.info().outputPath(`bp-peer-saved-${width}.png`) });
  }
  await records.click();
  const history = page.getByRole("region", { name: "Kraken 双边计划记录", exact: true });
  await expect(history).toBeVisible();
  await expect(history.locator(".stock-peer-plan-record")).toHaveCount(1);
  await page.reload();
  await expect(summary).toContainText("已预留 · 未下单");
  await page.getByRole("button", { name: "查看执行记录", exact: true }).click();
  await history.getByRole("button", { name: "取消预留", exact: true }).click();
  await expect(history).toContainText("已取消 · 未下单");
  await showSection(page, "跨所对比");
  await expect(summary).not.toContainText("已预留");
  await expect(builder.getByRole("status")).toHaveCount(0);
  await expect(save).toBeVisible();
  const builds = f.writes.filter(w => w.path === "/api/stocks/peer/plans");
  expect(builds).toHaveLength(2);
  // A proven rejection is terminal; a deliberate new attempt has its own operation identity.
  expect(builds[0].body.requestId).not.toBe(builds[1].body.requestId);
  expect(f.writes.map(w => w.path)).toEqual(["/api/stocks/peer/plans", "/api/stocks/peer/plans", "/api/stocks/peer/plans/cancel"]);
  expect(f.errors).toEqual([]);
});

test("BP peer build recovers after reload and requires current records before unlocking", async ({ page }) => {
  const f = await setup(page, "quote_state");
  await page.goto("/#stocks");
  await showSection(page, "跨所对比");
  const builder = page.getByRole("region", { name: "Kraken 双边计划构建", exact: true });
  const save = builder.getByRole("button", { name: "保存双边计划", exact: true }).last();
  const recovery = page.getByRole("alert", { name: "设置操作待核对", exact: true });
  const verify = recovery.getByRole("button", { name: "核对上次操作", exact: true });
  const history = page.getByRole("region", { name: "Kraken 双边计划记录", exact: true });
  await builder.getByRole("textbox", { name: "Solana 钱包", exact: true }).fill(WALLET);
  f.losePeerBuild();
  await save.click();
  await expect(recovery).toContainText("构建股票双边计划结果待核对");
  await expect(save).toBeDisabled();
  await page.reload();
  await expect(recovery).toContainText("构建股票双边计划结果待核对");
  const original = structuredClone(f.buildActions[0].result);
  f.buildActions[0].result.request.requestId = "another-request";
  const beforeInvalidReceipt = f.reads.filter(r => r.path === "/api/stocks/peer/plans").length;
  await verify.click();
  await expect(recovery).toContainText("原双边构建处理结果不完整");
  expect(f.reads.filter(r => r.path === "/api/stocks/peer/plans")).toHaveLength(beforeInvalidReceipt);
  f.buildActions[0].result = original;
  f.failPeerRead();
  await verify.click();
  await expect(recovery).toContainText("读取当前计划失败");
  await expect(page.locator(".stock-context-summary")).toContainText("双边构建待核对");
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await verify.scrollIntoViewIfNeeded();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    expect(await verify.evaluate((el) => {
      const r = el.getBoundingClientRect();
      return el.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2));
    })).toBe(true);
    await page.screenshot({ path: test.info().outputPath(`bp-peer-build-recovery-${width}.png`) });
  }
  // The immutable reserved receipt must not revive a later cancelled reservation.
  f.cancelPeerBuild();
  await verify.click();
  await expect(recovery).toBeHidden();
  await showSection(page, "执行记录");
  await expect(history).toContainText("已取消 · 未下单");
  expect(f.writes).toHaveLength(1);
  await showSection(page, "跨所对比");
  await builder.getByRole("textbox", { name: "Solana 钱包", exact: true }).fill(WALLET);
  f.mismatchPeerBuild();
  await save.click();
  await expect(builder.getByRole("alert")).toContainText("双边构建处理结果参数不匹配");
  await expect(save).toBeDisabled();
  f.otherStock();
  await expect(page.locator(".stock-heading h2")).toHaveText("SNDK");
  await verify.click();
  await expect(recovery).toBeHidden();
  await expect(page.locator(".stock-heading h2")).toHaveText("SNDK");
  await showSection(page, "执行记录");
  await expect(history.locator(".stock-peer-plan-record")).toHaveCount(1);
  expect(f.writes.map(w => w.path)).toEqual(["/api/stocks/peer/plans", "/api/stocks/peer/plans"]);
  expect(f.errors).toEqual([]);
});

test("BP plan summary prioritizes unresolved trades and funds over preflight", async ({ page }) => {
  const f = await setup(page, "quote_state");
  await page.goto("/#stocks");
  const summary = page.locator(".stock-context-summary");
  await expect(summary).toContainText("交易检查需更新");
  f.planState("submitted");
  await expect(summary).toContainText("提交待核对");
  await expect(summary).not.toContainText("已预留 · 未下单");
  f.planState("peer_reserved");
  await expect(summary).toContainText("已预留 · 未下单");
  f.planState("peer_expired");
  await expect(summary).toContainText("交易检查需更新");
  for (const state of ["funding", "conversion"] as const) {
    f.planState(state);
    await expect(summary).toContainText("资金待核对");
  }
  f.planState("damaged");
  await expect(summary).toContainText("资金记录待核对");
  await showSection(page, "跨所对比");
  await expect(page.getByRole("region", { name: "Kraken 双边计划构建", exact: true }).getByRole("alert")).toHaveText("双边计划账本读取失败");
  expect(f.writes).toEqual([]);
  expect(f.errors).toEqual([]);
});

test("Stock peer final settlement preserves native cash and stays archived after reload", async ({ page }) => {
  const f = await setup(page, "peer");
  await page.goto("/#stocks");
  await showSection(page, "执行记录");
  const plan = page.locator(".stock-peer-plan-record").first();
  const settle = plan.getByRole("button", { name: "结算并释放预留", exact: true });
  await expect(settle).toBeEnabled();
  f.missingFee();
  await expect(settle).toBeDisabled();
  await expect(plan).toContainText("实际费用尚未核齐");
  f.restorePeer();
  await expect(settle).toBeEnabled();
  for (const width of [1440, 390, 320]) {
    await page.setViewportSize({ width, height: 900 });
    await settle.scrollIntoViewIfNeeded();
    expect(await settle.evaluate((el) => {
      const r = el.getBoundingClientRect();
      const hit = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
      return !!hit && el.contains(hit) && r.left >= 0 && r.right <= innerWidth;
    })).toBe(true);
    expect(await plan.evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
    await page.screenshot({ path: test.info().outputPath(`stock-peer-settlement-${width}.png`) });
  }
  await settle.click();
  await expect(plan).toContainText("已结算 · 预留已释放");
  await expect(plan).toContainText("已结算 · 原币分别记账");
  await expect(settle).toHaveCount(0);
  await expect(plan.getByRole("button", { name: "取消预留", exact: true })).toHaveCount(0);
  for (const [asset, amount] of Object.entries(peerFixture.peerPlans[0].settlement.accounting.cashTotals)) {
    await expect(plan.locator(".stock-peer-native-cash")).toContainText(`${amount} ${asset}`);
  }
  await page.reload();
  await showSection(page, "执行记录");
  await expect(plan).toContainText("已结算 · 预留已释放");
  await expect(settle).toHaveCount(0);
  await expect(plan.getByRole("button", { name: "提交双边计划", exact: true })).toHaveCount(0);
  expect(f.writes.map((w) => w.path)).toEqual(["/api/stocks/peer/plans/settle"]);
  expect(f.errors).toEqual([]);
});

test("BP funding conflict stays visible after a transient error, original recheck and reload", async ({ page }) => {
  const f = await setup(page, "funding");
  await page.goto("/#stocks");
  await showSection(page, "执行记录");
  const plans = page.getByRole("region", { name: "股票补库计划", exact: true });
  const transfer = plans.locator(".stock-funding-transfer");
  const alert = transfer.getByRole("alert");
  const recheck = transfer.getByRole("button", { name: "核对原转账与 Backpack 入账", exact: true });
  await expect(alert).toContainText("原入账处理结果冲突");
  await expect(plans.locator(".stock-plan-phase")).toHaveText("入账记录有矛盾 · 占用保留");
  await expect(plans.locator(".stock-funding-followup")).toContainText("自动核对已暂停");
  await expect(plans).toContainText("核对期间保留");
  await expect(recheck).toBeEnabled();
  await expect(plans.getByRole("button", { name: "取消补库预留", exact: true })).toBeDisabled();
  await expect(plans.getByRole("button", { name: "提交本次链上转账", exact: true })).toHaveCount(0);
  f.fundingUnavailable();
  await expect(transfer.locator(".stock-problem[role=status]")).toContainText("原入账历史查询失败");
  await expect(alert).toContainText("后续正常回复不会自动解除");
  await recheck.click();
  await expect(transfer.locator(".stock-problem[role=status]")).toContainText("已补充原交易入账回复");
  await expect(alert).toContainText("原始收支和资金占用已保留");
  await page.reload();
  await showSection(page, "执行记录");
  await expect(alert).toContainText("原入账处理结果冲突");
  await expect(plans).toContainText("核对期间保留");
  for (const width of [1440, 390, 320]) {
    await page.setViewportSize({ width, height: 1000 });
    await alert.scrollIntoViewIfNeeded();
    expect(await alert.evaluate((el) => {
      const r = el.getBoundingClientRect();
      return r.left >= 0 && r.right <= innerWidth && el.scrollWidth <= el.clientWidth + 1;
    })).toBe(true);
    await page.screenshot({ path: test.info().outputPath(`stock-funding-conflict-${width}.png`) });
    await recheck.scrollIntoViewIfNeeded();
    expect(await recheck.evaluate((el) => {
      const r = el.getBoundingClientRect();
      const hit = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
      return !!hit && el.contains(hit) && r.left >= 0 && r.right <= innerWidth;
    })).toBe(true);
    expect(await plans.evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
  }
  expect(f.writes).toEqual([{ path: "/api/stocks/funding/plans/recheck", body: { planId: fundingFixture.fundingPlans[0].planId } }]);
  expect(f.errors).toEqual([]);
});

test("BP funding history resumes beyond 400 rows before releasing the original reservation", async ({ page }) => {
  const f = await setup(page, "funding_scan");
  await page.goto("/#stocks");
  await showSection(page, "执行记录");
  const plans = page.getByRole("region", { name: "股票补库计划", exact: true });
  const progress = plans.locator(".stock-deposit-scan");
  const resume = plans.getByRole("button", { name: "继续核对原转账与 Backpack 入账", exact: true });
  await expect(progress).toContainText("已核对 400 条");
  await expect(plans.locator(".stock-plan-phase")).toHaveText("入账历史核对中");
  await expect(plans).toContainText("核对期间保留");
  await expect(plans.locator(".stock-funding-transfer")).toContainText("confirmed");
  await expect(resume).toBeEnabled();
  await expect(plans.getByRole("button", { name: "取消补库预留", exact: true })).toBeDisabled();
  await expect(plans.getByRole("button", { name: "提交本次链上转账", exact: true })).toHaveCount(0);
  await page.reload();
  await showSection(page, "执行记录");
  await expect(progress).toContainText("已核对 400 条");
  for (const width of [1440, 390, 320]) {
    await page.setViewportSize({ width, height: 1000 });
    await resume.scrollIntoViewIfNeeded();
    expect(await resume.evaluate((el) => {
      const r = el.getBoundingClientRect();
      const hit = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
      return !!hit && el.contains(hit) && r.left >= 0 && r.right <= innerWidth;
    })).toBe(true);
    expect(await plans.evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
    await page.screenshot({ path: test.info().outputPath(`stock-funding-pagination-${width}.png`) });
  }
  await resume.click();
  await expect(progress).toContainText("本轮入账历史已查完 · 650 条 · 已找到原交易");
  await expect(plans).toContainText("已释放");
  await expect(resume).toHaveCount(0);
  await page.reload();
  await showSection(page, "执行记录");
  await expect(progress).toContainText("已查完 · 650 条");
  await expect(plans.getByRole("button", { name: "核对原转账与 Backpack 入账", exact: true })).toHaveCount(0);
  expect(f.writes).toEqual([{ path: "/api/stocks/funding/plans/recheck", body: { planId: scanFixture.pending.fundingPlans[0].planId } }]);
  expect(f.errors).toEqual([]);
});

test("BP conversion sizing fills the exact input, discards late results and never submits funds", async ({ page }) => {
  const f = await setup(page, "conversion");
  await page.goto("/#stocks");
  await showSection(page, "库存与成本");
  const panel = page.getByRole("region", { name: "Backpack 账户兑换", exact: true });
  const input = panel.getByLabel("Backpack 兑换投入 USDT", { exact: true });
  const target = panel.getByLabel("Backpack 最低到账 USDC", { exact: true });
  const size = panel.getByRole("button", { name: "计算所需 USDT", exact: true });
  const build = panel.getByRole("button", { name: "生成兑换计划", exact: true });
  const estimate = panel.getByRole("status", { name: "账户兑换试算", exact: true });
  await expect(size).toBeDisabled();
  await target.fill("9.98");
  await size.click();
  await expect(input).toHaveValue("10");
  await expect(estimate).toContainText("9.987003");
  await expect(estimate).toContainText("0.009997");
  await expect(estimate).toContainText("尚未预留");
  await expect(panel.getByRole("region", { name: "已保存 Backpack 兑换", exact: true })).toHaveCount(0);
  for (const width of [1440, 390, 320]) {
    await page.setViewportSize({ width, height: 1000 });
    await estimate.scrollIntoViewIfNeeded();
    expect(await panel.evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
    await size.scrollIntoViewIfNeeded();
    expect(await size.evaluate((el) => {
      const r = el.getBoundingClientRect();
      return r.left >= 0 && r.right <= innerWidth && el.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2));
    })).toBe(true);
    await page.screenshot({ path: test.info().outputPath(`stock-conversion-sizing-${width}.png`) });
  }
  await page.clock.setFixedTime(sizingFixture.sizing.validUntilMs + 1);
  await expect(estimate).toContainText("试算已过期");
  await expect(build).toBeDisabled();
  await page.clock.setFixedTime(CONVERSION_NOW);
  await expect(build).toBeEnabled();
  expect(f.errors, "sizing and expiration").toEqual([]);
  await build.click();
  const saved = panel.getByRole("region", { name: "已保存 Backpack 兑换", exact: true });
  await expect(saved).toContainText("10 USDT → 至少 9.98 USDC");
  await expect(saved.getByRole("button", { name: "提交兑换", exact: true })).toBeDisabled();
  await saved.getByRole("checkbox", { name: "确认本次 Backpack 实盘兑换", exact: true }).check();
  await expect(saved.getByRole("button", { name: "提交兑换", exact: true })).toBeEnabled();
  expect(f.errors, "new plan rendered").toEqual([]);
  await saved.getByRole("button", { name: "取消预留", exact: true }).click();
  await expect(saved).toContainText("已取消 · 未兑换");
  expect(f.errors, "plan revision replaced after cancellation").toEqual([]);
  await page.reload();
  await showSection(page, "库存与成本");
  await expect(saved).toContainText("已取消 · 未兑换");
  let release!: () => void;
  const gate = new Promise<void>((resolve) => { release = resolve; });
  let requested!: () => void;
  const arrived = new Promise<void>((resolve) => { requested = resolve; });
  await page.route("**/api/stocks/funding/exchange-conversions/size", async (route) => {
    requested();
    await gate;
    await route.fulfill({ json: sizingFixture.sizing });
  });
  await target.fill("9.98");
  await size.click();
  await arrived;
  await target.fill("15");
  await input.fill("20");
  release();
  await expect(size).toBeEnabled();
  await expect(target).toHaveValue("15");
  await expect(input).toHaveValue("20");
  await expect(estimate).toHaveCount(0);
  expect(f.writes.map((r) => r.path)).toEqual([
    "/api/stocks/funding/exchange-conversions/size",
    "/api/stocks/funding/exchange-conversions",
    "/api/stocks/funding/exchange-conversions/cancel",
  ]);
  expect(f.errors).toEqual([]);
});

test("BP funding shortcuts size the source deficit and reject stale or wrong-wallet inventory", async ({ page }) => {
  const f = await setup(page, "conversion");
  f.conversionFunding("Solana", true, true);
  await page.goto("/#stocks");
  await showSection(page, "库存与成本");
  const wallet = page.getByLabel("股票套利 Solana 钱包地址", { exact: true });
  const account = page.getByRole("button", { name: "先补足 Backpack USDC", exact: true });
  const chain = page.getByRole("button", { name: "用此缺口试算 USDT 补入", exact: true });
  const target = page.getByLabel("Backpack 最低到账 USDC", { exact: true });
  const input = page.getByLabel("Backpack 兑换投入 USDT", { exact: true });
  await expect(account).toBeDisabled();
  await wallet.fill(WALLET);
  await expect(account).toBeEnabled();
  await page.setViewportSize({ width: 390, height: 1000 });
  const need = page.locator(".stock-funding-need").filter({ has: account });
  await need.scrollIntoViewIfNeeded();
  expect(await need.evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
  await need.screenshot({ path: test.info().outputPath("stock-funding-source-reserves-390.png") });
  await chain.click();
  await expect(page.getByLabel("希望补入 USDC", { exact: true })).toHaveValue("5.98");
  await account.click();
  await expect(target).toHaveValue("9.98");
  await expect(input).toHaveValue("10");
  await expect(page.getByRole("region", { name: "已保存 Backpack 兑换", exact: true })).toHaveCount(0);
  await wallet.fill("other-wallet");
  await expect(account).toBeDisabled();
  await expect(chain).toBeDisabled();
  await wallet.fill(WALLET);
  await page.clock.setFixedTime(CONVERSION_NOW + 30_001);
  await expect(account).toBeDisabled();
  await page.clock.setFixedTime(CONVERSION_NOW);
  await expect(account).toBeEnabled();
  f.conversionFunding("Solana", false);
  await expect(account).toHaveCount(0);
  f.conversionFunding("Backpack");
  await page.getByRole("button", { name: "用账户 USDT 补入", exact: true }).click();
  await expect(input).toHaveValue("10");
  await expect(target).toHaveValue("9.98");
  expect(f.writes).toEqual([
    { path: "/api/stocks/funding/exchange-conversions/size", body: { minimumUsdc: "9.98" } },
    { path: "/api/stocks/funding/exchange-conversions/size", body: { minimumUsdc: "9.98" } },
  ]);
  expect(f.errors).toEqual([]);
});
