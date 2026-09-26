import { expect, test, type Locator, type Page } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";
const PREVIEW_ROUTE = "**/api/arbitrage/opportunities/mock-mu-perp/preview";
const HYPERLIQUID_BUILDER_VENUE = "hyperliquid:xyz";
const HYPERLIQUID_BUILDER_CLOID = "0x00000000000000000000000000000001";
const FIXTURE_TIME_MS = 1_770_000_000_000;

type JsonRecord = Record<string, unknown>;

type TicketPlanSource = {
  ticketId: string;
  container: JsonRecord;
  longEvidence: JsonRecord;
  shortEvidence: JsonRecord;
  longPlan: JsonRecord;
  shortPlan: JsonRecord;
  hasTicketPlans: boolean;
};

async function useApiBase(page: Page) {
  await page.addInitScript((apiBase) => {
    window.localStorage.setItem("api_base", JSON.stringify(apiBase));
    window.localStorage.setItem("api_auth_token", JSON.stringify("e2e-token"));
  }, API_BASE);
}

async function useTicketScenario(page: Page, scenario: "ready" | "missing") {
  await useApiBase(page);
  await page.route(PREVIEW_ROUTE, async (route) => {
    const upstream = await route.fetch();
    const preview = asRecord(await upstream.json(), "hedge preview response");

    if (scenario === "ready") {
      bindHyperliquidTicketPlan(preview);
      delete preview.longOrderPlan;
      delete preview.shortOrderPlan;
    } else {
      // Legacy top-level plans must not authorize a ticket without bound evidence.
      delete preview.ticketOrderPlans;
    }

    await route.fulfill({ response: upstream, json: preview });
  });
}

async function openTicketPlan(page: Page, scenario: "ready" | "missing"): Promise<Locator> {
  await useTicketScenario(page, scenario);
  const previewResponse = page.waitForResponse((response) =>
    response.url().includes("/api/arbitrage/opportunities/mock-mu-perp/preview")
      && response.status() === 200,
  );

  await page.goto("/#futures");
  await expect(page.getByRole("heading", { name: "期货套利" })).toBeVisible();
  await expect(page.locator(".futures-table tbody tr").filter({ hasText: "MU" })).toHaveCount(1);
  const build = page.getByRole("button", { name: "构建对冲" });
  await expect(build).toHaveCount(1);
  await expect(build).toBeEnabled();
  await build.click();
  await previewResponse;

  const plan = page.locator(".execution-risk-section .risk-notes > div").filter({
    hasText: "订单编译",
  }).locator("strong");
  await expect(plan).toBeVisible();
  return plan;
}

function bindHyperliquidTicketPlan(preview: JsonRecord) {
  const source = ticketPlanSource(preview);
  const longPlan = hyperliquidBuilderPlan(source.longPlan);
  const shortPlan = source.hasTicketPlans
    ? source.shortPlan
    : ticketLocalCounterpartyPlan(source.shortPlan);
  const shortEvidence = source.hasTicketPlans
    ? source.shortEvidence
    : ticketPlanEvidence({}, shortPlan);

  preview.ticketOrderPlans = {
    ...source.container,
    ticketId: source.ticketId,
    long: ticketPlanEvidence(source.longEvidence, longPlan),
    short: shortEvidence,
  };

  const ticket = asRecord(preview.ticket, "ticket");
  setExchange(ticket.longLeg, HYPERLIQUID_BUILDER_VENUE);
  setExchange(preview.longLeg, HYPERLIQUID_BUILDER_VENUE);
}

