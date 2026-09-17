import http from "node:http";
import { WebSocketServer } from "ws";

const port = Number(process.env.CROSSLINE_E2E_API_PORT ?? "18000");
const webBase = process.env.CROSSLINE_E2E_WEB_BASE ?? "http://127.0.0.1:18080";
const now = 1_770_000_000_000;
const largeTablesScenario = "large-tables";
const scenarioPrefixes = new Map([
  ["/e2e-pr-eo-kucoin-native-ready", "pr-eo-kucoin-native-ready"],
  ["/e2e-pr-eo-kucoin-runtime-unavailable", "pr-eo-kucoin-runtime-unavailable"],
  ["/e2e-pr-el-binance-identity-ready", "pr-el-binance-identity-ready"],
  ["/e2e-pr-el-binance-identity-missing", "pr-el-binance-identity-missing"],
  ["/e2e-pr-el-binance-runtime-unavailable", "pr-el-binance-runtime-unavailable"],
  ["/e2e-pr-er-gate-runtime", "pr-er-gate-runtime"],
  ["/e2e-pr-bx-runtime", "pr-bx-runtime"],
  ["/e2e-watchlist-alerts-runtime", "watchlist-alerts-runtime"],
  ["/e2e-large-tables", largeTablesScenario],
  ["/e2e-auth-401", "typed-auth-401"],
  ["/e2e-confirm-429", "confirm-429"],
  ["/e2e-confirm-200-failed", "confirm-200-failed"],
  ["/e2e-credential-save-denied", "credential-save-denied"],
  ["/e2e-confirm-denied", "confirm-denied"],
  ["/e2e-order-cancel-denied", "order-cancel-denied"],
  ["/e2e-position-close-denied", "position-close-denied"],
  ["/e2e-position-cancel-denied", "position-cancel-denied"],
  ["/e2e-extractor-422", "extractor-422"],
  ["/e2e-portfolio-snapshot-502", "portfolio-snapshot-502"],
  ["/e2e-review-502", "review-executed-502"],
  ["/e2e-review-missed-502", "review-missed-502"],
  ["/e2e-review-resources-502", "review-resources-502"],
  ["/e2e-review-stale-retry", "review-stale-retry"],
  ["/e2e-system-502", "system-health-502"],
  ["/e2e-api-transport", "api-transport"],
  ["/e2e-api-transport-fallback", "api-transport-fallback"],
  ["/e2e-private-order-stream-warning", "private-order-stream-warning"],
  ["/e2e-settings-credential-static-adapter-boundary", "settings-credential-static-adapter-boundary"],
  ["/e2e-pr-fu-settings-environment", "settings-credential-static-adapter-boundary"],
  [
    "/e2e-settings-private-order-stream-ok-capture-readiness",
    "settings-private-order-stream-ok-capture-readiness",
  ],
  [
    "/e2e-settings-selected-venue-trading-runtime-all-ok-local-capture-shaped",
    "settings-selected-venue-trading-runtime-all-ok-local-capture-shaped",
  ],
  ["/e2e-ws-auth", "ws-auth"],
  ["/e2e-ws-decode", "ws-decode"],
  ["/e2e-ws-error", "ws-error"],
]);
const wsAuthEvents = [];
const routeCounters = new Map();

const server = http.createServer((request, response) => {
  const url = new URL(request.url ?? "/", `http://${request.headers.host}`);
  const context = routeContext(url);
  route(context.url, request, response, context.scenario);
});

const ws = new WebSocketServer({ noServer: true });
server.on("upgrade", (request, socket, head) => {
  const url = new URL(request.url ?? "/", `http://${request.headers.host}`);
  const context = routeContext(url);
  if (context.url.pathname !== "/ws") {
    socket.destroy();
    return;
  }
  ws.handleUpgrade(request, socket, head, (client) => bindClient(client, context.scenario));
});

server.listen(port, "127.0.0.1");

function routeContext(url) {
  for (const [prefix, scenario] of scenarioPrefixes.entries()) {
    if (url.pathname === prefix || url.pathname.startsWith(`${prefix}/`)) {
      const routed = new URL(url);
      routed.pathname = routed.pathname.slice(prefix.length) || "/";
      return { url: routed, scenario };
    }
  }
  return { url, scenario: url.searchParams.get("e2eScenario") };
}

function bindClient(client, scenario) {
  let authenticated = scenario !== "ws-auth";
  client.on("message", (raw) => {
    const msg = parseJson(raw.toString());
    if (msg?.type === "auth") {
      wsAuthEvents.push({ scenario, type: "auth", ticket: msg.ticket ?? null });
      if (scenario !== "ws-auth" || msg.ticket === "e2e-ws-ticket") {
        authenticated = true;
        return;
      }
      send(client, {
        type: "error",
        code: "WS_UNAUTHORIZED",
        message: "mock websocket auth ticket invalid",
      });
      client.close();
    } else if (msg?.type === "subscribe") {
      wsAuthEvents.push({
        scenario,
        type: "subscribe",
        authenticated,
        channels: msg.channels ?? [],
      });
      if (!authenticated) {
        send(client, {
          type: "error",
          code: "WS_UNAUTHORIZED",
          message: "mock websocket auth required before subscribe",
        });
        client.close();
        return;
      }
      send(client, { type: "ack", subscribed: msg.channels ?? [] });
      pushSubscribed(client, msg.channels ?? [], scenario);
    } else if (msg?.type === "ping") {
      send(client, { type: "pong" });
    }
  });
}

function pushSubscribed(client, channels, scenario) {
  if (scenario === "ws-decode") {
    send(client, { type: "message", channel: "arbitrage", payload: null });
    return;
  }
  if (scenario === "ws-error") {
    send(client, {
      type: "error",
      channel: "arbitrage",
      code: "WS_CHANNEL_REJECTED",
      message: "mock ws channel rejected",
      retryAfterMs: 2000,
      requestId: "e2e-ws-error",
    });
    return;
  }
  for (const channel of channels) {
    const payload = channelPayload(channel, scenario);
    if (payload) {
      if (scenario === "watchlist-alerts-runtime") {
        setTimeout(() => send(client, { type: "message", channel, payload }), 100);
      } else {
        send(client, { type: "message", channel, payload });
      }
    }
    if (channel === "alerts" && scenario === "watchlist-alerts-runtime") {
      setTimeout(() => {
        send(client, {
          type: "message",
          channel,
          payload: {
            event: "alert_triggered",
            notification: alertRuntimeNotification(),
          },
        });
      }, 350);
    }
  }
}

function channelPayload(channel, scenario) {
  if (channel === "arbitrage") {
    if (scenario === largeTablesScenario) return null;
    return opportunityStreamEvent([muOpportunity()]);
  }
  if (channel === "portfolio") {
    if (scenario === "portfolio-snapshot-502") return null;
    if (scenario === "pr-er-gate-runtime") return prErGatePortfolioSnapshot();
    if (scenario === "position-close-denied") return positionCloseDeniedSnapshot();
    if (scenario === "position-cancel-denied") return positionCancelDeniedSnapshot();
    return portfolioSnapshot();
  }
  if (channel === "system") {
    if (scenario === "system-health-502") return null;
    return systemHealth();
  }
  if (channel === "watchlist" && scenario === "watchlist-alerts-runtime") {
    return {
      event: "watchlist_changed",
      envelope: watchlistRuntimeEnvelope(),
      timestampMs: now,
    };
  }
  if (channel === "alerts" && scenario === "watchlist-alerts-runtime") {
    return {
      event: "alert_rules_changed",
      envelope: alertRulesRuntimeEnvelope(),
      timestampMs: now,
    };
  }
  return null;
}

function route(url, request, response, scenario = null) {
  if (request.method === "OPTIONS") {
    return write(response, 204, "");
  }
  if (url.pathname === "/api/e2e/ws-auth-events") {
    if (request.method === "DELETE") {
      wsAuthEvents.splice(0, wsAuthEvents.length);
      return json(response, { events: [] });
    }
    return json(response, { events: wsAuthEvents });
  }
  if (url.pathname === "/api/e2e/request-stats") {
    if (request.method === "DELETE") {
      resetRouteCounters(scenario);
      return json(response, { counts: {} });
    }
    return json(response, { counts: routeCountsForScenario(scenario) });
  }
  if (url.pathname === "/health") return write(response, 200, "ok");
  if (url.pathname === "/metrics") return write(response, 200, metrics(), "text/plain");
  if (url.pathname === "/api/auth/ws-ticket") return wsTicket(response, request, scenario);
  if (url.pathname === "/api/system/health") {
    if (scenario === "system-health-502") {
      return json(
        response,
        apiProblem("SYSTEM_HEALTH_FAILED", "mock system health failed", 502, {
          requestId: "e2e-system-health-502",
          retryAfterMs: 4000,
          source: "e2e-fixture",
          details: { route: url.pathname, scenario: "system-health-502" },
        }),
        502,
        { "retry-after": "4", "x-request-id": "e2e-system-health-502" },
      );
    }
    return json(response, systemHealthEnvelope());
  }
  if (url.pathname === "/api/system/venue-operation-health") {
    return json(
      response,
      scenario === largeTablesScenario
        ? largeVenueOperationHealth()
        : venueOperationHealth(scenario),
    );
  }
  if (url.pathname === "/api/system/venue-runtime-health") {
    return json(response, venueRuntimeHealth(scenario));
  }
  if (url.pathname === "/api/system/market-data-diagnostics") {
    return json(
      response,
      scenario === largeTablesScenario ? largeMarketDataDiagnostics() : marketDataDiagnostics(),
    );
  }
  if (url.pathname === "/api/watchlist") {
    return json(
      response,
      scenario === "watchlist-alerts-runtime"
        ? watchlistRuntimeEnvelope()
        : emptyWatchlistEnvelope(),
    );
  }
  if (url.pathname === "/api/alerts/rules") {
    return json(
      response,
      scenario === "watchlist-alerts-runtime"
        ? alertRulesRuntimeEnvelope()
        : emptyAlertRulesEnvelope(),
    );
  }
  if (url.pathname === "/api/strategy/main-kinds") return json(response, mainStrategyKinds());
  if (url.pathname === "/api/v1/strategy/kinds") return json(response, strategyKinds());
  if (url.pathname === "/api/arbitrage/funding-rates") return json(response, fundingRatesEnvelope());
  if (url.pathname === "/api/exchanges/credentials") {
    return venueCredentialsRoute(response, request, scenario);
  }
  if (url.pathname === "/api/trading/status") return json(response, tradingStatus());
  if (url.pathname === "/api/trading/adapters") return json(response, tradingAdapters(scenario));
  if (url.pathname === "/api/trading/rest/endpoints") return json(response, restEndpoints());
  if (url.pathname === "/api/trading/fee-schedules") return json(response, feeSchedules());
  if (url.pathname === "/api/trading/kill-switch") return killSwitch(response, scenario);
  if (url.pathname === "/api/trading/ws/venues") return json(response, exchangeWsVenues(scenario));
  if (url.pathname === "/api/trading/credentials/env-template") return json(response, envTemplate());
  if (url.pathname === "/api/trading/portfolio/snapshot") {
    if (scenario === "portfolio-snapshot-502") {
      return portfolioSnapshotFailure(response);
    }
    if (scenario === "position-close-denied") {
      return json(response, portfolioSnapshotEnvelope(positionCloseDeniedSnapshot()));
    }
    if (scenario === "position-cancel-denied") {
      return json(response, portfolioSnapshotEnvelope(positionCancelDeniedSnapshot()));
    }
    if (scenario === "pr-er-gate-runtime") {
      return json(response, prErGatePortfolioEnvelope());
    }
    return json(response, portfolioSnapshotEnvelope());
  }
  if (url.pathname === "/api/trading/account-state") {
    return json(response, accountStateForScenario(scenario));
  }
  if (url.pathname === "/api/trading/positions") return json(response, positionEnvelope(portfolioSnapshot().positions));
  if (url.pathname === "/api/trading/balances") return json(response, balanceEnvelope(portfolioSnapshot().balances));
  if (url.pathname === "/api/trading/venues/quality") {
    if (scenario === "review-resources-502") {
      return json(
        response,
        apiProblem("VENUE_QUALITY_FAILED", "mock venue quality failed", 502, {
          requestId: "e2e-venue-quality-502",
          retryAfterMs: 5000,
          source: "e2e-fixture",
          details: { route: url.pathname, scenario: "review-resources-502" },
        }),
        502,
        { "retry-after": "5", "x-request-id": "e2e-venue-quality-502" },
      );
    }
    return json(response, venueQuality());
  }
  if (url.pathname === "/api/trading/execution-runs") {
    const hasExplicitRunContext = url.searchParams.has("runId");
    const rows = scenario === "order-cancel-denied" && hasExplicitRunContext
      ? [cancelableExecutionRun()]
      : [];
    return request.method === "GET"
      ? json(response, listEnvelope(rows))
      : methodNotAllowed(response, request, url.pathname);
  }
  if (url.pathname === "/api/trading/action-runs") {
    return request.method === "GET"
      ? json(response, actionRunsEnvelope())
      : methodNotAllowed(response, request, url.pathname);
  }
  if (url.pathname.startsWith("/api/trading/orders/") && url.pathname.endsWith("/cancel")) {
    return cancelOrderRoute(url, request, response, scenario);
  }
  if (url.pathname === "/api/trading/orders") {
    return request.method === "GET"
      ? json(response, listEnvelope([]))
      : methodNotAllowed(response, request, url.pathname);
  }
  if (
    scenario === "position-close-denied"
    && url.pathname === "/api/trading/portfolio/positions/mock/MU/close"
  ) {
    return positionCloseDenied(response, request, url.pathname);
  }
  if (url.pathname === "/api/review/executed") {
    if (scenario === "review-stale-retry") {
      return reviewExecutedStaleRetry(response, scenario);
    }
    if (scenario === "review-executed-502") {
      return json(
        response,
        apiProblem("REVIEW_EXECUTED_FAILED", "mock review executed failed", 502, {
          requestId: "e2e-review-executed-502",
          retryAfterMs: 3000,
          source: "e2e-fixture",
          details: { route: url.pathname, scenario: "review-executed-502" },
        }),
        502,
        { "retry-after": "3", "x-request-id": "e2e-review-executed-502" },
      );
    }
    return json(
      response,
      scenario === largeTablesScenario
        ? largeReviewEnvelope(url, "execution_ledger", largeExecutedTrade, 1_000)
        : reviewEnvelope([], "execution_ledger"),
    );
  }
  if (url.pathname === "/api/review/missed") {
    if (scenario === "review-missed-502") {
      return json(
        response,
        apiProblem("REVIEW_MISSED_FAILED", "mock review missed failed", 502, {
          requestId: "e2e-review-missed-502",
          retryAfterMs: 3000,
          source: "e2e-fixture",
          details: { route: url.pathname, scenario: "review-missed-502" },
        }),
        502,
        { "retry-after": "3", "x-request-id": "e2e-review-missed-502" },
      );
    }
    return json(
      response,
      scenario === largeTablesScenario
        ? largeReviewEnvelope(url, "missed_opportunity_store", largeMissedOpportunity, 1_000)
        : reviewEnvelope([], "missed_opportunity_store"),
    );
  }
  if (url.pathname === "/api/review/strategy-performance") {
    if (scenario === "review-resources-502") {
      return json(
        response,
        apiProblem(
          "REVIEW_STRATEGY_PERFORMANCE_FAILED",
          "mock review strategy performance failed",
          502,
          {
            requestId: "e2e-review-strategy-502",
            retryAfterMs: 4000,
            source: "e2e-fixture",
            details: { route: url.pathname, scenario: "review-resources-502" },
          },
        ),
        502,
        { "retry-after": "4", "x-request-id": "e2e-review-strategy-502" },
      );
    }
    return json(response, reviewEnvelope([], "execution_ledger"));
  }
  if (url.pathname === "/api/history/opportunities") return json(response, opportunityHistory());
  if (url.pathname === "/api/history/funding-diffs") return json(response, historyFundingDiffs());
  if (url.pathname === "/api/venues/index-compositions/fetch") {
    return json(response, indexCompositionEnvelope(url));
  }
  if (url.pathname === "/api/v3/arbitrage/opportunities/list") {
    if (scenario === "typed-rate-limit") {
      return json(
        response,
        apiProblem("MARKET_DATA_RATE_LIMITED", "mock opportunity list rate limited", 429, {
          requestId: "e2e-rate-limit-1",
          retryAfterMs: 2000,
          source: "e2e-fixture",
          details: {
            venue: "mock",
            operation: "opportunity_list",
            path: url.pathname,
            status: 429,
            source: "e2e-fixture",
            scenario: "typed-rate-limit",
          },
        }),
        429,
        { "retry-after": "2", "x-request-id": "e2e-rate-limit-1" },
      );
    }
    if (scenario === "typed-upstream-502") {
      return json(
        response,
        apiProblem("UPSTREAM_HTTP", "mock upstream gateway failed", 502, {
          requestId: "e2e-upstream-502",
          source: "e2e-fixture",
          details: { route: url.pathname, scenario: "typed-upstream-502" },
        }),
        502,
        { "x-request-id": "e2e-upstream-502" },
      );
    }
    if (scenario === "typed-auth-401") {
      return json(
        response,
        apiProblem("UNAUTHORIZED", "mock auth token missing", 401, {
          requestId: "e2e-auth-401",
          source: "e2e-fixture",
          details: { route: url.pathname, scenario: "typed-auth-401" },
        }),
        401,
        { "x-request-id": "e2e-auth-401" },
      );
    }
    if (scenario === largeTablesScenario) {
      const page = largeOpportunityPageFromUrl(url);
      return json(response, opportunityListResponse(page.rows, page.total, page.start, page.pageSize));
    }
    return json(response, opportunityListResponse([muOpportunity()]));
  }
  if (url.pathname.startsWith("/api/v3/arbitrage/opportunities/")) {
    if (url.pathname.endsWith("/detail")) {
      const id = url.pathname.split("/").at(-2) ?? "mock-mu-perp";
      return json(
        response,
        scenario === largeTablesScenario
          ? opportunityDetailEnvelope(largeOpportunityById(id))
          : opportunityDetailEnvelope(muOpportunity()),
      );
    }
    return json(
      response,
      scenario === largeTablesScenario
        ? largeOpportunityById(url.pathname.split("/").pop() ?? "")
        : muOpportunity(),
    );
  }
  if (url.pathname === "/api/v3/arbitrage/opportunities") {
    if (scenario === largeTablesScenario) {
      const page = largeOpportunityPage(0, 50);
      return json(response, opportunitiesResponse(page.rows, page.total));
    }
    return json(response, opportunitiesResponse([muOpportunity()]));
  }
  if (url.pathname === "/api/arbitrage/opportunities/mock-mu-perp/preview") {
    return json(response, hedgePreview(scenario));
  }
  if (url.pathname === "/api/arbitrage/opportunities/mock-mu-perp/confirm") {
    if (request.method !== "POST") {
      return methodNotAllowed(response, request, url.pathname);
    }
    if (scenario === "confirm-denied") {
      return json(
        response,
        apiProblem("HEDGE_CONFIRM_DENIED", "hedge confirm denied by execution policy", 403, {
          requestId: "req-confirm-denied",
          retryAfterMs: 12000,
          source: "execution_policy",
          details: { route: url.pathname, scenario, operation: "hedge_confirm" },
        }),
        403,
        { "retry-after": "12", "x-request-id": "req-confirm-denied" },
      );
    }
    if (scenario === "confirm-429") {
      return json(
        response,
        apiProblem("HEDGE_CONFIRM_RATE_LIMITED", "hedge confirm rate limited", 429, {
          requestId: "req-confirm-429",
          retryAfterMs: 7000,
          source: "e2e-fixture",
          details: { route: url.pathname, scenario },
        }),
        429,
        { "retry-after": "7", "x-request-id": "req-confirm-429" },
      );
    }
    if (scenario === "confirm-200-failed") {
      return json(response, {
        idempotencyKey: "preview-mu-001",
        status: "long_leg_failed",
        executionRun: null,
        longRecord: null,
        shortRecord: null,
        unwindRecord: null,
        problem: {
          code: "HEDGE_CONFIRM_HTTP_200_FAILED",
          message: "hedge confirm accepted transport but failed",
          status: 200,
          requestId: "req-confirm-200-failed",
          retryAfterMs: 7000,
          source: "e2e-fixture",
          details: { route: url.pathname, scenario },
        },
        partialOutcome: null,
        error: "hedge confirm accepted transport but failed",
      });
    }
    const executionRun = scenario === "order-cancel-denied" ? cancelableExecutionRun() : null;
    return json(response, {
      idempotencyKey: "preview-mu-001",
      status: "submitted",
      executionRun,
      longRecord: null,
      shortRecord: null,
      unwindRecord: null,
      problem: null,
      partialOutcome: null,
      error: null,
    });
  }
  if (url.pathname.startsWith("/api/exchanges/") && url.pathname.endsWith("/orderbook")) {
    return json(response, orderbookEnvelope(url));
  }
  return json(
    response,
    apiProblem("NOT_FOUND", `unhandled e2e fixture route: ${url.pathname}`, 404, {
      requestId: "e2e-route-not-found",
      source: "e2e-fixture",
      details: { route: url.pathname },
    }),
    404,
    { "x-request-id": "e2e-route-not-found" },
  );
}