function ticketPlanSource(preview: JsonRecord): TicketPlanSource {
  const ticket = asRecord(preview.ticket, "ticket");
  const ticketId = requiredString(ticket.ticketId, "ticket.ticketId");
  const ticketPlans = optionalRecord(preview.ticketOrderPlans);
  if (ticketPlans) {
    const longEvidence = asRecord(ticketPlans.long, "ticketOrderPlans.long");
    const shortEvidence = asRecord(ticketPlans.short, "ticketOrderPlans.short");
    return {
      ticketId,
      container: ticketPlans,
      longEvidence,
      shortEvidence,
      longPlan: compilePlanFrom(longEvidence, "ticketOrderPlans.long"),
      shortPlan: compilePlanFrom(shortEvidence, "ticketOrderPlans.short"),
      hasTicketPlans: true,
    };
  }

  return {
    ticketId,
    container: {},
    longEvidence: {},
    shortEvidence: {},
    longPlan: asRecord(preview.longOrderPlan, "legacy longOrderPlan"),
    shortPlan: asRecord(preview.shortOrderPlan, "legacy shortOrderPlan"),
    hasTicketPlans: false,
  };
}

function compilePlanFrom(evidence: JsonRecord, label: string): JsonRecord {
  return optionalRecord(evidence.compilePlan) ?? evidence;
}

function ticketPlanEvidence(baseEvidence: JsonRecord, compilePlan: JsonRecord): JsonRecord {
  return {
    ...baseEvidence,
    compilePlan,
    identityPlan: unconstrainedIdentityPlan(compilePlan),
  };
}

function hyperliquidBuilderPlan(basePlan: JsonRecord): JsonRecord {
  const symbol = requiredString(basePlan.symbol, "long plan symbol");
  const policy = optionalRecord(basePlan.clientOrderIdPolicy) ?? {};
  const referencePrice = numericValue(basePlan.referencePrice) ?? 664.63;
  const protectionPrice = referencePrice * 1.0005;

  return {
    ...basePlan,
    role: "long",
    exchange: HYPERLIQUID_BUILDER_VENUE,
    symbol,
    clientOrderIdPolicy: {
      ...policy,
      venue: HYPERLIQUID_BUILDER_VENUE,
      venueFamily: "hyperliquid",
      venueField: "c/cloid",
      publicClientOrderId: "ticket-long-builder-order",
      venueClientOrderId: HYPERLIQUID_BUILDER_CLOID,
      derivation: "stable_hash",
      policyVersion: "hyperliquid-cloid-v1",
      officialFormat: "0x + 32 lowercase hex",
      maxLength: 34,
      supportsQueryByClientId: true,
      supportsCancelByClientId: true,
      constraints: [],
      blockers: [],
      officialDocUrls: [],
    },
    product: "perp",
    requestedOrderType: "market",
    effectiveOrderType: "limit",
    requestedTimeInForce: "ioc",
    effectiveTimeInForce: "ioc",
    availableOrderTypes: ["limit", "market"],
    availableTimeInForce: ["ioc"],
    availableMarginModes: ["cross"],
    venueOrderKind: "protected_ioc",
    payloadPricePolicy: "protection_price",
    referencePrice,
    protectionPrice,
    payloadPrice: protectionPrice,
    slippageToleranceBps: 5,
    summary: "Hyperliquid builder XYZ ticket-owned protected IOC plan",
    blockers: [],
  };
}

function ticketLocalCounterpartyPlan(basePlan: JsonRecord): JsonRecord {
  const exchange = requiredString(basePlan.exchange, "short plan exchange");
  const symbol = requiredString(basePlan.symbol, "short plan symbol");
  const policy = optionalRecord(basePlan.clientOrderIdPolicy) ?? {};
  const referencePrice = numericValue(basePlan.referencePrice) ?? 664.2;

  return {
    ...basePlan,
    role: "short",
    exchange,
    symbol,
    clientOrderIdPolicy: {
      ...policy,
      venue: exchange,
      venueFamily: exchange.split(":")[0],
      venueField: "clientOrderId",
      publicClientOrderId: "ticket-short-counterparty-order",
      venueClientOrderId: "ticket-short-counterparty-order",
      derivation: "identity",
      policyVersion: "ticket-local-counterparty-v1",
      officialFormat: "ticket-local counterparty client id",
      maxLength: 36,
      supportsQueryByClientId: true,
      supportsCancelByClientId: true,
      constraints: [],
      blockers: [],
      officialDocUrls: [],
    },
    product: "perp",
    requestedOrderType: "limit",
    effectiveOrderType: "limit",
    requestedTimeInForce: "ioc",
    effectiveTimeInForce: "ioc",
    availableOrderTypes: ["limit"],
    availableTimeInForce: ["ioc"],
    availableMarginModes: ["cross"],
    venueOrderKind: "limit",
    payloadPricePolicy: "limit_price",
    referencePrice,
    protectionPrice: referencePrice,
    payloadPrice: referencePrice,
    summary: "ticket-local counterparty plan",
    blockers: [],
  };
}