function methodNotAllowed(response, request, pathname) {
  return json(
    response,
    apiProblem("METHOD_NOT_ALLOWED", `method ${request.method} not allowed`, 405, {
      requestId: "e2e-method-not-allowed",
      source: "e2e-fixture",
      details: { method: request.method, route: pathname },
    }),
    405,
    { "x-request-id": "e2e-method-not-allowed" },
  );
}

function venueCredentialsRoute(response, request, scenario) {
  if (request.method === "GET") return json(response, venueCredentials(scenario));
  if (request.method !== "POST") {
    return methodNotAllowed(response, request, "/api/exchanges/credentials");
  }
  if (scenario !== "credential-save-denied") {
    return methodNotAllowed(response, request, "/api/exchanges/credentials");
  }
  // Intentionally do not consume or retain the request body: it contains secrets.
  return json(
    response,
    apiProblem("CREDENTIAL_SAVE_DENIED", "credential save denied by secret policy", 403, {
      requestId: "req-credential-save-denied",
      retryAfterMs: 30000,
      source: "credential_policy",
      details: {
        route: "/api/exchanges/credentials",
        scenario,
        operation: "credential_update",
      },
    }),
    403,
    { "retry-after": "30", "x-request-id": "req-credential-save-denied" },
  );
}

function cancelOrderRoute(url, request, response, scenario) {
  if (request.method !== "POST") return methodNotAllowed(response, request, url.pathname);
  const executionCancel = scenario === "order-cancel-denied"
    && url.pathname === "/api/trading/orders/e2e-run-order-1/cancel";
  const positionCancel = scenario === "position-cancel-denied"
    && url.pathname === "/api/trading/orders/e2e-comp-order-1/cancel";
  if (!executionCancel && !positionCancel) {
    return methodNotAllowed(response, request, url.pathname);
  }
  const fixture = executionCancel
    ? {
        code: "ORDER_CANCEL_DENIED",
        message: "execution order cancel denied while venue finality is pending",
        requestId: "req-execution-cancel-denied",
        retryAfterMs: 8000,
        source: "order_runtime",
        operation: "execution_cancel_order",
      }
    : {
        code: "COMPENSATION_CANCEL_DENIED",
        message: "compensation cancel denied while order remains accepted",
        requestId: "req-position-cancel-denied",
        retryAfterMs: 9000,
        source: "close_run_runtime",
        operation: "close_run_compensation_cancel",
      };
  return json(
    response,
    apiProblem(fixture.code, fixture.message, 409, {
      requestId: fixture.requestId,
      retryAfterMs: fixture.retryAfterMs,
      source: fixture.source,
      details: { route: url.pathname, scenario, operation: fixture.operation },
    }),
    409,
    {
      "retry-after": String(fixture.retryAfterMs / 1000),
      "x-request-id": fixture.requestId,
    },
  );
}

function positionCloseDenied(response, request, pathname) {
  if (request.method !== "POST") return methodNotAllowed(response, request, pathname);
  return json(
    response,
    apiProblem("POSITION_CLOSE_DENIED", "position close denied by snapshot policy", 409, {
      requestId: "req-position-close-denied",
      retryAfterMs: 10000,
      source: "portfolio_close",
      details: { route: pathname, scenario: "position-close-denied", operation: "close_position" },
    }),
    409,
    { "retry-after": "10", "x-request-id": "req-position-close-denied" },
  );
}

function routeCounterKey(scenario, pathname) {
  return `${scenario ?? "default"}\t${pathname}`;
}

function nextRouteCount(scenario, pathname) {
  const key = routeCounterKey(scenario, pathname);
  const count = (routeCounters.get(key) ?? 0) + 1;
  routeCounters.set(key, count);
  return count;
}

function resetRouteCounters(scenario) {
  const prefix = `${scenario ?? "default"}\t`;
  for (const key of routeCounters.keys()) {
    if (key.startsWith(prefix)) routeCounters.delete(key);
  }
}

function routeCountsForScenario(scenario) {
  const prefix = `${scenario ?? "default"}\t`;
  const counts = {};
  for (const [key, count] of routeCounters.entries()) {
    if (key.startsWith(prefix)) counts[key.slice(prefix.length)] = count;
  }
  return counts;
}

function reviewExecutedStaleRetry(response, scenario) {
  const count = nextRouteCount(scenario, "/api/review/executed");
  if (count === 1) {
    const rows = [staleExecutedTrade()];
    return json(response, {
      ...reviewEnvelope(rows, "execution_ledger"),
      rowCount: 2,
      page: listPage(rows.length, 2, 0, rows.length),
    });
  }
  if (count === 2) {
    return json(
      response,
      apiProblem("REVIEW_EXECUTED_RATE_LIMITED", "mock review executed refresh rate limited", 429, {
        requestId: "e2e-review-stale-retry-2",
        retryAfterMs: 15000,
        source: "e2e-fixture",
        details: { route: "/api/review/executed", scenario },
      }),
      429,
      { "retry-after": "15", "x-request-id": "e2e-review-stale-retry-2" },
    );
  }
  return json(response, reviewEnvelope([staleExecutedTrade()], "execution_ledger"));
}

function wsTicket(response, request, scenario) {
  const authorization = request.headers.authorization ?? "";
  wsAuthEvents.push({
    scenario,
    type: "ws_ticket",
    authorized: authorization === "Bearer e2e-token",
  });
  if (scenario === "ws-auth" && authorization !== "Bearer e2e-token") {
    return json(
      response,
      apiProblem("UNAUTHORIZED", "mock websocket ticket bearer missing", 401, {
        requestId: "e2e-ws-ticket-auth",
        source: "e2e-fixture",
        details: { route: "/api/auth/ws-ticket", scenario },
      }),
      401,
      { "x-request-id": "e2e-ws-ticket-auth" },
    );
  }
  return json(response, {
    ticket: scenario === "ws-auth" ? "e2e-ws-ticket" : "e2e-default-ticket",
    expiresAtMs: now + 30_000,
  });
}

function write(response, status, body, contentType = "text/plain", headers = {}) {
  response.writeHead(status, {
    "content-type": `${contentType}; charset=utf-8`,
    "access-control-allow-origin": webBase,
    "access-control-allow-methods": "GET,POST,OPTIONS",
    "access-control-allow-headers": "content-type,authorization,accept,x-request-id,idempotency-key",
    "access-control-expose-headers": "retry-after,x-request-id",
    vary: "origin",
    ...headers,
  });
  response.end(body);
}

function json(response, body, status = 200, headers = {}) {
  write(response, status, JSON.stringify(body), "application/json", headers);
}

function apiProblem(code, message, status, options = {}) {
  const error = {
    code,
    message,
    status,
    source: options.source ?? "e2e-fixture",
  };
  if (options.requestId) error.requestId = options.requestId;
  if (options.retryAfterMs !== undefined) error.retryAfterMs = options.retryAfterMs;
  if (options.details) error.details = options.details;
  return {
    error,
  };
}

function send(client, payload) {
  client.send(JSON.stringify(payload));
}

function parseJson(value) {
  try {
    return JSON.parse(value);
  } catch {
    return null;
  }
}

function boundedInt(value, fallback, min, max) {
  const parsed = Number.parseInt(value ?? "", 10);
  if (!Number.isFinite(parsed)) return fallback;
  return Math.min(Math.max(parsed, min), max);
}

function metrics() {
  return [
    "# TYPE crypto_arb_market_cache_hit_ratio gauge",
    "crypto_arb_market_cache_hit_ratio 0.99",
    "# TYPE crypto_arb_http_requests_total counter",
    'crypto_arb_http_requests_total{exchange="mock",method="GET",path="/api/v3/arbitrage/opportunities/list"} 1',
    "# TYPE crypto_arb_last_scan_ms gauge",
    "crypto_arb_last_scan_ms 12",
    "",
  ].join("\n");
}

function opportunitiesResponse(opportunities, total = opportunities.length) {
  return {
    opportunities,
    count: total,
    totalCount: total,
    filteredCount: total,
    beforeLimitCount: total,
    returnedCount: opportunities.length,
    visibleCount: opportunities.length,
    executableCount: total,
    strategyCounts: { perp_cross: total },
    executableStrategyCounts: { perp_cross: total },
    mainP0Counts: opportunityCountBreakdown(total),
    registryCounts: opportunityCountBreakdown(total),
    meta: opportunityScanMeta(total),
    status: "fresh",
    scope: "main_p0",
    queryKey: "scope=main_p0",
    cachedAt: new Date(now).toISOString(),
    observedAtMs: now,
    freshnessMs: 0,
    retryAfterMs: null,
    error: null,
    partialFailures: [],
    source: "e2e-fixture",
  };
}

function opportunityListResponse(
  opportunities,
  total = opportunities.length,
  startOffset = 0,
  pageSize = opportunities.length,
) {
  const rows = opportunities.map(opportunityListRow);
  const nextOffset = startOffset + rows.length;
  return {
    rows,
    page: {
      pageSize,
      startOffset,
      returnedCount: rows.length,
      totalRows: total,
      hasNextPage: nextOffset < total,
      nextCursor: nextOffset < total ? String(nextOffset) : null,
      sortKey: "score",
      snapshotId: "e2e-snapshot-1",
    },
    scopeMeta: opportunityScopeMeta(total),
    mainP0Counts: opportunityCountBreakdown(total),
    registryCounts: opportunityCountBreakdown(total),
    meta: opportunityScanMeta(total),
    status: "fresh",
    scope: "main_p0",
    queryKey: "scope=main_p0",
    source: "e2e-fixture",
    cachedAt: new Date(now).toISOString(),
    observedAtMs: now,
    freshnessMs: 0,
    retryAfterMs: null,
    error: null,
    partialFailures: [],
  };
}

function opportunityStreamEvent(opportunities, total = opportunities.length) {
  const rows = opportunities.map(opportunityListRow);
  return {
    event: "snapshot_invalidated",
    snapshotId: "e2e-snapshot-1",
    scopeMeta: opportunityScopeMeta(total),
    changedIds: rows.map((row) => row.id),
    changedRows: rows,
    removedIds: [],
    topIds: rows.map((row) => row.id),
    mainP0Counts: opportunityCountBreakdown(total),
    registryCounts: opportunityCountBreakdown(total),
    meta: opportunityScanMeta(total),
    status: "fresh",
    scope: "main_p0",
    queryKey: "scope=main_p0",
    source: "e2e-fixture-ws",
    cachedAt: new Date(now).toISOString(),
    observedAtMs: now,
    freshnessMs: 0,
    retryAfterMs: null,
    error: null,
    partialFailures: [],
  };
}

function opportunityScopeMeta(count) {
  return {
    globalTotalCount: count,
    strategyScopeCount: count,
    symbolScopeCount: count,
    filteredCount: count,
    pageCount: 1,
    candidateCount: count,
    emittedCount: count,
  };
}

function opportunityCountBreakdown(count) {
  return {
    totalCount: count,
    executableCount: count,
    strategyCounts: { perp_cross: count },
    executableStrategyCounts: { perp_cross: count },
  };
}

function opportunityScanMeta(count) {
  return {
    candidateCount: count,
    emittedCount: count,
    droppedExtremeYieldCount: 0,
    droppedNonPositiveYieldCount: 0,
    droppedUnprofitableAfterCostCount: 0,
    droppedBelowMinNetYieldCount: 0,
    historyAppendOk: true,
    scanMs: 12,
    publishMs: 1,
    marketDataProblemCount: 0,
    degradedVenues: [],
    marketDataStatus: null,
    coverage: {
      fundingSymbols: count,
      fundingVenues: 2,
      fundingRows: count * 2,
      perpTickers: 2,
      spotTicks: 0,
      optionQuotes: 0,
      indexCompositions: 0,
    },
  };
}