function unconstrainedIdentityPlan(compilePlan: JsonRecord): JsonRecord {
  const policy = asRecord(compilePlan.clientOrderIdPolicy, "compile plan client id policy");
  return {
    evidenceRequired: false,
    canonicalSymbol: requiredString(compilePlan.symbol, "compile plan symbol"),
    product: requiredString(compilePlan.product, "compile plan product"),
    clientOrderIdPolicy: policy,
    exchangeOrderIdFinalitySource: "unavailable",
    evidence: ["metadata", "user_stream", "order_finality", "fee"].map((kind) => ({
      kind,
      status: "unavailable",
    })),
    blockers: [],
  };
}

function setExchange(value: unknown, exchange: string) {
  const record = optionalRecord(value);
  if (record) record.exchange = exchange;
}

function asRecord(value: unknown, label: string): JsonRecord {
  const record = optionalRecord(value);
  if (!record) throw new Error(`${label} must be an object`);
  return record;
}

function optionalRecord(value: unknown): JsonRecord | undefined {
  if (!value || typeof value !== "object" || Array.isArray(value)) return undefined;
  return value as JsonRecord;
}

function requiredString(value: unknown, label: string): string {
  if (typeof value !== "string" || value.trim().length === 0) {
    throw new Error(`${label} must be a non-empty string`);
  }
  return value;
}