function opportunityListRow(opportunity) {
  const cost = opportunity.executionCost;
  const oneCycleNetBps = cost.grossEdgeBps - cost.totalCostBps;
  return {
    id: opportunity.id,
    symbol: opportunity.symbol,
    strategyKind: opportunity.strategyKind,
    strategyCategory: opportunity.strategyCategory,
    typeLabel: opportunity.typeLabel,
    longLeg: opportunityListLeg(
      opportunity.longExchange,
      `${opportunity.longExchange} 做多永续`,
      opportunity.longPrice,
      opportunity.longLegDepthUsd5bps,
    ),
    shortLeg: opportunityListLeg(
      opportunity.shortExchange,
      `${opportunity.shortExchange} 做空永续`,
      opportunity.shortPrice,
      opportunity.shortLegDepthUsd5bps,
    ),
    metrics: {
      score: opportunity.score,
      riskLevel: opportunity.riskLevel,
      netSingleYield: opportunity.netSingleYield,
      annualizedFundingBps: opportunity.annualizedFundingBps,
      oneCycleNetBps,
      timeToSettlementMs: opportunity.timeToSettlementMs,
      settlementCountdownSeconds: opportunity.settlementCountdownSeconds,
      liquidityScore: opportunity.liquidityScore,
    },
    cost: {
      verified: true,
      grossEdgeBps: cost.grossEdgeBps,
      totalCostBps: cost.totalCostBps,
      wearBps: cost.wearBps,
      oneCycleNetBps,
      oneCycleCoversCost: oneCycleNetBps > 0,
      breakevenPeriods: cost.breakevenPeriods,
      breakevenHours: cost.breakevenHours,
      recommendedHoldHours: cost.recommendedHoldHours,
      netBpsAtRecommendedHold: cost.netBpsAtRecommendedHold,
      feeEvidenceCount: 2,
      feeEvidenceComplete: true,
    },
    execution: {
      eligible: opportunity.executionEligible,
      blockers: opportunity.executionBlockers,
      optimalPosition: opportunity.optimalPosition,
      maxPosition: opportunity.maxPosition,
    },
    dataSource: opportunity.dataSource,
    updatedAt: opportunity.updatedAt,
  };
}

function opportunityListLeg(venue, action, price, depthUsd5bps) {
  return {
    venue,
    action,
    price,
    depthUsd5bps,
    marketEvidence: {
      venue,
      symbol: "MU",
      price,
      health: {
        quality: "fresh",
        source: "local_cache",
        freshnessMs: 0,
        retryAfterMs: null,
        lastError: null,
        observedAtMs: now,
        coverage: null,
        problem: null,
      },
    },
  };
}

function fundingWindow(cycles, windowHours, currentPercentile) {
  return {
    cycles,
    windowHours,
    sampleCount: cycles,
    meanDiffBps: 9.3129,
    p50DiffBps: 8.9,
    p75DiffBps: 9.1,
    p90DiffBps: 9.4,
    p95DiffBps: 9.7,
    stddevDiffBps: 0.6,
    positiveRatio: 1,
    reversalCount: 0,
    currentPercentile,
    source: "e2e-fixture",
    freshnessMs: 0,
    sampleHealth: "ok",
    problem: null,
    retryAfterMs: null,
  };
}

function largeOpportunityPageFromUrl(url) {
  const pageSize = boundedInt(url.searchParams.get("pageSize"), 50, 1, 100);
  const start = boundedInt(url.searchParams.get("cursor"), 0, 0, 499);
  return largeOpportunityPage(start, pageSize);
}

function largeOpportunityPage(start, pageSize) {
  const total = 500;
  const end = Math.min(start + pageSize, total);
  const rows = [];
  for (let index = start; index < end; index += 1) {
    rows.push(largeOpportunity(index));
  }
  return { rows, total, start, pageSize };
}

function largeOpportunityById(id) {
  const match = /mock-mu-perp-(\d+)/.exec(id);
  return largeOpportunity(match ? Number(match[1]) : 0);
}

function largeOpportunity(index) {
  const symbol = `MU${String(index + 1).padStart(3, "0")}`;
  return {
    ...muOpportunity(),
    id: `mock-mu-perp-${index}`,
    symbol,
    score: 99 - (index % 80) * 0.5,
    longPrice: 664.63 + index * 0.01,
    shortPrice: 664.2 + index * 0.01,
    optimalPosition: 750 + index,
    maxPosition: 5_000 + index,
    volume24h: 12_000_000 + index * 1_000,
    longVolume24h: 7_000_000 + index * 800,
    shortVolume24h: 5_000_000 + index * 700,
    dataSource: "e2e-large-fixture",
    updatedAt: new Date(now - index * 1_000).toISOString(),
    strategyDescription: `long hyperliquid:km ${symbol} / short KuCoin ${symbol}USDTM`,
  };
}

function muOpportunity() {
  const fundingWindows = [
    fundingWindow(1, 8, 76),
    fundingWindow(3, 24, 84),
    fundingWindow(9, 72, 91),
  ];
  return {
    id: "mock-mu-perp",
    symbol: "MU",
    type: "cross_exchange",
    typeLabel: "永续跨所",
    longExchange: "hyperliquid:km",
    shortExchange: "kucoin",
    spread8h: 0.093129,
    longRate8h: -0.08,
    shortRate8h: 0.013129,
    longRate: -0.08,
    shortRate: 0.013129,
    singleYield: 0.093129,
    netSingleYield: 0.093129,
    rawSingleYield: 0.093129,
    settlementInterval: 8,
    riskAdjustedYield: 0.081,
    tradingCostRate: 0.00021,
    minHoldingPeriods: 2,
    riskLevel: "low",
    volatility: 0.18,
    sharpeRatio: 2.1,
    score: 88,
    recommendation: "buy",
    optimalPosition: 750,
    maxPosition: 5_000,
    liquidityScore: 0.92,
    volume24h: 12_000_000,
    longVolume24h: 7_000_000,
    shortVolume24h: 5_000_000,
    dataSource: "e2e-fixture",
    confidence: 0.96,
    updatedAt: new Date(now).toISOString(),
    longFundingInterval: 8,
    shortFundingInterval: 8,
    settlementTimeDiff: false,
    strategyDescription: "long hyperliquid:km MU / short KuCoin MUUSDTM",
    longAction: "做多",
    shortAction: "做空",
    longNextFundingTime: now + 60 * 60 * 1000,
    shortNextFundingTime: now + 60 * 60 * 1000,
    timeToSettlementMs: 60 * 60 * 1000,
    isSnipeReady: true,
    longPrice: 664.63,
    shortPrice: 664.2,
    priceDeviation: 0.00064,
    basisSpread: 0.00093,
    basisAnnualCost: 1.0198,
    riskWarnings: [],
    executionEligible: true,
    executionBlockers: [],
    executionCost: {
      grossEdgeBps: 9.3129,
      feeBps: 1.5,
      wearBps: 2,
      totalCostBps: 3.5,
      breakevenPeriods: 1,
      breakevenHours: 8,
      recommendedHoldPeriods: 2,
      recommendedHoldHours: 16,
      netBpsAtRecommendedHold: 15.1,
    },
    strategyKind: "perp_cross",
    strategyCategory: "futures",
    basisBps: 9.3129,
    annualizedFundingBps: 10_198,
    predictedNextFunding: { longBps: -8, shortBps: 1.3129, netBps: 9.3129, confidence: 0.96 },
    fundingDiffWindow: fundingWindows[2],
    fundingDiffWindows: fundingWindows,
    longLegDepthUsd5bps: 520_000,
    shortLegDepthUsd5bps: 480_000,
    borrowCostBpsPerDay: 0,
    fundingWindowAlignmentMinutes: 0,
    fundingCapDistanceBps: 34,
    minHoldHours: 8,
    settlementCountdownSeconds: 3_600,
    underlyingEvent: null,
  };
}

function hedgePreview(scenario = null) {
  const isPrEl = scenario?.startsWith("pr-el-binance-");
  const isPrEo = scenario?.startsWith("pr-eo-kucoin-");
  const symbol = isPrEl ? "BTCUSDC" : isPrEo ? "BTC-USDC" : "MU";
  const longExchange = isPrEl ? "binance" : isPrEo ? "kucoin" : "hyperliquid:km";
  const shortExchange = isPrEl ? "binance" : "kucoin";
  const longLeg = orderIntent("long-leg", longExchange, symbol, "buy");
  const shortLeg = orderIntent("short-leg", shortExchange, symbol, "sell");
  const longOrderPlan = orderCompilePlan("long", longExchange, symbol, scenario);
  const shortOrderPlan = orderCompilePlan("short", shortExchange, symbol, scenario);
  return {
    opportunityId: "mock-mu-perp",
    ticket: {
      ticketId: "ticket-mu-001",
      opportunityId: "mock-mu-perp",
      strategy: "perp_cross",
      symbol,
      createdAtMs: now,
      // Keep the browser contract deterministic even when Wasm compilation delays startup.
      expiresAtMs: Date.now() + 60_000,
      longLeg: ticketLeg("long", longExchange, symbol, "buy", 664.63),
      shortLeg: ticketLeg("short", shortExchange, symbol, "sell", 664.2),
      cost: {
        grossEdgeBps: 9.3129,
        feeBps: 1.5,
        wearBps: 2,
        totalCostBps: 3.5,
        breakevenPeriods: 1,
        breakevenHours: 8,
        recommendedHoldPeriods: 2,
        recommendedHoldHours: 16,
        netBpsAtRecommendedHold: 15.1,
      },
      sizing: {
        requestedCapitalUsd: 750,
        leverage: 1,
        targetNotionalUsd: 750,
        maxExecutableNotional: {
          status: "available",
          amountUsd: 480_000,
          longLegDepthUsd: 520_000,
          shortLegDepthUsd: 520_000,
        },
      },
      guards: [
        { key: "fresh_books", label: "盘口新鲜度", passed: true, detail: "Fresh WS/cache" },
        { key: "depth", label: "20bps 深度", passed: true, detail: "$480K 可用" },
        { key: "no_blockers", label: "硬阻断", passed: true, detail: "无硬阻断" },
      ],
      blockers: [],
    },
    longLeg,
    longRisk: { allowed: true, reasons: [], computedNotional: 750 },
    shortLeg,
    shortRisk: { allowed: true, reasons: [], computedNotional: 750 },
    ticketOrderPlans: ticketOrderPlans("ticket-mu-001", longOrderPlan, shortOrderPlan),
    // The frontend deliberately ignores legacy top-level plans: test fixtures do too.
    longOrderPlan: null,
    shortOrderPlan: null,
    estimatedFundingPer8hUsd: 0.7,
    estimatedOpenCostUsd: 0.13,
    estimatedCloseCostUsd: 0.13,
    estimatedSlippageUsd: 0.2,
    currentAccountLiqDistancePct: 31,
    afterHedgeLiqDistancePct: 29,
    usedCapitalUsd: 750,
    maxLossUsd: 0.45,
    idempotencyKey: "preview-mu-001",
  };
}

function ticketOrderPlans(ticketId, longOrderPlan, shortOrderPlan) {
  return {
    ticketId,
    long: ticketOrderPlanEvidence(longOrderPlan),
    short: ticketOrderPlanEvidence(shortOrderPlan),
  };
}

function ticketOrderPlanEvidence(compilePlan) {
  return {
    compilePlan,
    identityPlan: orderIdentityPlan(compilePlan),
  };
}

function orderIdentityPlan(compilePlan) {
  const fields = identityFields(compilePlan.clientOrderIdPolicy?.constraints ?? []);
  const declaredCanonicalSymbol = identityField(fields, "identity.canonical_symbol");
  const evidenceRequired = isBinancePerp(compilePlan.exchange, compilePlan.product)
    || Object.keys(fields).length > 0;
  const evidence = identityEvidenceRows(fields);
  const plan = {
    evidenceRequired,
    canonicalSymbol: declaredCanonicalSymbol ?? compilePlan.symbol,
    ...optionalIdentityField("nativeSymbol", identityField(fields, "identity.native_symbol")),
    ...optionalIdentityField("settleAsset", identityField(fields, "identity.settle_asset")),
    ...optionalIdentityField("quoteAsset", identityField(fields, "identity.quote_asset")),
    product: compilePlan.product,
    clientOrderIdPolicy: compilePlan.clientOrderIdPolicy,
    exchangeOrderIdFinalitySource: identityFinalitySource(
      identityField(fields, "identity.exchange_order_id_finality_source"),
    ),
    evidence,
    blockers: [],
  };
  plan.blockers = identityBlockers(plan, compilePlan.symbol, declaredCanonicalSymbol, fields);
  return plan;
}

function identityFields(constraints) {
  const fields = {};
  for (const constraint of constraints) {
    const separator = constraint.indexOf("=");
    if (separator <= 0) continue;
    const key = constraint.slice(0, separator);
    const value = constraint.slice(separator + 1);
    if (key.startsWith("identity.") && nonEmpty(value)) fields[key] = value;
  }
  return fields;
}

function identityField(fields, key) {
  const value = fields[key];
  return typeof value === "string" && nonEmpty(value) ? value : undefined;
}

function optionalIdentityField(key, value) {
  return value === undefined ? {} : { [key]: value };
}

function identityEvidenceRows(fields) {
  return [
    ["metadata", "metadata"],
    ["user_stream", "user_stream"],
    ["order_finality", "order_finality"],
    ["fee", "fee"],
  ].map(([kind, key]) => {
    const prefix = `identity.evidence.${key}`;
    return {
      kind,
      status: identityEvidenceStatus(identityField(fields, `${prefix}.status`)),
      ...optionalIdentityField("evidenceId", identityField(fields, `${prefix}.evidence_id`)),
      ...optionalIdentityField("source", identityField(fields, `${prefix}.source`)),
      ...optionalIdentityField("fixtureId", identityField(fields, `${prefix}.fixture_id`)),
      ...optionalIdentityField("parserTest", identityField(fields, `${prefix}.parser_test`)),
      ...optionalIdentityField("detail", identityField(fields, `${prefix}.detail`)),
    };
  });
}

function identityEvidenceStatus(value) {
  if (value === "verified") return "verified";
  if (value === "mismatched") return "mismatched";
  return "unavailable";
}

function identityFinalitySource(value) {
  if (value === "private_user_stream") return "private_user_stream";
  if (value === "rest_order_query") return "rest_order_query";
  if (value === "private_user_stream_with_rest_fallback") {
    return "private_user_stream_with_rest_fallback";
  }
  return "unavailable";
}

function identityBlockers(plan, compileSymbol, declaredCanonicalSymbol, fields) {
  if (!plan.evidenceRequired) return [];
  const blockers = [];
  identityRequiredMatch(
    blockers,
    "ORDER_IDENTITY_CANONICAL_SYMBOL",
    declaredCanonicalSymbol,
    compileSymbol,
  );
  identityRequiredValue(blockers, "ORDER_IDENTITY_NATIVE_SYMBOL_MISSING", plan.nativeSymbol);
  identityRequiredValue(blockers, "ORDER_IDENTITY_SETTLE_ASSET_MISSING", plan.settleAsset);
  identityRequiredValue(blockers, "ORDER_IDENTITY_QUOTE_ASSET_MISSING", plan.quoteAsset);
  if (identityField(fields, "identity.product") !== plan.product) {
    blockers.push("ORDER_IDENTITY_PRODUCT_MISMATCH");
  }
  if (!clientOrderIdPolicyReady(plan.clientOrderIdPolicy)) {
    blockers.push("ORDER_IDENTITY_CLIENT_ID_POLICY_UNVERIFIED");
  }
  if (plan.exchangeOrderIdFinalitySource === "unavailable") {
    blockers.push("ORDER_IDENTITY_FINALITY_SOURCE_UNAVAILABLE");
  }
  for (const [kind, code] of [
    ["metadata", "METADATA"],
    ["user_stream", "USER_STREAM"],
    ["order_finality", "ORDER_FINALITY"],
    ["fee", "FEE"],
  ]) {
    const evidence = plan.evidence.find((row) => row.kind === kind);
    if (!identityEvidenceVerified(evidence)) {
      blockers.push(`ORDER_IDENTITY_${code}_EVIDENCE_UNAVAILABLE`);
    }
  }
  return blockers;
}

function identityRequiredMatch(blockers, code, actual, expected) {
  if (actual === undefined) blockers.push(`${code}_MISSING`);
  else if (actual !== expected) blockers.push(`${code}_MISMATCH`);
}

function identityRequiredValue(blockers, code, value) {
  if (value === undefined || !nonEmpty(value)) blockers.push(code);
}

function clientOrderIdPolicyReady(policy) {
  return nonEmpty(policy?.publicClientOrderId)
    && nonEmpty(policy?.venueClientOrderId)
    && policy.derivation !== "unsupported"
    && policy.derivation !== "rejected"
    && (policy.blockers ?? []).length === 0;
}

function identityEvidenceVerified(evidence) {
  return evidence?.status === "verified"
    && nonEmpty(evidence.evidenceId)
    && nonEmpty(evidence.source);
}

function isBinancePerp(venue, product) {
  return venue.split(/[:_-]/)[0]?.toLowerCase() === "binance" && product === "perp";
}

function nonEmpty(value) {
  return typeof value === "string" && value.trim().length > 0;
}

function orderCompilePlan(role, exchange, symbol, scenario) {
  const isPrEl = scenario?.startsWith("pr-el-binance-");
  const isPrEo = scenario?.startsWith("pr-eo-kucoin-");
  const identityStatus = scenario === "pr-el-binance-runtime-unavailable"
    || scenario === "pr-eo-kucoin-runtime-unavailable"
    ? "unavailable"
    : "verified";
  const canonicalSymbol = scenario === "pr-el-binance-runtime-unavailable"
    ? "BTCUSDT"
    : symbol;
  const constraints = scenario === "pr-el-binance-identity-missing"
    ? []
    : identityConstraints({
        canonicalSymbol,
        nativeSymbol: isPrEo ? "XBTUSDCM" : symbol,
        quoteAsset: isPrEl || isPrEo ? "USDC" : "USDT",
        settleAsset: isPrEl || isPrEo ? "USDC" : "USDT",
        status: identityStatus,
        venue: isPrEo ? "kucoin" : "binance",
      });
  return {
    role,
    exchange,
    symbol,
    clientOrderIdPolicy: {
      venue: exchange,
      venueFamily: exchange,
      venueField: exchange === "binance" ? "newClientOrderId" : "clientOrderId",
      publicClientOrderId: `public-${role}-order`,
      venueClientOrderId: `venue-${role}-order`,
      derivation: "identity",
      policyVersion: `${exchange}-identity-v1`,
      officialFormat: "e2e verified client id policy",
      maxLength: 36,
      supportsQueryByClientId: true,
      supportsCancelByClientId: true,
      constraints,
      blockers: [],
      officialDocUrls: isPrEl
        ? ["https://developers.binance.com/docs/derivatives"]
        : isPrEo
          ? ["https://www.kucoin.com/docs-new/rest/futures-trading/market-data/get-all-symbols"]
          : [],
    },
    product: "perp",
    requestedOrderType: "limit",
    effectiveOrderType: "limit",
    requestedTimeInForce: "ioc",
    effectiveTimeInForce: "ioc",
    availableOrderTypes: ["limit", "market", "post_only"],
    availableTimeInForce: ["ioc", "fok", "gtc", "gtx"],
    availableMarginModes: ["cross", "isolated"],
    venueOrderKind: "limit",
    payloadPricePolicy: "limit_price",
    referencePrice: 664.63,
    protectionPrice: 664.63,
    payloadPrice: 664.63,
    summary: isPrEl
      ? "Binance USD-M USDC native order plan"
      : isPrEo
        ? "KuCoin native USDC multiplier sizing plan"
        : "e2e order plan",
    blockers: [],
  };
}

function identityConstraints({
  canonicalSymbol,
  nativeSymbol,
  quoteAsset,
  settleAsset,
  status,
  venue = "binance",
}) {
  const constraints = [
    `identity.canonical_symbol=${canonicalSymbol}`,
    `identity.native_symbol=${nativeSymbol}`,
    `identity.settle_asset=${settleAsset}`,
    `identity.quote_asset=${quoteAsset}`,
    "identity.product=perp",
    "identity.exchange_order_id_finality_source=private_user_stream_with_rest_fallback",
  ];
  for (const kind of ["metadata", "user_stream", "order_finality", "fee"]) {
    constraints.push(`identity.evidence.${kind}.status=${status}`);
    constraints.push(`identity.evidence.${kind}.evidence_id=${venue}-${kind}-capture`);
    constraints.push(`identity.evidence.${kind}.source=${venue}-futures-capture-registry`);
    constraints.push(`identity.evidence.${kind}.fixture_id=${venue}/${kind}.json`);
    constraints.push(`identity.evidence.${kind}.parser_test=${venue}_${kind}_fixture_contract`);
    if (status !== "verified") {
      constraints.push(
        `identity.evidence.${kind}.detail=unavailable without live credential capture`,
      );
    }
  }
  return constraints;
}

function opportunityHistory() {
  return {
    count: 1,
    rows: [
      {
        occurredAtMs: now,
        id: "mock-mu-perp",
        symbol: "MU",
        longExchange: "hyperliquid:km",
        shortExchange: "kucoin",
        spread8h: 0.093129,
        netYield: 0.093129,
        volume24hMin: 5_000_000,
        payload: muOpportunity(),
      },
    ],
    page: {
      limit: 6,
      maxLimit: 1000,
      returnedCount: 1,
      hasMore: false,
      nextCursor: null,
    },
    rowCap: rowCap(6, 1, 1, false, "history:e2e-fixture"),
    source: "e2e-fixture",
    observedAtMs: now,
    latestAtMs: now,
    freshnessMs: 0,
    problem: null,
    retryAfterMs: null,
    problems: [],
  };
}

function orderbookEnvelope(url) {
  const venue = decodeURIComponent(url.pathname.split("/")[3] ?? "mock");
  const symbol = url.searchParams.get("symbol") ?? "MU";
  return orderbookEnvelopeFor(venue, symbol, Number(url.searchParams.get("depth") ?? 5));
}

function orderbookEnvelopeFor(venue, symbol, depth = 5) {
  const price = symbol === "MU" && venue === "kucoin" ? 664.2 : 664.63;
  const health = freshMarketHealth();
  return {
    data: {
      symbol,
      exchange: venue,
      bids: [[price - 0.05, 750]],
      asks: [[price + 0.05, 750]],
      timestamp: now,
    },
    health,
    rowCap: rowCap(depth, 1, 1, false, `orderbook:${venue}:${symbol}`),
    rowEvidence: [
      {
        venue,
        symbol,
        operation: "orderbooks",
        health,
      },
    ],
    fanout: [],
  };
}

function opportunityDetailEnvelope(opportunity = muOpportunity()) {
  const symbol = opportunity.symbol ?? "MU";
  return {
    opportunity,
    longOrderbook: orderbookEnvelopeFor("hyperliquid:km", symbol),
    shortOrderbook: orderbookEnvelopeFor("kucoin", symbol),
    history: opportunityHistory(),
    longIndexComposition: indexCompositionFor("hyperliquid:km", symbol),
    shortIndexComposition: indexCompositionFor("kucoin", symbol),
    status: "fresh",
    source: "e2e-fixture",
    observedAtMs: now,
    freshnessMs: 0,
    retryAfterMs: null,
    error: null,
    partialFailures: [],
    requestId: "e2e-detail-1",
  };
}

function indexCompositionEnvelope(url) {
  const venue = url.searchParams.get("venue") ?? "mock";
  const symbol = url.searchParams.get("symbol") ?? "MU";
  return indexCompositionFor(venue, symbol);
}

function indexCompositionFor(venue, symbol) {
  return {
    data: null,
    health: unsupportedMarketHealth(`${venue} ${symbol} index composition unsupported in e2e`),
    rowCap: null,
    rowEvidence: [],
    fanout: [],
  };
}

function rowCap(maxRows, returnedCount, totalRows, lowerBound, source) {
  return {
    maxRows,
    returnedCount,
    totalRows,
    totalRowsIsLowerBound: lowerBound,
    truncated: totalRows > returnedCount,
    truncatedCount: Math.max(0, totalRows - returnedCount),
    source,
  };
}

function freshMarketHealth() {
  return {
    quality: "fresh",
    source: "local_cache",
    freshnessMs: 0,
    retryAfterMs: null,
    lastError: null,
    observedAtMs: now,
    coverage: null,
    problem: null,
  };
}

function unsupportedMarketHealth(lastError) {
  return {
    quality: "unsupported",
    source: "local_cache",
    freshnessMs: null,
    retryAfterMs: null,
    lastError,
    observedAtMs: now,
    coverage: null,
    problem: null,
  };
}

function fundingRatesEnvelope() {
  const rows = [
    {
      symbol: "MU",
      exchange: "hyperliquid:km",
      rate: -0.0008,
      rate8h: -0.0008,
      predictedRate: null,
      nextFundingTime: now + 60 * 60 * 1000,
      fundingInterval: 8,
      volume24h: 7_000_000,
      timestamp: now,
      smoothedRate: null,
      rateStd: null,
      isOutlier: false,
    },
    {
      symbol: "MU",
      exchange: "kucoin",
      rate: 0.00013129,
      rate8h: 0.00013129,
      predictedRate: null,
      nextFundingTime: now + 60 * 60 * 1000,
      fundingInterval: 8,
      volume24h: 5_000_000,
      timestamp: now,
      smoothedRate: null,
      rateStd: null,
      isOutlier: false,
    },
  ];
  const health = freshMarketHealth();
  return {
    data: rows,
    health,
    rowCap: null,
    rowEvidence: rows.map((row) => ({
      venue: row.exchange,
      symbol: row.symbol,
      operation: "funding_rates",
      health,
    })),
    fanout: [],
  };
}

function envTemplate() {
  return {
    lines: [
      { venue: "mock", fieldLabel: "API Key", key: "MOCK_API_KEY", configured: false },
      { venue: "mock", fieldLabel: "API Secret", key: "MOCK_API_SECRET", configured: false },
    ],
    text: "MOCK_API_KEY=\nMOCK_API_SECRET=",
  };
}

function venueCredentials(scenario = null) {
  const boundary = isSettingsCredentialRuntimeScenario(scenario);
  const validationEvidence = settingsCredentialValidationEvidence(scenario);
  return {
    venues: [
      {
        venue: "okx",
        label: "OKX",
        fields: [
          credentialField("api_key", "API Key", "OKX_API_KEY", boundary),
          credentialField("api_secret", "API Secret", "OKX_API_SECRET", boundary),
          credentialField("passphrase", "Passphrase", "OKX_PASSPHRASE", boundary),
          credentialField("live_key", "Live Key", "OKX_LIVE_API_KEY", boundary),
          credentialField("live_secret", "Live API Secret", "OKX_LIVE_API_SECRET", boundary),
          credentialField("live_passphrase", "Live Passphrase", "OKX_LIVE_PASSPHRASE", boundary),
        ],
        publicMarket: true,
        privateRead: true,
        testnetWrite: false,
        liveWrite: true,
        note: "Live guarded",
        ...(validationEvidence ? { validationEvidence } : {}),
      },
    ],
    secretStorage: {
      mode: "runtime_only",
      persistent: false,
      encrypted: false,
      atomicWrite: false,
      path: null,
      label: "仅进程内缓存",
      message: "E2E fixture does not persist secrets.",
      warning: "mock runtime credentials only",
    },
  };
}

function settingsCredentialValidationEvidence(scenario) {
  if (scenario === "settings-selected-venue-trading-runtime-all-ok-local-capture-shaped") {
    return credentialAllOkLocalCaptureValidationEvidence();
  }
  if (isSettingsCredentialRuntimeScenario(scenario)) {
    return credentialBoundaryValidationEvidence();
  }
  return null;
}

function credentialBoundaryValidationEvidence() {
  return {
    status: "unknown",
    checkedAtMs: now,
    probes: [
      credentialProbe("balance_read", "ok", "private_read", "credential_validation", "balance read probe passed"),
      credentialProbe("positions_read", "ok", "private_read", "credential_validation", "positions probe passed"),
      credentialProbe("open_orders_read", "ok", "private_read", "credential_validation", "open orders probe passed"),
      credentialProbe(
        "order_permission",
        "unknown",
        "place_cancel_order_stream",
        "safe_noop_probe",
        "safe/noop probe 未授予 live_write；仍需 live place/cancel/order stream 证据",
      ),
    ],
  };
}

function credentialAllOkLocalCaptureValidationEvidence() {
  return {
    status: "read_only_ok",
    checkedAtMs: now,
    probes: [
      credentialProbe("balance_read", "ok", "private_read", "credential_validation", "balance read probe passed"),
      credentialProbe("positions_read", "ok", "private_read", "credential_validation", "positions probe passed"),
      credentialProbe("open_orders_read", "ok", "private_read", "credential_validation", "open orders probe passed"),
      credentialProbe(
        "order_permission",
        "ok",
        "place_cancel_order_stream",
        "credential_validation",
        "本地 UI/CI gate：order_permission capture-shaped Ok，非真实交易所 live 样本",
        "req-settings-runtime-all-ok-order-permission",
      ),
    ],
  };
}

function credentialProbe(kind, status, scope, source, message, requestId = null) {
  return {
    kind,
    status,
    scope,
    source,
    message,
    checkedAtMs: now,
    requestId: requestId ?? (status === "unknown" ? "req-settings-order-permission-unknown" : null),
  };
}

function credentialField(key, label, envKey, configured = false) {
  return {
    key,
    label,
    envKey,
    configured,
    secret: true,
  };
}

function exchangeWsVenues(scenario = null) {
  const ready = "ready";
  return {
    venues: [
      {
        venue: "okx",
        label: "OKX",
        publicEndpoint: "wss://ws.okx.com:8443/ws/v5/public",
        privateEndpoint: "wss://ws.okx.com:8443/ws/v5/private",
        tradeEndpoint: "wss://ws.okx.com:8443/ws/v5/private",
        accountStream: wsOperation(ready, "account", "Spot/SWAP", "私有 account channel"),
        positionStream: wsOperation(ready, "positions", "SWAP", "私有 positions channel"),
        fillStream: wsOperation(ready, "orders fill*", "Spot/SWAP", "orders channel 携带 fill 字段"),
        orderStream: wsOperation(ready, "orders", "Spot/SWAP", "私有 orders channel"),
        placeOrder: wsOperation(ready, "order", "Spot/SWAP", "login 后 op=order"),
        cancelOrder: wsOperation(ready, "cancel-order", "Spot/SWAP", "login 后 op=cancel-order"),
        closePosition: wsOperation(ready, "order reduceOnly", "SWAP", "SWAP 平仓按 reduceOnly/posSide 映射"),
        orderStatus: wsOperation(ready, "orders", "Spot/SWAP", "实时订单状态"),
        authFields: ["api_key", "api_secret", "passphrase"],
        docs: [{ label: "api v5 websocket", url: "https://www.okx.com/docs-v5/en/" }],
        note: "Demo/实盘由 x-simulated-trading 区分；执行前校验双腿凭证与能力。",
      },
    ],
  };
}

function tradingAdapters(scenario = null) {
  const boundary = isSettingsCredentialRuntimeScenario(scenario);
  return {
    current: "mock",
    currentEnvironment: "paper",
    options: [
      {
        id: "mock",
        label: "Paper",
        environment: "paper",
        enabled: true,
        credentialsAvailable: true,
        capabilities: adapterCapabilities(false, false),
        disabledReason: null,
      },
      {
        id: "live_router",
        label: "实盘",
        environment: "live",
        enabled: boundary,
        credentialsAvailable: boundary,
        capabilities: adapterCapabilities(false, true),
        disabledReason: boundary
          ? null
          : "至少补齐一个交易所 API 字段组；每张 HedgeTicket 仍会校验双腿权限与运行态证据",
      },
    ],
    venues: boundary
      ? [
          {
            venue: "okx",
            environment: "live",
            credentialsAvailable: true,
            capabilities: adapterCapabilities(false, true),
            source: "live_adapter.capabilities",
            problem: null,
          },
        ]
      : [],
  };
}

function restEndpoints() {
  return {
    venues: [
      {
        venue: "okx",
        endpoints: [
          {
            method: "GET",
            path: "/api/v5/account/balance",
            weight: 1,
            checkedAt: "2026-07-02",
            docVersion: "okx-v5-account-balance-2026-07-02",
            schemaHash: "sha256:e2e-rest-endpoint",
            fixtureId: "e2e-okx-account-balance",
            parserTest: "okx_account_balance_parses_official_fixture",
            requestBuilderTest: "okx_account_balance_request_uses_official_path",
            authKind: "signed",
            docUrls: ["https://www.okx.com/docs-v5/en/"],
            useCases: ["account_balance"],
            dataKinds: ["account"],
            rateScopes: ["user"],
          },
        ],
      },
    ],
  };
}

function feeSchedules() {
  return {
    schema: {
      version: "fee_schedule_registry_v2",
      fingerprint: "7e08b1bd44fe99ef",
    },
    venues: [
      {
        venue: "okx",
        schedules: [
          {
            product: "perp",
            makerFeeBps: 2,
            takerFeeBps: 5,
            evidence: {
              evidenceId: "e2e-okx-perp-fee-schedule",
              sourceName: "OKX official fee schedule",
              sourceUrl: "https://www.okx.com/fees",
              checkedAtMs: now,
              effectiveAtMs: now,
              scheduleVersion: "okx-fee-schedule-2026-07-02",
              tier: "regular",
              scope: "perp",
              problem: null,
            },
            fixtureId: "e2e-okx-perp-fee-schedule",
            fixtureSymbol: "BTC-USDT-SWAP",
            snapshotTtlMs: 86_400_000,
          },
        ],
      },
    ],
  };
}

function adapterCapabilities(spot, perp) {
  return {
    spot,
    perp,
    limitOrders: true,
    marketOrders: true,
    postOnly: true,
    reduceOnly: true,
  };
}

function wsOperation(status, operation, product, note) {
  return {
    supported: true,
    status,
    operation,
    product,
    note,
    evidence: {
      checkedAt: "2026-07-02",
      docVersion: "okx-v5-private-ws-mock-2026-07-02",
      docUrl: "https://www.okx.com/docs-v5/en/",
      parserTest: "parses_order_update_rows",
      subscriptionTest: "positions_and_orders_use_any_inst_type",
      authKind: "login",
    },
  };
}

function tradingStatus() {
  return {
    adapter: "mock",
    environment: "paper",
    openOrderCount: 0,
    risk: {
      liveTradingEnabled: false,
      killSwitchActive: false,
      maxOrderNotional: 5_000,
      maxOpenOrders: 10,
      maxHedgeImbalancePct: 0.05,
      liquidationWarnPct: 0.2,
      liquidationDangerPct: 0.1,
      allowedExchanges: ["mock", "hyperliquid:km", "kucoin"],
      allowedSymbols: ["MU"],
    },
    wsChannels: {
      orders: "orders",
      execution: "execution",
      riskAlerts: "risk_alerts",
    },
  };
}