function numericValue(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

async function useHyperliquidSettingsScenario(page: Page) {
  await useApiBase(page);
  await page.route("**/api/exchanges/credentials", async (route) => {
    await route.fulfill({ json: hyperliquidCredentials() });
  });
  await page.route("**/api/trading/ws/venues", async (route) => {
    await route.fulfill({ json: hyperliquidWsVenues() });
  });
}

function hyperliquidCredentials() {
  return {
    venues: [{
      venue: HYPERLIQUID_BUILDER_VENUE,
      label: "Hyperliquid XYZ",
      fields: [
        credentialField("account_address", "余额读取地址（主账户 / 子账户）", "HYPERLIQUID_ACCOUNT_ADDRESS", false),
        credentialField("private_key", "已授权 API / Agent 钱包私钥", "HYPERLIQUID_PRIVATE_KEY", true),
        credentialField("vault_address", "Vault 执行地址（可选）", "HYPERLIQUID_VAULT_ADDRESS", false),
      ],
      publicMarket: true,
      privateRead: true,
      testnetWrite: false,
      liveWrite: true,
      note: "Builder-scoped endpoint metadata and API wallet/vault relation are read-only evidence, not live writer readiness.",
      validationEvidence: {
        status: "read_only_ok",
        checkedAtMs: FIXTURE_TIME_MS,
        probes: [
          credentialProbe("balance_read", "ok", "hyperliquid:xyz", "hyperliquid.info clearinghouseState", "private balance read observed"),
          credentialProbe("positions_read", "ok", "hyperliquid:xyz", "hyperliquid.info clearinghouseState", "private position read observed"),
          credentialProbe("open_orders_read", "ok", "hyperliquid:xyz", "hyperliquid.info frontendOpenOrders", "private open-order read observed"),
          credentialProbe("order_permission", "unknown", "hyperliquid_noop", "hyperliquid.POST /exchange action=noop", "safe noop is not a live place/cancel acknowledgement"),
          credentialProbe(
            "account_mode_read",
            "ok",
            "hyperliquid_account_role_abstraction",
            "userRole(account)+userRole(signer)+userAbstraction(account)+userDexAbstraction(account)",
            "role relation=verified；abstraction=verified",
          ),
          credentialProbe(
            "account_signer_vault_relation",
            "ok",
            "hyperliquid_account_signer_vault",
            "userRole(account)+userRole(signer)+vaultDetails",
            "account=0x1111111111111111111111111111111111111111 role=user owner=self; derived signer=0x2222222222222222222222222222222222222222 role=agent owner=0x1111111111111111111111111111111111111111; vault=none role=none leader=none",
          ),
          credentialProbe(
            "account_abstraction",
            "ok",
            "0x1111111111111111111111111111111111111111",
            "userAbstraction(account)+userDexAbstraction(account)",
            "account=0x1111111111111111111111111111111111111111 userAbstraction=default userDexAbstraction=xyz",
          ),
          credentialProbe("perp_margin_read", "ok", "perp_margin.USDC", "hyperliquid.POST /info type=clearinghouseState", "read-only perp margin probe succeeded; rows=1"),
          credentialProbe("spot_truth_read", "ok", "spot_truth.USDC", "hyperliquid.POST /info type=spotClearinghouseState", "read-only spot truth probe succeeded; rows=1"),
        ],
      },
    }],
    secretStorage: {
      mode: "runtime_only",
      persistent: false,
      encrypted: false,
      atomicWrite: false,
      path: null,
      label: "E2E runtime-only credentials",
      message: "PR-EQ browser fixture does not persist wallet material.",
      warning: "no live credentials",
    },
  };
}

function credentialField(key: string, label: string, envKey: string, secret: boolean) {
  return { key, label, envKey, configured: false, secret };
}

function credentialProbe(
  kind: string,
  status: string,
  scope: string,
  source: string,
  message: string,
) {
  return { kind, status, scope, source, message, checkedAtMs: FIXTURE_TIME_MS, requestId: null };
}

function hyperliquidWsVenues() {
  const staticRead = hyperliquidWsOperation("userEvents", "builder account stream metadata", false);
  const protectedWrite = hyperliquidWsOperation(
    "post exchange order (builder dex xyz)",
    "API wallet/vault relation does not replace authenticated live writer evidence",
    true,
  );
  return {
    venues: [{
      venue: HYPERLIQUID_BUILDER_VENUE,
      label: "Hyperliquid XYZ",
      publicEndpoint: "wss://api.hyperliquid.xyz/ws",
      privateEndpoint: "wss://api.hyperliquid.xyz/ws",
      tradeEndpoint: "wss://api.hyperliquid.xyz/ws (builder dex xyz)",
      accountStream: staticRead,
      positionStream: staticRead,
      fillStream: staticRead,
      orderStream: staticRead,
      placeOrder: protectedWrite,
      cancelOrder: protectedWrite,
      closePosition: protectedWrite,
      orderStatus: staticRead,
      authFields: ["account_address", "private_key", "vault_address"],
      docs: [{
        label: "Hyperliquid websocket subscriptions",
        url: "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/websocket/subscriptions",
      }],
      note: "HIP-3 builder dex xyz endpoint scope is static metadata; userRole and vaultDetails remain non-live evidence.",
    }],
  };
}

function hyperliquidWsOperation(operation: string, note: string, requiresRuntimeEvidence: boolean) {
  return {
    supported: true,
    status: requiresRuntimeEvidence ? "requires_permission" : "ready",
    operation,
    product: "HIP-3 builder perpetual",
    note,
    evidence: {
      releaseStatus: "production_ready",
      requiresAuthenticatedRuntimeEvidence: requiresRuntimeEvidence,
      authenticatedRuntimeEvidence: false,
      checkedAt: "2026-07-12",
      docVersion: "hyperliquid-builder-ws-2026-07-12",
      docUrl: "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/websocket/subscriptions",
      parserTest: "hyperliquid_builder_ws_fixture",
      subscriptionTest: "hyperliquid_builder_subscription_fixture",
      authKind: "L1 wallet signature",
    },
  };
}

test("PR-EQ renders ticket-owned Hyperliquid builder cloid and protected IOC evidence", async ({ page }) => {
  const plan = await openTicketPlan(page, "ready");

  await expect(plan).toContainText("多腿 保护 IOC");
  await expect(plan).toHaveAttribute("title", /多腿 hyperliquid:xyz/);
  await expect(plan).toHaveAttribute("title", /venue client id 0x00000000000000000000000000000001/);
  await expect(plan).toHaveAttribute("title", /Hyperliquid builder XYZ ticket-owned protected IOC plan/);
  await expect(plan).toHaveAttribute("title", /payload 保护价/);
  await expect(page.locator(".confirm-action.primary")).toBeEnabled();
});

test("PR-EQ blocks submit when ticket-owned identity and protected IOC evidence is absent", async ({ page }) => {
  const plan = await openTicketPlan(page, "missing");

  await expect(plan).toContainText("等待订单编译");
  await expect(page.locator(".execution-risk-section .risk-notes > p")).toContainText(
    "HEDGE_TICKET_ORDER_PLAN_EVIDENCE_MISSING",
  );
  await expect(page.locator(".confirm-action.primary")).toBeDisabled();
});

test("PR-EQ Settings keeps builder-scoped Hyperliquid endpoint and wallet/vault evidence non-live", async ({ page }) => {
  await useHyperliquidSettingsScenario(page);
  const credentials = page.waitForResponse((response) =>
    response.url().includes("/api/exchanges/credentials") && response.status() === 200,
  );
  const wsVenues = page.waitForResponse((response) =>
    response.url().includes("/api/trading/ws/venues") && response.status() === 200,
  );

  await page.goto("/#settings");
  await Promise.all([credentials, wsVenues]);
  await expect(page.locator("h1", { hasText: "设置" })).toBeVisible();
  await expect(page.getByLabel("交易所")).toHaveValue(HYPERLIQUID_BUILDER_VENUE);

  const wsPanel = page.locator(".ws-venue-panel").filter({ hasText: "Hyperliquid XYZ" });
  await expect(wsPanel).toContainText("交易 WS · wss://api.hyperliquid.xyz/ws (builder dex xyz)");
  await expect(wsPanel).toContainText("静态 WS 能力，不代表当前连接、权限或订单状态流已验证。");
  const placeOrder = wsPanel.locator(".ws-cap").filter({ hasText: "下单" });
  await expect(placeOrder).toContainText("缺认证运行数据依据");
  await expect(placeOrder).toContainText("live writer 禁止提交");

  const validation = page.locator(".runtime-health-panel").filter({ hasText: "保存期验证" });
  await expect(validation).toContainText("权限未完整");
  await expect(validation).not.toContainText("权限验证完整");
  const walletVault = validation.locator("tbody tr").filter({
    hasText: "userRole(account)+userRole(signer)+vaultDetails",
  });
  await expect(walletVault).toContainText("通过");
  await expect(walletVault).toContainText(
    "derived signer=0x2222222222222222222222222222222222222222",
  );
  await expect(walletVault.locator("td").last()).toHaveAttribute(
    "title",
    /hyperliquid_account_signer_vault/,
  );
  const abstraction = validation.locator('[data-validation-probe="account_abstraction"]');
  await expect(abstraction).toContainText("userAbstraction=default");
  await expect(abstraction).toContainText("userDexAbstraction=xyz");
  const spotTruth = validation.locator('[data-validation-probe="spot_truth_read"]');
  await expect(spotTruth).toContainText("Spot Truth");
  await expect(spotTruth).toContainText("通过");
  const perpMargin = validation.locator('[data-validation-probe="perp_margin_read"]');
  await expect(perpMargin).toContainText("Perp Margin");
  await expect(perpMargin).toContainText("通过");
});