function killSwitch(response, scenario) {
  if (scenario === "extractor-422") {
    return json(
      response,
      apiProblem("REQUEST_BODY_INVALID", "invalid JSON request body", 422, {
        requestId: "e2e-extractor-422",
        source: "e2e-fixture",
        details: { extractor: "Json", kind: "data" },
      }),
      422,
      { "x-request-id": "e2e-extractor-422" },
    );
  }
  const status = tradingStatus();
  status.risk.killSwitchActive = true;
  return json(response, {
    status,
    summary: {
      previousActive: false,
      active: true,
      openOrderCount: status.openOrderCount,
      expectedOpenOrderCount: status.openOrderCount,
      reason: "e2e.kill_switch.enable",
      checkedAtMs: now,
    },
    actionRunId: "e2e-kill-switch-action",
    requestId: "e2e-kill-switch",
  });
}

function ticketLeg(role, exchange, symbol, side, price) {
  return {
    role,
    exchange,
    symbol,
    side,
    referencePrice: price,
    bid: price - 0.05,
    ask: price + 0.05,
    mid: price,
    depthUsd5bps: 520_000,
    depthUsd10bps: 680_000,
    depthUsd20bps: 900_000,
    maxNotionalUsd: 520_000,
    depthReason: null,
    fundingBps: role === "long" ? -8 : 1.3129,
    nextFundingTime: now + 60 * 60 * 1000,
    marketTimestampMs: now,
    blockers: [],
  };
}

function orderIntent(id, exchange, symbol, side) {
  return {
    id,
    source: "arbitrage_preview",
    strategy: "perp_cross",
    mode: "dry_run",
    exchange,
    symbol,
    side,
    orderType: "limit",
    quantity: 1.13,
    price: side === "buy" ? 664.63 : 664.2,
    reduceOnly: false,
    timeInForce: "ioc",
    postOnly: false,
    marginMode: "cross",
    leverage: 1,
    clientOrderId: `${id}-client`,
    createdAtMs: now,
  };
}

function portfolioSnapshot() {
  const balances = [mockBalance()];
  const problems = [positionRuntimeProblem()];
  const operationHealth = [
    balanceOperationHealth(balances.length),
    failedPositionOperationHealth(),
  ];
  return {
    summary: {
      totalNavUsd: 10_000,
      navChange24hPct: 0.12,
      netDeltaUsd: 0,
      netDeltaPctOfNav: 0,
      nakedExposureUsd: 0,
      nakedPositionCount: 0,
      realizedPnlTodayUsd: 0,
      pnlBreakdown: { fundingUsd: 0, priceUsd: 0, feeRebateUsd: 0 },
      updatedAtMs: now,
    },
    positions: [],
    balances,
    risk: {
      var991dUsd: 0,
      varPctOfNav: 0,
      fundingClustering: [],
      deltaConcentration: [],
      marginUtilization: [{ venue: "mock", utilizationPct: 2.5, maintenanceMarginUsd: 0, equityUsd: 10_000 }],
      hardLimits: {
        openOrdersUsed: 0,
        openOrdersMax: 10,
        maxSymbolNotionalUsd: 10_000,
        maxOrderNotionalUsd: 5_000,
        killSwitchActive: false,
      },
      updatedAtMs: now,
    },
    serverNowMs: now,
    degraded: true,
    problems,
    operationHealth,
    accountState: accountStateSnapshot(balances, problems, operationHealth),
  };
}

function mockBalance() {
  return {
    venue: "mock",
    currency: "USDC",
    total: 10_000,
    available: 9_750,
    frozen: 250,
    unrealizedPnl: 0,
  };
}

function accountStateSnapshot(balances, problems, operationHealth) {
  return {
    balances: {
      rows: balances,
      rowCount: balances.length,
      status: "fresh",
      source: "account_balance_runtime",
      observedAtMs: now,
      problems: [],
      operationHealth: [balanceOperationHealth(balances.length)],
      fieldQuality: [],
      rowHealth: [],
    },
    positions: {
      rows: [],
      rowCount: 0,
      status: "degraded",
      source: "account_position_runtime",
      observedAtMs: now,
      problems: [positionProbeProblem()],
      operationHealth: [failedPositionOperationHealth()],
      fieldQuality: [],
    },
    status: "degraded",
    source: "account_state_runtime",
    observedAtMs: now,
    problems,
    operationHealth,
    fieldQuality: [],
  };
}

function accountStateForScenario(scenario) {
  if (scenario === "position-close-denied") return positionCloseDeniedSnapshot().accountState;
  if (scenario === "position-cancel-denied") return positionCancelDeniedSnapshot().accountState;
  if (scenario === "pr-er-gate-runtime") return prErGatePortfolioSnapshot().accountState;
  return portfolioSnapshot().accountState;
}

function portfolioSnapshotEnvelope(snapshot = portfolioSnapshot()) {
  return {
    status: "fresh",
    source: "e2e-fixture",
    observedAtMs: now,
    snapshot,
    problem: null,
    problems: [],
    operationHealth: snapshot.operationHealth,
    retryAfterMs: null,
  };
}

function prErGatePortfolioEnvelope() {
  const snapshot = prErGatePortfolioSnapshot();
  const problem = prErGateAccountProblem();
  return {
    status: "degraded",
    source: "pr_er_gate_account_state",
    observedAtMs: now,
    snapshot,
    problem,
    problems: [problem],
    operationHealth: snapshot.operationHealth,
    retryAfterMs: 60_000,
  };
}

function prErGatePortfolioSnapshot() {
  const snapshot = portfolioSnapshot();
  const balance = {
    venue: "gate",
    currency: "USDT",
    total: 2_500,
    available: 2_100,
    frozen: 400,
    unrealizedPnl: 12,
  };
  const position = {
    ...mutationPosition(),
    venue: "gate",
    symbol: "BTC_USDT",
    quantity: 2,
    entryPrice: 60_000,
    markPrice: 60_100,
    maintenanceMarginRatio: 0,
    marginUsd: 120.2,
  };
  const accountPosition = {
    ...mutationAccountPosition(),
    exchange: "gate",
    symbol: "BTC_USDT",
    quantity: 2,
    entryPrice: 60_000,
    markPrice: 60_100,
    maintenanceMarginRatio: 0,
    margin: 120.2,
  };
  const problem = prErGateAccountProblem();
  const operationHealth = prErGateRuntimeHealthRows();
  const fieldQuality = prErGateFieldQuality();
  const accountBindings = [prErGateAccountBinding(problem)];
  return {
    ...snapshot,
    summary: {
      ...snapshot.summary,
      totalNavUsd: 2_500,
      updatedAtMs: now,
    },
    positions: [position],
    balances: [balance],
    risk: {
      ...snapshot.risk,
      marginUtilization: [{
        venue: "gate",
        utilizationPct: 4.808,
        maintenanceMarginUsd: 0,
        equityUsd: 2_500,
        estimated: true,
      }],
    },
    degraded: true,
    problems: [{
      scope: "portfolio",
      operation: "account_state",
      code: problem.code,
      message: problem.message,
      venue: "gate",
      retryAfterMs: problem.retryAfterMs,
      observedAtMs: now,
    }],
    operationHealth,
    accountState: {
      balances: {
        rows: [balance],
        rowCount: 1,
        status: "degraded",
        source: "pr_er_gate_account_state",
        observedAtMs: now,
        problems: [problem],
        operationHealth: operationHealth.filter((row) => row.operation === "balance"),
        fieldQuality,
        rowHealth: [],
        accountBindings,
      },
      positions: {
        rows: [accountPosition],
        rowCount: 1,
        status: "degraded",
        source: "pr_er_gate_account_state",
        observedAtMs: now,
        problems: [problem],
        operationHealth: operationHealth.filter((row) => row.operation === "positions"),
        fieldQuality,
        rowHealth: [],
        accountBindings,
      },
      status: "degraded",
      source: "pr_er_gate_account_state",
      observedAtMs: now,
      problems: [problem],
      operationHealth,
      fieldQuality,
    },
  };
}

function prErGateAccountProblem() {
  return {
    code: "GATE_ACCOUNT_EVIDENCE_INCOMPLETE",
    message: "Gate fee and maintenance evidence is incomplete; live credentials were not used",
    status: 409,
    requestId: "req-pr-er-gate-account-evidence",
    retryAfterMs: 60_000,
    source: "pr_er_gate_account_state",
    details: {
      venue: "gate",
      feeEvidence: "missing",
      maintenanceEvidence: "missing",
      liveCredentialsUsed: false,
    },
  };
}

function prErGateAccountBinding(problem) {
  return {
    venue: "gate",
    accountScope: null,
    status: "unverified",
    source: "pr_er_gate_account_state",
    checkedAtMs: now,
    freshnessMs: 0,
    credentialFingerprint: null,
    problem,
  };
}

function prErGateFieldQuality() {
  return [
    {
      subject: { kind: "account", venue: "gate" },
      field: "feeEvidence",
      status: "missing",
      source: "pr_er_gate_account_state",
      observedAtMs: now,
      problem: {
        code: "GATE_FEE_EVIDENCE_MISSING",
        message: "Gate fill fee evidence is absent; zero fee is not assumed",
        status: 409,
        requestId: "req-pr-er-gate-fee",
        source: "pr_er_gate_account_state",
      },
    },
    {
      subject: { kind: "position", venue: "gate", symbol: "BTC_USDT", side: "long" },
      field: "maintenanceMarginRatio",
      status: "missing",
      source: "pr_er_gate_account_state",
      observedAtMs: now,
      problem: {
        code: "GATE_MAINTENANCE_EVIDENCE_MISSING",
        message: "Gate maintenance ratio evidence is absent; utilization remains estimated",
        status: 409,
        requestId: "req-pr-er-gate-maintenance",
        source: "pr_er_gate_account_state",
      },
    },
  ];
}

function cancelableExecutionRun() {
  return {
    runId: "e2e-run-cancel-denied",
    ticketId: "ticket-mu-001",
    opportunityId: "mock-mu-perp",
    state: "second_leg_submitted",
    longLeg: executionRunLeg("long", "hyperliquid:km", "e2e-run-order-1"),
    shortLeg: executionRunLeg("short", "kucoin", "e2e-run-order-2"),
    netExposureUsd: 0,
    recoveryAction: "cancel_open_orders",
    statusReason: "venue acknowledgements pending finality",
    createdAtMs: now - 2_000,
    updatedAtMs: now,
  };
}

function executionRunLeg(role, exchange, orderId) {
  return {
    role,
    exchange,
    symbol: "MU",
    orderIds: [orderId],
    state: "accepted",
    targetQuantity: 1.13,
    filledQuantity: null,
    targetNotionalUsd: 750,
    filledNotionalUsd: null,
    filledFee: null,
  };
}

function positionCloseDeniedSnapshot() {
  const snapshot = portfolioSnapshot();
  const position = mutationPosition();
  const accountPosition = mutationAccountPosition();
  return {
    ...snapshot,
    snapshotVersion: "positions-mutation-v1",
    degraded: false,
    problems: [],
    positions: [position],
    accountState: {
      ...snapshot.accountState,
      positions: {
        ...snapshot.accountState.positions,
        rows: [accountPosition],
        rowCount: 1,
        status: "fresh",
        problems: [],
        operationHealth: [positionOperationHealth(1)],
      },
      status: "fresh",
      problems: [],
      operationHealth: [
        balanceOperationHealth(snapshot.balances.length),
        positionOperationHealth(1),
      ],
    },
  };
}

function mutationAccountPosition() {
  return {
    symbol: "MU",
    exchange: "mock",
    side: "long",
    quantity: 1.13,
    entryPrice: 660,
    markPrice: 664.63,
    unrealizedPnl: 5.23,
    leverage: 1,
    liquidationPrice: 330,
    liquidationDistancePct: 50.35,
    nextFundingMs: now + 60 * 60 * 1000,
    pairedWith: null,
    margin: 750,
    maintenanceMarginRatio: 0.05,
  };
}

function mutationPosition() {
  return {
    venue: "mock",
    symbol: "MU",
    side: "long",
    quantity: 1.13,
    entryPrice: 660,
    markPrice: 664.63,
    leverage: 1,
    unrealizedPnlUsd: 5.23,
    liquidationPrice: 330,
    liquidationDistancePct: 50.35,
    nextFundingMs: now + 60 * 60 * 1000,
    fundingRate8h: -0.0008,
    fundingRateVerified: true,
    maintenanceMarginRatio: 0.05,
    pairedWith: null,
    marginUsd: 750,
    severity: "ok",
    secondsUntilFunding: 3600,
  };
}

function positionCancelDeniedSnapshot() {
  const snapshot = portfolioSnapshot();
  return {
    ...snapshot,
    snapshotVersion: "positions-cancel-v1",
    recentCloseRuns: [cancelableCompensationCloseRun()],
  };
}

function cancelableCompensationCloseRun() {
  return {
    id: "e2e-close-run-cancel-denied",
    scope: "pair",
    status: "compensation_submitted",
    actionRunId: "e2e-close-action-1",
    requestId: "req-close-run-original",
    idempotencyKey: "close-run-original-key",
    snapshotVersion: "positions-cancel-v1",
    expectedLegCount: 2,
    reason: "positions.close_pair",
    legs: [],
    submittedOrderCount: 1,
    failedLegCount: 1,
    nakedExposureUsd: 12,
    message: "compensation accepted; waiting for finality",
    problem: null,
    finalityProblem: null,
    finalityCheckedAtMs: now - 1_000,
    unwindPlan: {
      status: "compensation_submitted",
      filledLegs: [],
      failedLegs: [],
      compensationCandidates: [],
      remainingPositions: [],
      compensationAttempts: [compensationAttempt()],
      nextActions: [{
        kind: "cancel_compensation_order",
        label: "撤销补偿单",
        candidateIndex: 0,
        requiresConfirmation: false,
        requiredEvidence: ["accepted_compensation_order"],
        reason: "operator requested cancel before finality",
      }],
      requiredEvidence: ["order_finality"],
    },
    costEvents: [],
    startedAtMs: now - 5_000,
    updatedAtMs: now,
  };
}

function compensationAttempt() {
  return {
    actionRunId: "e2e-comp-action-1",
    venue: "mock",
    symbol: "MU",
    side: "long",
    compensationOrderSide: "buy",
    targetQuantity: 1.13,
    status: "accepted",
    order: compensationOrderRecord(),
    submittedAtMs: now - 2_000,
    updatedAtMs: now - 1_000,
  };
}

function compensationOrderRecord() {
  return {
    intent: {
      ...orderIntent("e2e-comp-order-1", "mock", "MU", "buy"),
      source: "close_run_compensation",
      orderType: "market",
      reduceOnly: true,
    },
    state: "accepted",
    risk: null,
    exchangeOrderId: "e2e-comp-exchange-1",
    message: "accepted; finality pending",
    updatedAtMs: now - 1_000,
  };
}

function portfolioSnapshotFailure(response) {
  return json(
    response,
    apiProblem("PORTFOLIO_SNAPSHOT_UNAVAILABLE", "mock portfolio snapshot unavailable", 502, {
      requestId: "e2e-portfolio-snapshot-502",
      retryAfterMs: 6000,
      source: "e2e-fixture",
      details: {
        route: "/api/trading/portfolio/snapshot",
        scenario: "portfolio-snapshot-502",
      },
    }),
    502,
    { "retry-after": "6", "x-request-id": "e2e-portfolio-snapshot-502" },
  );
}

function positionRuntimeProblem() {
  return {
    scope: "portfolio",
    operation: "positions",
    code: "VENUE_POSITIONS_READ_FAILED",
    message: "Gate positions read failed; other venues retained",
    venue: "gate",
    retryAfterMs: 5_000,
    observedAtMs: now,
  };
}

function positionProbeProblem() {
  return {
    code: "VENUE_POSITIONS_READ_FAILED",
    message: "Gate positions read failed; other venues retained",
    status: 502,
    requestId: "req-gate-positions",
    retryAfterMs: 5_000,
    source: "account_position_runtime",
    details: {
      venue: "gate",
      operation: "positions",
    },
  };
}

function listEnvelope(rows) {
  return {
    rows,
    page: listPage(rows.length),
    status: "fresh",
    source: "e2e-fixture",
    observedAtMs: now,
    problems: [],
  };
}

function listPage(rowCount, totalRows = rowCount, startOffset = 0, limit = rowCount) {
  return {
    limit,
    maxLimit: 100,
    startOffset,
    returnedCount: rowCount,
    totalRows,
    hasMore: startOffset + rowCount < totalRows,
    nextCursor: startOffset + rowCount < totalRows ? String(startOffset + rowCount) : null,
  };
}

function reviewEnvelope(rows, source) {
  return {
    rows,
    generatedAtMs: now,
    days: 7,
    source,
    rowCount: rows.length,
    page: listPage(rows.length),
    status: "fresh",
    ledgerStatus: null,
    missingFields: [],
    problems: [],
  };
}

function largeReviewEnvelope(url, source, rowBuilder, totalRows) {
  const limit = boundedInt(url.searchParams.get("limit"), 50, 1, 100);
  const start = boundedInt(url.searchParams.get("cursor"), 0, 0, totalRows - 1);
  const end = Math.min(start + limit, totalRows);
  const rows = [];
  for (let index = start; index < end; index += 1) {
    rows.push(rowBuilder(index));
  }
  return {
    ...reviewEnvelope(rows, source),
    rowCount: totalRows,
    page: listPage(rows.length, totalRows, start, limit),
  };
}

function largeExecutedTrade(index) {
  return {
    id: `exec-${index}`,
    strategy: "perp_cross",
    symbol: `MU${String(index + 1).padStart(3, "0")}`,
    longVenue: "hyperliquid:km",
    shortVenue: "kucoin",
    openedAtMs: now - (index + 1) * 60_000,
    closedAtMs: now - index * 30_000,
    holdingMinutes: 480,
    grossPnlUsd: 12.5 + index * 0.01,
    feeUsd: 0.21,
    fundingUsd: 0.7,
    slippageUsd: 0.12,
    netPnlUsd: 12.87 + index * 0.01,
    evidence: {},
    actualFields: ["gross", "fee", "funding", "slippage", "net"],
    estimatedFields: [],
    missingFields: [],
    longOrders: [],
    shortOrders: [],
  };
}

function staleExecutedTrade() {
  return {
    ...largeExecutedTrade(0),
    id: "exec-stale-retry-1",
    symbol: "MU-STL",
    netPnlUsd: 13.37,
  };
}

function largeMissedOpportunity(index) {
  return {
    id: `missed-${index}`,
    opportunityId: `mock-mu-perp-${index}`,
    strategy: "perp_cross",
    symbol: `MU${String(index + 1).padStart(3, "0")}`,
    detectedAtMs: now - (index + 1) * 90_000,
    expectedPnlUsd: 2.5 + index * 0.01,
    reason: "latency_exceeded",
    detail: `e2e large missed row ${index}`,
  };
}

function balanceEnvelope(rows) {
  return {
    rows,
    rowCount: rows.length,
    status: "fresh",
    source: "account_balance_runtime",
    observedAtMs: now,
    problems: [],
    operationHealth: [balanceOperationHealth(rows.length)],
  };
}

function positionEnvelope(rows) {
  return {
    rows,
    rowCount: rows.length,
    status: "fresh",
    source: "account_position_runtime",
    observedAtMs: now,
    problems: [],
    operationHealth: [positionOperationHealth(rows.length)],
  };
}

function balanceOperationHealth(rowCount) {
  return {
    venue: "mock",
    operation: "balance",
    status: "ok",
    source: "account_cache",
    message: "账户缓存有新鲜运行态样本",
    supported: true,
    configured: true,
    rows: rowCount,
    freshnessMs: 0,
    observedAtMs: now,
  };
}

function positionOperationHealth(rowCount) {
  return {
    venue: "mock",
    operation: "positions",
    status: "ok",
    source: "account_cache",
    message: "持仓缓存有新鲜运行态样本",
    supported: true,
    configured: true,
    rows: rowCount,
    freshnessMs: 0,
    observedAtMs: now,
  };
}

function failedPositionOperationHealth() {
  return {
    venue: "gate",
    operation: "positions",
    status: "warn",
    source: "account_cache",
    message: "Gate positions read failed; other venues retained",
    supported: true,
    configured: true,
    rows: 0,
    freshnessMs: 30_000,
    retryAfterMs: 5_000,
    error: "Gate positions read failed; other venues retained",
    problem: {
      code: "VENUE_POSITIONS_READ_FAILED",
      message: "Gate positions read failed; other venues retained",
      status: 502,
      requestId: "req-gate-positions",
      retryAfterMs: 5_000,
      source: "portfolio",
      details: {
        venue: "gate",
        operation: "positions",
      },
    },
    observedAtMs: now,
  };
}

function historyFundingDiffs() {
  return {
    count: 1,
    rows: [
      {
        occurredAtMs: now,
        symbol: "MU",
        longExchange: "hyperliquid:km",
        shortExchange: "kucoin",
        longRate8h: -0.0008,
        shortRate8h: 0.00013129,
        grossDiffBps: 9.3129,
        longNextFundingMs: now + 60 * 60 * 1000,
        shortNextFundingMs: now + 60 * 60 * 1000,
        windowAlignmentMinutes: 0,
        longIntervalHours: 8,
        shortIntervalHours: 8,
        minVolume24h: 1_000_000,
      },
    ],
    page: {
      limit: 100,
      maxLimit: 1000,
      returnedCount: 1,
      hasMore: false,
      nextCursor: null,
    },
    source: "e2e-fixture",
    observedAtMs: now,
    latestAtMs: now,
    freshnessMs: 0,
    problem: null,
    retryAfterMs: null,
    problems: [],
  };
}

function systemHealth() {
  return {
    api: { healthy: 8, total: 8, failedVenues: [] },
    ws: { channels: 3, disconnected: [] },
    orderElapsedMs: 24,
    risk: "ok",
    netDeltaUsd: 0,
    netDeltaPctOfNav: 0,
    nextFunding: {
      symbol: "MU",
      venue: "hyperliquid:km",
      minutesToSettle: 60,
      estimatedOutflowUsd: -0.7,
    },
    updatedAtMs: now,
  };
}

function systemHealthEnvelope() {
  return {
    data: systemHealth(),
    status: "ready",
    source: "system-health-snapshot",
    observedAtMs: now,
    problems: [],
  };
}

function actionRunsEnvelope() {
  return {
    data: [
      {
        id: "e2e-action-run-1",
        kind: "trading_order_submit",
        status: "succeeded",
        actor: "e2e-operator",
        target: "mock/MU",
        requestId: "req-pr-ew-action-run",
        idempotencyKey: "idem-pr-ew-action-run",
        message: "order submission completed",
        startedAtMs: now - 1_000,
        updatedAtMs: now,
      },
    ],
    status: "ready",
    source: "action-run-registry",
    observedAtMs: now,
    coverage: {
      expected: 1,
      observed: 1,
      coveragePct: 1,
      truncated: false,
    },
    problems: [],
  };
}

function watchlistRuntimeContract() {
  return {
    featureGate: "api_surface.watchlist_alerts",
    persistence: "memory",
    volatile: true,
    restartBehavior: "cleared_on_restart",
    publicOrderbookPrewarmLimit: 1,
    publicTickerSymbolsPerVenueLimit: 1,
    privateWsSymbolsFromWatchlist: 0,
  };
}

function emptyWatchlistEnvelope() {
  return { items: [], runtime: watchlistRuntimeContract() };
}

function emptyAlertRulesEnvelope() {
  return { rules: [], runtime: watchlistRuntimeContract() };
}

function watchlistRuntimeEnvelope() {
  return {
    items: [
      {
        id: 41,
        symbol: "BTC-USDT",
        venueLong: "binance",
        venueShort: "okx",
        minNetYield: 0.08,
        minVolume24h: 1_000_000,
        enabled: true,
        createdAtMs: now - 60_000,
        runtime: {
          status: "capped",
          requestedPublicLegs: 2,
          plannedOrderbookLegs: 1,
          plannedTickerLegs: 1,
          deduplicatedLegs: 0,
          cappedLegs: 1,
          lastPrewarmAtMs: now,
          problem: {
            code: "WATCHLIST_PREWARM_CAPPED",
            message: "Watchlist public prewarm cap retained one of two requested venue legs",
            status: 200,
            requestId: "e2e-watchlist-prewarm-capped",
            source: "market_data_runtime",
            details: {
              requestedPublicLegs: 2,
              plannedOrderbookLegs: 1,
              cappedLegs: 1,
            },
          },
        },
      },
    ],
    runtime: watchlistRuntimeContract(),
  };
}

function alertRulesRuntimeEnvelope() {
  return {
    rules: [
      {
        id: 101,
        watchlistId: 41,
        channel: { kind: "toast" },
        cooldownSecs: 300,
        enabled: true,
        createdAtMs: now - 50_000,
        runtime: {
          status: "queued",
          transport: "app_websocket_toast",
          deliverySupported: true,
          watchlistPublicPrewarmLegs: 2,
          privateWsSymbols: 0,
          lastEvaluatedAtMs: now,
          lastTriggeredAtMs: now,
          nextEligibleAtMs: now + 300_000,
          triggerCount: 3,
          lastOpportunityId: "opp-btc-runtime",
        },
      },
      {
        id: 102,
        watchlistId: 41,
        channel: { kind: "toast" },
        cooldownSecs: 300,
        enabled: true,
        createdAtMs: now - 40_000,
        runtime: {
          status: "blocked",
          transport: "app_websocket_toast",
          deliverySupported: true,
          watchlistPublicPrewarmLegs: 2,
          privateWsSymbols: 0,
          lastEvaluatedAtMs: now,
          triggerCount: 0,
          problem: {
            code: "ALERT_TOAST_NOT_QUEUED",
            message: "No subscribed app client accepted the toast notification",
            status: 409,
            requestId: "e2e-alert-toast-not-queued",
            source: "alert_runtime",
            details: { channel: "alerts", subscriberCount: 0 },
          },
        },
      },
    ],
    runtime: watchlistRuntimeContract(),
  };
}

function alertRuntimeNotification() {
  return {
    id: "alert-notification-runtime-1",
    ruleId: 101,
    watchlistId: 41,
    opportunityId: "opp-btc-runtime",
    symbol: "BTC-USDT",
    strategy: "perp_cross",
    longExchange: "binance",
    shortExchange: "okx",
    finalScore: 91.25,
    netSingleYield: 0.1234,
    queuedAtMs: now,
  };
}

function venueOperationHealth(scenario) {
  const snapshot = portfolioSnapshot();
  if (scenario === "pr-er-gate-runtime") {
    const rows = prErGateRuntimeHealthRows();
    return {
      rows,
      generatedAtMs: now,
      rowCount: rows.length,
      attentionCount: rows.filter((row) => row.status !== "ok").length,
    };
  }
  if (scenario === "pr-bx-runtime") {
    const rows = prBxRuntimeHealthRows();
    return {
      rows,
      generatedAtMs: now,
      rowCount: rows.length,
      attentionCount: rows.filter((row) => row.status !== "ok").length,
    };
  }
  if (scenario === "settings-credential-static-adapter-boundary") {
    const rows = [
      settingsCredentialOrderPermissionUnknown(),
      balanceOperationHealth(snapshot.balances.length),
      positionOperationHealth(snapshot.positions.length),
    ];
    return {
      rows,
      generatedAtMs: now,
      rowCount: rows.length,
      attentionCount: rows.filter((row) => row.status !== "ok").length,
    };
  }
  if (scenario === "settings-private-order-stream-ok-capture-readiness") {
    const rows = [
      settingsCredentialOrderPermissionUnknown(),
      settingsPrivateOrderStreamOk(),
      balanceOperationHealth(snapshot.balances.length),
      positionOperationHealth(snapshot.positions.length),
    ];
    return {
      rows,
      generatedAtMs: now,
      rowCount: rows.length,
      attentionCount: rows.filter((row) => row.status !== "ok").length,
    };
  }
  if (scenario === "settings-selected-venue-trading-runtime-all-ok-local-capture-shaped") {
    const rows = [
      settingsCredentialOrderPermissionOkLocalCapture(),
      settingsOrderWriteOkLocalCapture(),
      settingsPrivateOrderStreamOkLocalCapture(),
      settingsOrderFinalityOkLocalCapture(),
      balanceOperationHealth(snapshot.balances.length),
      positionOperationHealth(snapshot.positions.length),
    ];
    return {
      rows,
      generatedAtMs: now,
      rowCount: rows.length,
      attentionCount: rows.filter((row) => row.status !== "ok").length,
    };
  }
  const rows = [
    credentialValidationFailure(),
    ...(scenario === "api-transport" ? [apiTransportWarning()] : []),
    ...(scenario === "api-transport-fallback" ? [apiTransportFallbackWarning()] : []),
    ...(scenario === "private-order-stream-warning" ? [] : [privateWsAuthFailure()]),
    privateWsOrderStreamWarning(),
    orderFinalityWarning(),
    balanceOperationHealth(snapshot.balances.length),
    positionOperationHealth(snapshot.positions.length),
  ];
  return {
    rows,
    generatedAtMs: now,
    rowCount: rows.length,
    attentionCount: rows.filter((row) => row.status !== "ok").length,
  };
}

function venueRuntimeHealth(scenario) {
  const rows = venueOperationHealth(scenario).rows;
  const byVenue = new Map();
  for (const row of rows) {
    const venueName = String(row.venue ?? "").trim().toLowerCase();
    if (!venueName) continue;
    const venue = byVenue.get(venueName) ?? { venue: venueName, generatedAtMs: now };
    for (const [slot, operation] of runtimeSlots(row.operation)) {
      const candidate = runtimeOperationHealth(row, operation);
      if (!venue[slot] || runtimeCandidateIsWorse(candidate, venue[slot])) {
        venue[slot] = candidate;
      }
    }
    byVenue.set(venueName, venue);
  }

  const venues = [...byVenue.values()].sort((left, right) => left.venue.localeCompare(right.venue));
  const operations = venues.flatMap((venue) => runtimeSlotNames.flatMap((slot) => venue[slot] ?? []));
  const currentlyUsableCount = operations.filter((operation) => operation.currentlyUsable).length;
  return {
    venues,
    generatedAtMs: now,
    venueCount: venues.length,
    operationCount: operations.length,
    currentlyUsableCount,
    attentionCount: operations.length - currentlyUsableCount,
  };
}

const runtimeSlotNames = [
  "publicRest",
  "publicWs",
  "privateRest",
  "privateWs",
  "balance",
  "positions",
  "openOrders",
  "placeOrder",
  "cancelOrder",
  "orderStream",
  "finality",
];

function runtimeSlots(operation) {
  const key = String(operation ?? "").trim().toLowerCase();
  if (
    [
      "rest_orderbooks",
      "rest_funding_rates",
      "rest_index_compositions",
      "rest_instrument_specs",
      "rest_metadata",
      "rest_perp_tickers",
      "rest_spot_ticks",
      "rest_funding_fallback",
      "rest_ticker_fallback",
    ].includes(key)
  ) return [["publicRest", "public_rest"]];
  if (
    [
      "ws_funding",
      "ws_funding_subscribe",
      "ws_funding_snapshot",
      "ws_ticker",
      "ws_ticker_subscribe",
      "ws_ticker_snapshot",
    ].includes(key)
  ) return [["publicWs", "public_ws"]];
  if (["private_read", "credential_probe:account_mode_read"].includes(key)) {
    return [["privateRest", "private_rest"]];
  }
  if (["private_ws_session", "private_ws_subscribe", "private_ws_account_stream"].includes(key)) {
    return [["privateWs", "private_ws"]];
  }
  if (["balance", "credential_probe:balance_read"].includes(key)) {
    return [["balance", "balance"]];
  }
  if (["positions", "credential_probe:positions_read"].includes(key)) {
    return [["positions", "positions"]];
  }
  if (key === "credential_probe:open_orders_read") return [["openOrders", "open_orders"]];
  if (key === "order_write") {
    return [["placeOrder", "place_order"], ["cancelOrder", "cancel_order"]];
  }
  if (key === "private_ws_order_stream") return [["orderStream", "order_stream"]];
  if (["order_finality", "order_reconciliation"].includes(key)) {
    return [["finality", "finality"]];
  }
  return [];
}

function runtimeOperationHealth(row, operation) {
  const capabilityStatus = row.supported === true
    ? "supported"
    : row.supported === false
      ? "unsupported"
      : "unknown";
  const publicOperation = operation === "public_rest" || operation === "public_ws";
  const configurationStatus = row.configured === true
    ? "configured"
    : row.configured === false
      ? "not_configured"
      : publicOperation
        ? "not_required"
        : "unknown";
  const retryAfterMs = Math.max(row.retryAfterMs ?? 0, row.problem?.retryAfterMs ?? 0) || undefined;
  return {
    operation,
    status: row.status,
    source: row.source,
    freshnessMs: row.freshnessMs,
    latencyMs: row.latencyMs,
    latencyP95Ms: row.latencyP95Ms,
    requested: row.requested,
    rows: row.rows,
    lastSuccessMs: row.status === "ok" ? row.observedAtMs : undefined,
    lastError: row.error ?? row.problem?.message,
    retryAfterMs,
    requestId: row.evidence?.requestId ?? row.problem?.requestId,
    problem: row.problem,
    capabilityStatus,
    configurationStatus,
    currentlyUsable: row.status === "ok"
      && capabilityStatus === "supported"
      && ["configured", "not_required"].includes(configurationStatus),
    observedAtMs: row.observedAtMs,
  };
}

function runtimeCandidateIsWorse(candidate, current) {
  const severity = { ok: 0, warn: 1, unknown: 2, unsupported: 3, blocked: 4 };
  const candidateSeverity = severity[candidate.status] ?? 2;
  const currentSeverity = severity[current.status] ?? 2;
  return candidateSeverity > currentSeverity
    || (candidateSeverity === currentSeverity && candidate.observedAtMs > current.observedAtMs);
}

function prErGateRuntimeHealthRows() {
  return [
    prErGateOperation({
      operation: "rest_metadata",
      status: "ok",
      source: "gate_contract_metadata_runtime",
      message: "Gate native USDT contract metadata is fixture-backed public evidence",
      requestId: "req-pr-er-gate-native-contract",
      method: "GET",
      path: "/futures/{settle}/contracts/{contract}",
      authKind: "public",
      fixtureId: "gate/futures_usdt_contracts_btc_usdt.json",
      parserTest: "official_fixture_closes_native_identity_and_contract_spec",
      requestBuilderTest: "contract_list_request_binds_the_verified_settle_path",
      context: [
        "settle=usdt",
        "native_symbol=BTC_USDT",
        "contract_size=0.0001",
        "quanto_multiplier=0.0001",
        "order_size_unit=contract",
        "public_metadata_only=true",
        "credentials_present=false",
        "not_real_exchange_live_sample=true",
      ],
      useCases: ["native_contract_identity", "instrument_spec_cache"],
      dataKinds: ["instrument_spec", "contract_metadata"],
    }),
    prErGateOperation({
      operation: "order_finality",
      status: "unknown",
      source: "gate_order_finality_runtime",
      message: "Gate order finality is unproven without a live order query sample",
      requestId: "req-pr-er-gate-finality",
      method: "GET",
      path: "/futures/{settle}/orders/{order_id}",
      authKind: "signed",
      fixtureId: "gate/futures_usdt_get_order_ioc.json",
      parserTest: "gate_get_order_ioc_fixture_preserves_partial_fill_as_terminal_cancel",
      requestBuilderTest: "order_row_parses_official_success_envelope",
      context: [
        "terminal_mapping=finished|cancelled",
        "live_order_query_sample=missing",
        "credentials_present=false",
        "currently_usable=false",
        "not_real_exchange_live_sample=true",
      ],
      useCases: ["order_finality", "execution_run_finality"],
      dataKinds: ["order_state"],
    }),
    prErGateOperation({
      operation: "private_ws_order_stream",
      status: "blocked",
      source: "gate_private_ws_runtime",
      message: "Gate private order stream is blocked because no live credentials are configured",
      requestId: "req-pr-er-gate-private-ws",
      method: "WS",
      path: "wss://fx-ws.gateio.ws/v4/ws/usdt#futures.orders",
      authKind: "signed_subscription",
      fixtureId: "inline:gate_ws_user_tests",
      parserTest: "ws_order_update_accepts_known_open_gtc",
      requestBuilderTest: "subscribe_payload_matches_gate_authenticated_schema",
      context: [
        "channel=futures.orders",
        "user_id=not_recorded",
        "credentials_present=false",
        "currently_usable=false",
        "not_real_exchange_live_sample=true",
      ],
      useCases: ["private_ws_runtime", "order_stream"],
      dataKinds: ["order_state_stream"],
    }),
    prErGateOperation({
      operation: "balance",
      status: "unknown",
      source: "pr_er_gate_account_state",
      message: "Gate fee evidence is missing; account PnL does not assume zero fees",
      requestId: "req-pr-er-gate-fee",
      method: "GET",
      path: "/futures/{settle}/my_trades",
      authKind: "signed",
      fixtureId: "gate/futures_usdt_my_trades_order.json",
      parserTest: "parses_official_my_trades_fixture_without_combining_fee_units",
      requestBuilderTest: "my_trades_uses_signed_order_query_and_official_fixture",
      context: [
        "fee_evidence=missing",
        "zero_fee_assumption=false",
        "credentials_present=false",
        "currently_usable=false",
        "not_real_exchange_live_sample=true",
      ],
      useCases: ["account_state", "fill_fee_evidence"],
      dataKinds: ["fill_fee"],
    }),
    prErGateOperation({
      operation: "positions",
      status: "unknown",
      source: "pr_er_gate_account_state",
      message: "Gate maintenance ratio evidence is missing; margin utilization is estimated",
      requestId: "req-pr-er-gate-maintenance",
      method: "GET",
      path: "/futures/{settle}/positions",
      authKind: "signed",
      fixtureId: "gate/futures_usdt_positions.json",
      parserTest: "parse_positions_maps_official_maintenance_rate",
      requestBuilderTest: "private_reads_reject_unsigned_query_or_path_drift_before_io",
      context: [
        "maintenance_evidence=missing",
        "margin_utilization_estimated=true",
        "credentials_present=false",
        "currently_usable=false",
        "not_real_exchange_live_sample=true",
      ],
      useCases: ["account_state", "maintenance_margin_evidence"],
      dataKinds: ["position_risk"],
    }),
  ];
}

function prErGateOperation({
  operation,
  status,
  source,
  message,
  requestId,
  method,
  path,
  authKind,
  fixtureId,
  parserTest,
  requestBuilderTest,
  context,
  useCases,
  dataKinds,
}) {
  const usable = status === "ok";
  const configured = operation === "rest_metadata";
  const problem = usable
    ? null
    : {
        code: `GATE_${operation.toUpperCase()}_EVIDENCE_UNAVAILABLE`,
        message,
        status: status === "blocked" ? 503 : 409,
        requestId,
        retryAfterMs: 60_000,
        source,
        details: { venue: "gate", operation, liveCredentialsUsed: false },
      };
  return {
    venue: "gate",
    operation,
    status,
    source,
    message,
    supported: true,
    configured,
    requested: 1,
    rows: usable ? 1 : 0,
    freshnessMs: usable ? 500 : null,
    retryAfterMs: usable ? null : 60_000,
    error: usable ? null : message,
    evidence: {
      method,
      path,
      checkedAt: "2026-07-11",
      docVersion: "gate-futures-api-v4-pr-er",
      schemaHash: "sha256:pr-er-gate-contract",
      fixtureId,
      parserTest,
      requestBuilderTest,
      authKind,
      requestId,
      requestContext: context,
      docUrls: ["https://www.gate.com/docs/developers/apiv4/en/"],
      useCases,
      dataKinds,
      rateScopes: configured ? ["public"] : ["user"],
      weight: 1,
    },
    problem,
    observedAtMs: now,
  };
}

function prBxRuntimeHealthRows() {
  return [
    prBxOperation({
      venue: "binance",
      operation: "rest_orderbooks",
      status: "ok",
      source: "pr_bx_market_data_runtime",
      message: "PR-BX market-data source is fresh",
      requestId: "req-pr-bx-market-data",
    }),
    prBxOperation({
      venue: "okx",
      operation: "order_write",
      status: "warn",
      source: "pr_bx_trading_runtime",
      message: "PR-BX configured trading API is degraded",
      requestId: "req-pr-bx-trading-warn",
      retryAfterMs: 23_000,
      problemRetryAfterMs: 17_000,
    }),
    prBxOperation({
      venue: "gate",
      operation: "balance",
      status: "ok",
      source: "pr_bx_trading_runtime",
      message: "PR-BX trading API balance source is usable",
      requestId: "req-pr-bx-trading-ok",
    }),
    prBxOperation({
      venue: "bybit",
      operation: "private_ws_order_stream",
      status: "blocked",
      source: "pr_bx_private_ws_runtime",
      message: "PR-BX configured private WS is blocked",
      requestId: "req-pr-bx-private-ws-blocked",
      retryAfterMs: 31_000,
      problemRetryAfterMs: 29_000,
    }),
    prBxOperation({
      venue: "binance",
      operation: "private_ws_session",
      status: "ok",
      source: "pr_bx_private_ws_runtime",
      message: "PR-BX private WS session source is usable",
      requestId: "req-pr-bx-private-ws-ok",
    }),
  ];
}

function prBxOperation({
  venue,
  operation,
  status,
  source,
  message,
  requestId,
  retryAfterMs = null,
  problemRetryAfterMs = null,
}) {
  const problem = status === "ok"
    ? null
    : {
        code: "PR_BX_RUNTIME_NOT_USABLE",
        message,
        status: status === "blocked" ? 503 : 429,
        requestId,
        retryAfterMs: problemRetryAfterMs,
        source,
        details: { venue, operation, configured: true, supported: true },
      };
  return {
    venue,
    operation,
    status,
    source,
    message,
    supported: true,
    configured: true,
    requested: 1,
    rows: status === "ok" ? 1 : 0,
    freshnessMs: 900,
    retryAfterMs,
    error: status === "ok" ? null : message,
    evidence: {
      method: operation.startsWith("private_ws_") ? "WS" : "GET",
      path: `pr-bx://${venue}/${operation}`,
      checkedAt: "2026-07-11",
      docVersion: "pr-bx-browser-contract",
      schemaHash: "sha256:pr-bx-runtime-contract",
      fixtureId: "pr-bx-runtime",
      parserTest: "pr_bx_runtime_browser_contract",
      requestBuilderTest: "pr_bx_runtime_browser_contract",
      authKind: "e2e_fixture",
      requestId,
      requestContext: [
        `runtime_source=${source}`,
        `configured=true`,
        `capability_supported=true`,
        `currently_usable=${status === "ok"}`,
      ],
      docUrls: [],
      useCases: ["pr_bx_runtime_health"],
      dataKinds: [operation],
      rateScopes: [],
      weight: 0,
    },
    problem,
    observedAtMs: now,
  };
}

function isSettingsCredentialRuntimeScenario(scenario) {
  return scenario === "settings-credential-static-adapter-boundary"
    || scenario === "settings-private-order-stream-ok-capture-readiness"
    || scenario === "settings-selected-venue-trading-runtime-all-ok-local-capture-shaped";
}

function settingsCredentialOrderPermissionOkLocalCapture() {
  const message = "本地 UI/CI gate：下单/撤单权限 capture-shaped Ok，非真实交易所 live 样本";
  return {
    venue: "okx",
    operation: "credential_probe:order_permission",
    status: "ok",
    source: "credential_validation",
    message,
    supported: true,
    configured: true,
    requested: 1,
    rows: 1,
    freshnessMs: 700,
    retryAfterMs: null,
    error: null,
    evidence: {
      method: "POST",
      path: "/api/v5/trade/order-precheck",
      checkedAt: "2026-07-02",
      docVersion: "okx-v5-trade-order-precheck-2026-07-02",
      schemaHash: "not_recorded",
      fixtureId: "not_recorded",
      parserTest: "not_recorded",
      requestBuilderTest: "order_precheck_request_uses_official_shape",
      authKind: "signed",
      requestId: "req-settings-runtime-all-ok-order-permission",
      requestContext: [
        "kind=order_permission",
        "status=ok",
        "credentialsAvailable=true",
        "liveWrite=true",
        "local_ui_ci_gate=true",
        "capture_shape=selected_venue_trading_runtime_all_ok",
        "not_real_exchange_live_sample=true",
      ],
      docUrls: ["https://www.okx.com/docs-v5/en/"],
      useCases: ["credential_validation", "order_permission_boundary"],
      dataKinds: ["order_permission"],
      rateScopes: ["user"],
      weight: 1,
    },
    problem: null,
    observedAtMs: now,
  };
}

function settingsCredentialOrderPermissionUnknown() {
  const message = "safe/noop probe 未授予 live_write；仍需 live place/cancel/order stream 证据";
  return {
    venue: "okx",
    operation: "credential_probe:order_permission",
    status: "unknown",
    source: "credential_validation",
    message,
    supported: true,
    configured: true,
    requested: 1,
    rows: 0,
    freshnessMs: 3_000,
    retryAfterMs: null,
    error: message,
    evidence: {
      method: "POST",
      path: "/api/v5/trade/order-precheck",
      checkedAt: "2026-07-02",
      docVersion: "okx-v5-trade-order-precheck-2026-07-02",
      schemaHash: "not_recorded",
      fixtureId: "not_recorded",
      parserTest: "not_recorded",
      requestBuilderTest: "order_precheck_request_uses_official_shape",
      authKind: "signed",
      requestId: "req-settings-order-permission-unknown",
      requestContext: [
        "kind=order_permission",
        "status=unknown",
        "credentialsAvailable=true",
        "liveWrite=true",
        "does_not_grant_live_write=true",
        "probe_scope=place_cancel_order_stream",
        "probe_source=safe_noop_probe",
      ],
      docUrls: ["https://www.okx.com/docs-v5/en/"],
      useCases: ["credential_validation", "order_permission_boundary"],
      dataKinds: ["order_permission"],
      rateScopes: ["user"],
      weight: 1,
    },
    problem: {
      code: "ORDER_PERMISSION_UNPROVEN",
      message,
      status: 409,
      requestId: "req-settings-order-permission-unknown",
      source: "credential_validation",
      details: {
        venue: "okx",
        operation: "credential_probe:order_permission",
        kind: "order_permission",
        status: "unknown",
        credentialsAvailable: true,
        liveWrite: true,
      },
    },
    observedAtMs: now,
  };
}

function settingsOrderWriteOkLocalCapture() {
  const message = "本地 UI/CI gate：live place/cancel runtime proof 已成形，非真实交易所 live 样本";
  return {
    venue: "okx",
    operation: "order_write",
    status: "ok",
    source: "live_order_proof_runtime",
    message,
    supported: true,
    configured: true,
    requested: 2,
    rows: 2,
    freshnessMs: 600,
    retryAfterMs: null,
    error: null,
    evidence: {
      method: "internal",
      path: "live_order_proof.runtime",
      checkedAt: "not_recorded",
      docVersion: "not_recorded",
      schemaHash: "not_recorded",
      fixtureId: "not_recorded",
      parserTest: "not_recorded",
      requestBuilderTest: "not_recorded",
      authKind: "live_order_remote_proof",
      requestId: "req-settings-runtime-all-ok-order-write",
      requestContext: [
        "live_place_remote_proof=ok",
        "live_cancel_remote_proof=ok",
        "place_ack_count=1",
        "cancel_requested_count=1",
        "cancel_finality_count=1",
        "sample_place_request_id=req-settings-runtime-all-ok-place",
        "sample_cancel_request_id=req-settings-runtime-all-ok-cancel",
        "local_ui_ci_gate=true",
        "capture_shape=selected_venue_trading_runtime_all_ok",
        "not_real_exchange_live_sample=true",
      ],
      docUrls: [],
      useCases: ["order_write", "live_place_cancel_remote_proof"],
      dataKinds: ["order_ack", "cancel_request_ack", "cancel_finality"],
      rateScopes: [],
      weight: 0,
    },
    problem: null,
    observedAtMs: now,
  };
}

function settingsPrivateOrderStreamOk() {
  const message = "私有订单流已捕获 3 行官方 order_state_stream 样本";
  return {
    venue: "okx",
    operation: "private_ws_order_stream",
    status: "ok",
    source: "private_ws_runtime",
    message,
    supported: true,
    configured: true,
    requested: 3,
    rows: 3,
    freshnessMs: 800,
    retryAfterMs: null,
    error: null,
    evidence: {
      method: "WS",
      path: "wss://ws.okx.com:8443/ws/v5/private#orders",
      checkedAt: "2026-07-02",
      docVersion: "okx-v5-private-ws-orders-2026-07-02",
      schemaHash: "sha256:e2e-private-order-stream-ok",
      fixtureId: "e2e-private-order-stream-ok",
      parserTest: "okx_orders_channel_parses_order_state_stream",
      requestBuilderTest: "okx_private_ws_subscribe_orders_channel",
      authKind: "signed_subscription",
      requestId: "req-settings-private-order-stream-ok",
      requestContext: [
        "runtime_operation=private_ws_order_stream",
        "ws_operation=orders",
        "sample_rows=3",
        "official_evidence=order_state_stream",
        "source=private_ws_runtime",
      ],
      docUrls: ["https://www.okx.com/docs-v5/en/"],
      useCases: ["private_ws_runtime", "order_stream"],
      dataKinds: ["order_state_stream"],
      rateScopes: ["user"],
      weight: 1,
    },
    problem: null,
    observedAtMs: now,
  };
}

function settingsPrivateOrderStreamOkLocalCapture() {
  const message = "本地 UI/CI gate：私有订单流 capture-shaped order_state_stream Ok，非真实交易所 live 样本";
  return {
    venue: "okx",
    operation: "private_ws_order_stream",
    status: "ok",
    source: "private_ws_runtime",
    message,
    supported: true,
    configured: true,
    requested: 3,
    rows: 3,
    freshnessMs: 500,
    retryAfterMs: null,
    error: null,
    evidence: {
      method: "WS",
      path: "wss://ws.okx.com:8443/ws/v5/private#orders",
      checkedAt: "2026-07-02",
      docVersion: "okx-v5-private-ws-orders-2026-07-02",
      schemaHash: "sha256:e2e-private-order-stream-all-ok-local-capture",
      fixtureId: "e2e-private-order-stream-all-ok-local-capture",
      parserTest: "okx_orders_channel_parses_order_state_stream",
      requestBuilderTest: "okx_private_ws_subscribe_orders_channel",
      authKind: "signed_subscription",
      requestId: "req-settings-runtime-all-ok-private-stream",
      requestContext: [
        "runtime_operation=private_ws_order_stream",
        "ws_operation=orders",
        "sample_rows=3",
        "official_evidence_shape=order_state_stream",
        "source=private_ws_runtime",
        "local_ui_ci_gate=true",
        "capture_shape=selected_venue_trading_runtime_all_ok",
        "not_real_exchange_live_sample=true",
      ],
      docUrls: ["https://www.okx.com/docs-v5/en/"],
      useCases: ["private_ws_runtime", "order_stream"],
      dataKinds: ["order_state_stream"],
      rateScopes: ["user"],
      weight: 1,
    },
    problem: null,
    observedAtMs: now,
  };
}

function apiTransportWarning() {
  const message = "Gate order query HTTP degraded; using retry-after before next probe";
  return {
    venue: "gate",
    operation: "http_rest:GET /api/v4/orders",
    status: "warn",
    source: "exchange_http",
    message,
    supported: true,
    configured: true,
    requested: 1,
    rows: 0,
    freshnessMs: 2_000,
    retryAfterMs: 4_000,
    latencyMs: 41,
    latencyP95Ms: 80,
    error: message,
    evidence: {
      method: "GET",
      path: "/api/v4/orders",
      checkedAt: "2026-07-02",
      docVersion: "gate-apiv4-list-futures-orders-2026-07-02",
      schemaHash: "sha256:e2e-api-transport",
      fixtureId: "e2e-api-transport",
      parserTest: "gate_open_orders_parses_official_fixture",
      requestBuilderTest: "open_orders_rejects_malformed_order_row",
      authKind: "signed",
      requestId: "req-api-transport",
      requestContext: [
        "operation=http_rest:GET /api/v4/orders",
        "probe_status=rate_limited",
      ],
      docUrls: ["https://www.gate.com/docs/developers/apiv4/en/#list-futures-orders"],
      useCases: ["order_status", "transport_runtime"],
      dataKinds: ["order_state"],
      rateScopes: ["user"],
      weight: 1,
    },
    problem: {
      code: "EXCHANGE_HTTP_DEGRADED",
      message,
      status: 429,
      requestId: "req-api-transport",
      retryAfterMs: 4_000,
      source: "exchange_http",
      details: {
        venue: "gate",
        operation: "http_rest:GET /api/v4/orders",
      },
    },
    observedAtMs: now,
  };
}

function apiTransportFallbackWarning() {
  const message = "Gate order query HTTP outcome has no registered endpoint evidence";
  return {
    venue: "gate",
    operation: "http_rest:GET /api/v4/orders",
    status: "warn",
    source: "exchange_http",
    message,
    supported: true,
    configured: true,
    requested: 1,
    rows: 0,
    freshnessMs: 2_000,
    retryAfterMs: 4_000,
    latencyMs: 41,
    latencyP95Ms: 80,
    error: message,
    evidence: {
      method: "GET",
      path: "/api/v4/orders",
      checkedAt: "not_recorded",
      docVersion: "not_recorded",
      schemaHash: "not_recorded",
      fixtureId: "not_recorded",
      parserTest: "not_recorded",
      requestBuilderTest: "not_recorded",
      authKind: "not_recorded",
      requestId: "req-api-transport-fallback",
      requestContext: [
        "endpoint_evidence=not_recorded",
        "operation=http_rest:GET /api/v4/orders",
        "probe_status=rate_limited",
      ],
      docUrls: [],
      useCases: ["http_outcome_metrics"],
      dataKinds: ["http_outcome"],
      rateScopes: [],
      weight: 0,
    },
    problem: {
      code: "EXCHANGE_HTTP_DEGRADED",
      message,
      status: 429,
      requestId: "req-api-transport-fallback",
      retryAfterMs: 4_000,
      source: "exchange_http",
      details: {
        venue: "gate",
        operation: "http_rest:GET /api/v4/orders",
        endpointEvidence: "not_recorded",
      },
    },
    observedAtMs: now,
  };
}

function largeVenueOperationHealth() {
  const rows = [];
  for (let index = 0; index < 200; index += 1) {
    rows.push({
      venue: `venue-${String(index % 8).padStart(2, "0")}`,
      operation: `diagnostic_probe_${String(index).padStart(3, "0")}`,
      status: index % 17 === 0 ? "warn" : "ok",
      source: "e2e-large-diagnostics",
      message: `large diagnostics row ${index}`,
      supported: true,
      configured: true,
      requested: 1,
      rows: 1,
      freshnessMs: 0,
      observedAtMs: now - index,
    });
  }
  return {
    rows,
    generatedAtMs: now,
    rowCount: rows.length,
    attentionCount: rows.filter((row) => row.status !== "ok").length,
  };
}

function marketDataDiagnostics() {
  const statusRows = [
    marketStatusRow("mock", "funding_rates", "fresh", "local_cache"),
    marketStatusRow("mock", "perp_tickers", "fresh", "local_cache"),
  ];
  return {
    generatedAtMs: now,
    cache: marketCacheCounters(2),
    restBaseline: restBaselineDiagnostics(),
    status: { observedAtMs: now, rows: statusRows },
    accessRows: [
      {
        feed: "funding_rates",
        outcome: "hit",
        source: "local_cache",
        quality: "fresh",
        count: 2,
      },
    ],
    accessTotal: 2,
  };
}

function largeMarketDataDiagnostics() {
  const statusRows = [];
  const accessRows = [];
  for (let index = 0; index < 200; index += 1) {
    statusRows.push(
      marketStatusRow(
        `venue-${String(index % 8).padStart(2, "0")}`,
        index % 2 === 0 ? "funding_rates" : "perp_tickers",
        "fresh",
        "local_cache",
      ),
    );
    accessRows.push({
      feed: `feed_${String(index).padStart(3, "0")}`,
      outcome: "hit",
      source: "local_cache",
      quality: "fresh",
      count: index + 1,
    });
  }
  return {
    generatedAtMs: now,
    cache: marketCacheCounters(200),
    restBaseline: restBaselineDiagnostics(),
    status: { observedAtMs: now, rows: statusRows },
    accessRows,
    accessTotal: accessRows.length,
  };
}

function marketCacheCounters(count) {
  return {
    hitTotal: count,
    missTotal: 0,
    staleTotal: 0,
    hitRatio: 1,
    perpTickerSnapshotServedStaleTotal: 0,
    spotTickSnapshotServedStaleTotal: 0,
  };
}

function restBaselineDiagnostics() {
  return {
    orderbookGuardKeys: 0,
    orderbookInFlight: 0,
    orderbookWaitCountTotal: 0,
    orderbookWaitMsTotal: 0,
    orderbookGuardEvictedTotal: 0,
    orderbookGuardOldestIdleMs: 0,
    snapshotFeedKeys: 0,
    snapshotFeedInFlight: 0,
    snapshotWaitCountTotal: 0,
    snapshotWaitMsTotal: 0,
  };
}

function marketStatusRow(venue, operation, quality, source) {
  return {
    venue,
    operation,
    health: {
      quality,
      source,
      freshnessMs: 0,
      retryAfterMs: null,
      lastError: null,
      observedAtMs: now,
      coverage: { requested: 1, received: 1, coveragePct: 1 },
      problem: null,
    },
  };
}

function credentialValidationFailure() {
  const message = "OKX balance read permission denied";
  return {
    venue: "okx",
    operation: "credential_probe:balance_read",
    status: "blocked",
    source: "credential_validation",
    message,
    supported: true,
    configured: true,
    freshnessMs: 2_000,
    error: message,
    evidence: {
      method: "GET",
      path: "/api/v5/account/balance",
      requestId: "req-credential-probe",
      requestContext: [
        "probe_kind=balance_read",
        "probe_status=failed",
        "validation_status=permission_denied",
      ],
      docUrls: ["https://www.okx.com/docs-v5/en/"],
      useCases: ["credential_validation"],
      dataKinds: ["account"],
      rateScopes: ["user"],
      weight: 1,
    },
    problem: {
      code: "CREDENTIAL_PERMISSION_DENIED",
      message,
      status: 400,
      requestId: "req-credential-probe",
      source: "credential_validation",
      details: {
        venue: "okx",
        operation: "credential_probe:balance_read",
        scope: "private_read",
      },
    },
    observedAtMs: now,
  };
}

function privateWsAuthFailure() {
  const message = "Bybit private WS auth failed: 401 auth failed";
  return {
    venue: "bybit",
    operation: "private_ws_subscribe",
    status: "blocked",
    source: "private_ws_runtime",
    message,
    supported: true,
    configured: true,
    requested: 1,
    rows: 0,
    freshnessMs: 15_000,
    retryAfterMs: 30_000,
    error: message,
    evidence: {
      method: "WS",
      path: "wss://stream.bybit.com/v5/private",
      checkedAt: "2026-07-02",
      docVersion: "bybit-v5-private-ws-order-2026-07-02",
      schemaHash: "not_recorded",
      fixtureId: "not_recorded",
      parserTest: "parses_order_event_to_order_delta",
      requestBuilderTest: "subscribe_payload_uses_bybit_private_topics",
      authKind: "signed_subscription",
      requestId: "req-private-ws-auth",
      requestContext: [
        "phase=auth",
        "auth_status=failed",
        "operation=private_ws_subscribe",
        "runtime_operation=private_ws_subscribe",
        "schema_hash=not_recorded",
        "fixture_id=not_recorded",
      ],
      docUrls: ["https://bybit-exchange.github.io/docs/v5/websocket/private/order"],
      useCases: ["private_ws_auth", "order_stream"],
      dataKinds: ["orders"],
      rateScopes: ["user"],
      weight: 1,
    },
    problem: {
      code: "PRIVATE_WS_RUNTIME_FAILED",
      message,
      status: 401,
      requestId: "req-private-ws-auth",
      retryAfterMs: 30_000,
      source: "private_ws_runtime",
      details: {
        venue: "bybit",
        operation: "private_ws_subscribe",
        phase: "auth",
      },
    },
    observedAtMs: now,
  };
}

function privateWsOrderStreamWarning() {
  const message = "存在 2 笔未决实盘订单，但尚无新鲜私有 WS 订单流样本";
  return {
    venue: "hyperliquid:km",
    operation: "private_ws_order_stream",
    status: "warn",
    source: "private_ws_runtime",
    message,
    supported: true,
    configured: true,
    requested: 2,
    rows: 0,
    freshnessMs: 45_000,
    retryAfterMs: 60_000,
    error: message,
    evidence: {
      method: "WS",
      path: "wss://api.hyperliquid.xyz/ws#orderUpdates",
      checkedAt: "2026-07-02",
      docVersion: "hyperliquid-user-ws-order-updates-2026-07-02",
      schemaHash: "not_recorded",
      fixtureId: "not_recorded",
      parserTest: "parses_order_fills_funding_and_user_events",
      requestBuilderTest: "subscribe_payloads_match_official_user_subscriptions",
      authKind: "user_address",
      requestId: "req-private-order-stream-stale",
      requestContext: [
        "runtime_operation=private_ws_order_stream",
        "ws_operation=orderUpdates",
        "schema_hash=not_recorded",
        "fixture_id=not_recorded",
      ],
      docUrls: [
        "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/websocket/subscriptions",
      ],
      useCases: ["private_ws_runtime", "order_stream"],
      dataKinds: ["order_state_stream"],
      rateScopes: [],
      weight: 0,
    },
    problem: {
      code: "PRIVATE_WS_RUNTIME_FAILED",
      message,
      status: 409,
      requestId: "req-private-order-stream-stale",
      retryAfterMs: 60_000,
      source: "private_ws_runtime",
      details: {
        venue: "hyperliquid:km",
        operation: "private_ws_order_stream",
        unresolvedOrders: 2,
        rows: 0,
      },
    },
    observedAtMs: now,
  };
}

function settingsOrderFinalityOkLocalCapture() {
  const message = "本地 UI/CI gate：订单终态回查 capture-shaped Ok，非真实交易所 live 样本";
  return {
    venue: "okx",
    operation: "order_finality",
    status: "ok",
    source: "run_finality_runtime",
    message,
    supported: true,
    configured: true,
    requested: 1,
    rows: 1,
    freshnessMs: 900,
    retryAfterMs: null,
    error: null,
    evidence: {
      method: "internal",
      path: "run_finality.refresh_pending_runs",
      checkedAt: "not_recorded",
      docVersion: "not_recorded",
      schemaHash: "not_recorded",
      fixtureId: "not_recorded",
      parserTest: "not_recorded",
      requestBuilderTest: "not_recorded",
      authKind: "internal_order_query",
      requestId: "req-settings-runtime-all-ok-order-finality",
      requestContext: [
        "operation=order_finality",
        "scanned_order_count=1",
        "skipped_terminal_count=1",
        "terminal_count=1",
        "remote_missing_count=0",
        "refresh_failure_count=0",
        "publish_failure_count=0",
        "local_ui_ci_gate=true",
        "capture_shape=selected_venue_trading_runtime_all_ok",
        "not_real_exchange_live_sample=true",
      ],
      docUrls: [],
      useCases: [
        "order_finality",
        "execution_run_finality",
        "close_run_finality",
      ],
      dataKinds: [
        "order_state",
        "execution_run_finality",
        "close_run_finality",
      ],
      rateScopes: [],
      weight: 0,
    },
    problem: null,
    observedAtMs: now,
  };
}

function orderFinalityWarning() {
  const message = "订单终态回查完成：待确认 1，刷新 0，远端缺失 1，已终态 0，刷新失败 0，发布失败 0";
  return {
    venue: "mock",
    operation: "order_finality",
    status: "warn",
    source: "run_finality_runtime",
    message,
    supported: true,
    configured: true,
    requested: 1,
    rows: 0,
    freshnessMs: 12_000,
    retryAfterMs: 90_000,
    error: message,
    evidence: {
      method: "internal",
      path: "run_finality.refresh_pending_runs",
      checkedAt: "not_recorded",
      docVersion: "not_recorded",
      schemaHash: "not_recorded",
      fixtureId: "not_recorded",
      parserTest: "not_recorded",
      requestBuilderTest: "not_recorded",
      authKind: "internal_order_query",
      requestId: "req-order-finality-retry",
      requestContext: [
        "operation=order_finality",
        "scanned_order_count=1",
        "remote_missing_count=1",
        "refresh_failure_count=0",
        "publish_failure_count=0",
      ],
      docUrls: [],
      useCases: [
        "order_finality",
        "execution_run_finality",
        "close_run_finality",
      ],
      dataKinds: [
        "order_state",
        "execution_run_finality",
        "close_run_finality",
      ],
      rateScopes: [],
      weight: 0,
    },
    problem: {
      code: "HEDGE_ORDER_FINALITY_FAILED",
      message,
      status: 409,
      source: "run_finality_runtime",
      requestId: "req-order-finality-retry",
      retryAfterMs: 90_000,
      details: {
        venue: "mock",
        operation: "order_finality",
        remoteMissingCount: 1,
      },
    },
    observedAtMs: now,
  };
}

function venueQuality() {
  const rows = [
    venueQualityNoSampleRow("binance"),
    venueQualityNoSampleRow("okx"),
  ];
  return {
    rows,
    rowCount: rows.length,
    sampledCount: 0,
    source: "neutral_no_sample",
    generatedAtMs: now,
  };
}

function venueQualityNoSampleRow(venue) {
  return {
    venue,
    source: "neutral_no_sample",
    sampleStatus: "no_sample",
    avgRestLatencyMs: 0,
    restLatencySamples: 0,
    wsJitterP99Ms: 0,
    wsJitterSamples: 0,
    fillRatePct: 0,
    fillWindowSamples: 0,
    avgSlippageBps: 0,
    slippageSamples: 0,
    uptimeWindowPct: 0,
    uptimeWindowSamples: 0,
  };
}

function strategyKinds() {
  return [
    ...mainStrategyKinds(),
    kind("rwa_basket", "rwa", "RWA 篮子", false, "diagnostic"),
  ];
}

function mainStrategyKinds() {
  return [
    kind("perp_cross", "futures", "永续跨所", true, "main_p0"),
    kind("spot_perp", "futures", "现货-永续", true, "main_p0"),
    kind("cross_spot_perp", "futures", "跨所期现", true, "main_p0"),
  ];
}

function kind(kindName, category, labelZh, frontendEnabled, exposure) {
  const mainP0Executable = frontendEnabled && exposure === "main_p0";
  return {
    kind: kindName,
    category,
    labelZh,
    labelEn: kindName,
    description: labelZh,
    implemented: frontendEnabled,
    frontendEnabled,
    exposure,
    enginePresent: mainP0Executable,
    dataContractReady: mainP0Executable,
    executionSupported: mainP0Executable,
  };
}
