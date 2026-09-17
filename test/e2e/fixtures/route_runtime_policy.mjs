import { createServer } from "node:http";
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createRouteRuntimeAuditRecorder } from "./route_runtime_audit.mjs";

export const LOCAL_FIXTURE_TOKEN = "local-route-runtime-fixture-token";
export const LOCAL_FIXTURE_SECRET = "route-runtime-fixture-secret";

const INVENTORY_HEADER = [
  "path",
  "methods",
  "router",
  "class",
  "default_exposure",
  "risk",
  "auth_policy",
  "audit_policy",
  "feature_flag",
  "shared_dto",
  "frontend_client",
  "owner",
  "notes",
];
const REPOSITORY_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../..");
const ROUTE_INVENTORY_PATH = path.join(REPOSITORY_ROOT, "docs", "API_ROUTE_INVENTORY.tsv");

function loadAlwaysOnBearerRouteRuntimePolicy() {
  const lines = readFileSync(ROUTE_INVENTORY_PATH, "utf8").trim().split("\n");
  const header = lines.shift()?.split("\t");
  if (!header || header.join("\t") !== INVENTORY_HEADER.join("\t")) {
    throw new Error("route runtime fixture inventory header drifted");
  }

  return Object.freeze(lines.flatMap((line) => {
    const values = line.split("\t");
    if (values.length !== header.length) {
      throw new Error(`route runtime fixture inventory row has ${values.length} columns: ${line}`);
    }
    const row = Object.fromEntries(header.map((field, index) => [field, values[index]]));
    if (row.default_exposure !== "always" || row.auth_policy !== "bearer") return [];
    return row.methods.split(",").map((method) => Object.freeze({
      auditPolicy: row.audit_policy,
      authPolicy: row.auth_policy,
      class: row.class,
      defaultExposure: row.default_exposure,
      featureFlag: row.feature_flag,
      method,
      path: row.path,
      risk: row.risk,
    }));
  }));
}

export const ALWAYS_ON_BEARER_ROUTE_RUNTIME_POLICY = loadAlwaysOnBearerRouteRuntimePolicy();

export function sampleRouteRuntimePath(routePath) {
  return routePath.replace(/:[A-Za-z_][A-Za-z0-9_]*/g, "route-runtime-fixture-id");
}
export const ROUTE_RUNTIME_POLICY = Object.freeze([
  Object.freeze({
    label: "core_status",
    path: "/api/trading/status",
    method: "GET",
    class: "main_p0",
    defaultExposure: "always",
    risk: "low",
    authPolicy: "bearer",
    auditPolicy: "read",
    featureFlag: "core",
  }),
  Object.freeze({
    label: "disabled_spot_diagnostic",
    path: "/api/v1/spot/ticks",
    method: "GET",
    class: "diagnostic",
    defaultExposure: "default_off",
    risk: "low",
    authPolicy: "bearer",
    auditPolicy: "read",
    featureFlag: "api_surface.spot_v1",
  }),
  Object.freeze({
    label: "disabled_strategy_diagnostic",
    path: "/api/v1/strategy/kinds",
    method: "GET",
    class: "diagnostic",
    defaultExposure: "default_off",
    risk: "low",
    authPolicy: "bearer",
    auditPolicy: "read",
    featureFlag: "api_surface.strategy_v1",
  }),
  Object.freeze({
    label: "high_risk_action",
    action: "trading.kill_switch.set",
    actionKind: "trading_kill_switch",
    resourceKind: "kill_switch",
    path: "/api/trading/kill-switch",
    method: "POST",
    class: "main_p0",
    defaultExposure: "always",
    risk: "high",
    authPolicy: "bearer",
    auditPolicy: "action_run",
    featureFlag: "core",
  }),
  Object.freeze({
    label: "high_risk_secret",
    action: "venue_credentials.update",
    actionKind: "venue_credentials_update",
    resourceKind: "venue_credentials",
    path: "/api/exchanges/credentials",
    method: "POST",
    class: "main_p0",
    defaultExposure: "always",
    risk: "high",
    authPolicy: "bearer",
    auditPolicy: "secret_mutation",
    featureFlag: "core",
  }),
  Object.freeze({
    label: "high_risk_execution",
    action: "hedge.confirm",
    actionKind: "hedge_confirm",
    resourceKind: "hedge_ticket",
    path: "/api/arbitrage/opportunities/:id/confirm",
    method: "POST",
    class: "main_p0",
    defaultExposure: "always",
    risk: "high",
    authPolicy: "bearer",
    auditPolicy: "action_run",
    featureFlag: "core",
  }),
]);

const METADATA_PATH = "/api/e2e/route-runtime-policy/metadata";
const AUDIT_PATH = "/api/e2e/route-runtime-policy/audit-events";

function routePolicy(label) {
  const route = ROUTE_RUNTIME_POLICY.find((candidate) => candidate.label === label);
  if (!route) throw new Error(`missing route runtime policy ${label}`);
  return route;
}

function matchesRouteTemplate(route, method, pathname) {
  if (route.method !== method) return false;
  const template = route.path.split("/");
  const actual = pathname.split("/");
  return template.length === actual.length
    && template.every((segment, index) => segment.startsWith(":") || segment === actual[index]);
}

function requestId(request) {
  const value = request.headers["x-request-id"];
  return typeof value === "string" && value.length > 0 ? value : "route-runtime-fixture-request";
}

function allowBrowserOrigin(request, response) {
  const origin = request.headers.origin;
  if (typeof origin !== "string" || !/^http:\/\/127\.0\.0\.1:\d+$/.test(origin)) return;

  response.setHeader("access-control-allow-origin", origin);
  response.setHeader(
    "access-control-allow-headers",
    "authorization,content-type,idempotency-key,x-idempotency-key,x-request-id",
  );
  response.setHeader("access-control-allow-methods", "GET,POST,PATCH,OPTIONS");
  response.setHeader("access-control-expose-headers", "www-authenticate,x-request-id");
  response.setHeader("vary", "origin");
}

function sendJson(request, response, status, body) {
  allowBrowserOrigin(request, response);
  response.writeHead(status, {
    "content-type": "application/json; charset=utf-8",
    "x-request-id": requestId(request),
  });
  response.end(JSON.stringify(body));
}

function unauthorized(request, response) {
  response.setHeader("www-authenticate", 'Bearer realm="crossline-api"');
  sendJson(request, response, 401, {
    error: {
      code: "UNAUTHORIZED",
      details: { recoveryAction: "provide_valid_bearer_token" },
      message: "authentication required; provide a valid Bearer token and retry",
      requestId: requestId(request),
      source: "api.auth",
      status: 401,
    },
  });
}

function isAuthorized(request) {
  return request.headers.authorization === `Bearer ${LOCAL_FIXTURE_TOKEN}`;
}

async function readJson(request) {
  let body = "";
  for await (const chunk of request) body += chunk;
  return body.length === 0 ? {} : JSON.parse(body);
}

export async function startRouteRuntimeFixture() {
  const auditEvents = [];
  const { recordAuditPair, recordDeniedAudit } = createRouteRuntimeAuditRecorder(
    auditEvents,
    requestId,
  );
  const server = createServer(async (request, response) => {
    const url = new URL(request.url ?? "/", "http://127.0.0.1");

    if (request.method === "OPTIONS") {
      allowBrowserOrigin(request, response);
      response.writeHead(204);
      response.end();
      return;
    }

    if (request.method === "GET" && url.pathname === METADATA_PATH) {
      if (!isAuthorized(request)) return unauthorized(request, response);
      return sendJson(request, response, 200, {
        alwaysOnBearerRoutes: ALWAYS_ON_BEARER_ROUTE_RUNTIME_POLICY,
        routes: ROUTE_RUNTIME_POLICY,
      });
    }

    if (request.method === "GET" && url.pathname === AUDIT_PATH) {
      if (!isAuthorized(request)) return unauthorized(request, response);
      return sendJson(request, response, 200, { events: auditEvents });
    }

    const disabledRoute = ROUTE_RUNTIME_POLICY.find(
      (route) => route.defaultExposure === "default_off"
        && route.method === request.method
        && route.path === url.pathname,
    );
    if (disabledRoute) {
      return sendJson(request, response, 404, {
        error: {
          code: "ROUTE_DISABLED",
          message: `${disabledRoute.class} route is disabled by default`,
          requestId: requestId(request),
        },
      });
    }

    const alwaysOnRoute = ALWAYS_ON_BEARER_ROUTE_RUNTIME_POLICY.find(
      (route) => route.method === request.method && route.path === url.pathname,
    ) ?? ALWAYS_ON_BEARER_ROUTE_RUNTIME_POLICY.find((route) =>
      matchesRouteTemplate(route, request.method, url.pathname),
    );
    if (url.searchParams.get("routeRuntimeMatrix") === "1" && alwaysOnRoute) {
      if (!isAuthorized(request)) return unauthorized(request, response);
      if (request.method === "GET") {
        return sendJson(request, response, 200, { route: alwaysOnRoute });
      }
      return sendJson(request, response, 422, {
        error: {
          code: "REQUEST_BODY_INVALID",
          details: { route: `${alwaysOnRoute.method} ${alwaysOnRoute.path}` },
          message: "local route runtime matrix rejects mutation probes before side effects",
          requestId: requestId(request),
          status: 422,
        },
      });
    }

    if (
      request.method === "POST"
      && url.pathname === routePolicy("high_risk_action").path
    ) {
      if (!isAuthorized(request)) {
        recordDeniedAudit({ request, route: routePolicy("high_risk_action") });
        return unauthorized(request, response);
      }

      const body = await readJson(request);
      const actionRunId = "action-run-kill-switch";
      const idempotencyKey = request.headers["idempotency-key"] ?? "route-runtime-kill";
      const audit = {
        action: "trading.kill_switch.set",
        actionRunId,
        idempotencyKey,
        outcome: "success",
        redactedFields: ["apiKey", "apiSecret"].filter((field) => field in body),
      };
      recordAuditPair({ ...audit, request });
      return sendJson(request, response, 200, {
        actionRun: { id: actionRunId, status: "succeeded" },
      });
    }

    if (
      request.method === "POST"
      && url.pathname === routePolicy("high_risk_secret").path
    ) {
      if (!isAuthorized(request)) {
        recordDeniedAudit({ request, route: routePolicy("high_risk_secret") });
        return unauthorized(request, response);
      }

      const body = await readJson(request);
      const actionRunId = "action-run-venue-credentials";
      const idempotencyKey = request.headers["idempotency-key"] ?? "route-runtime-credentials";
      recordAuditPair({
        action: "venue_credentials.update",
        actionRunId,
        idempotencyKey,
        outcome: "denied",
        redactedFields: Array.isArray(body.fields)
          ? body.fields.map((field) => `fields.${field.key}`)
          : [],
        request,
      });
      return sendJson(request, response, 400, {
        error: {
          code: "CREDENTIAL_PERMISSION_DENIED",
          details: { actionRunId, idempotencyKey },
          message: "credential verification denied by local runtime fixture",
          requestId: requestId(request),
          status: 400,
        },
      });
    }

    if (
      request.method === "POST"
      && matchesRouteTemplate(routePolicy("high_risk_execution"), request.method, url.pathname)
    ) {
      if (!isAuthorized(request)) {
        recordDeniedAudit({ request, route: routePolicy("high_risk_execution") });
        return unauthorized(request, response);
      }

      await readJson(request);
      const actionRunId = "action-run-hedge-confirm";
      const idempotencyKey = request.headers["idempotency-key"] ?? "route-runtime-hedge";
      const orderIds = ["long-order-1", "short-order-1"];
      const runIds = ["execution-run-1"];
      recordAuditPair({
        action: "hedge.confirm",
        actionRunId,
        idempotencyKey,
        orderIds,
        outcome: "success",
        redactedFields: [],
        request,
        runIds,
      });
      return sendJson(request, response, 200, {
        actionRun: { id: actionRunId, status: "succeeded" },
        executionRun: {
          longLeg: { orderIds: [orderIds[0]] },
          runId: runIds[0],
          shortLeg: { orderIds: [orderIds[1]] },
        },
      });
    }

    return sendJson(request, response, 404, {
      error: {
        code: "ROUTE_NOT_FOUND",
        message: "local route runtime fixture has no matching route",
        requestId: requestId(request),
      },
    });
  });

  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });

  const address = server.address();
  if (!address || typeof address === "string") {
    await new Promise((resolve, reject) => server.close((error) => error ? reject(error) : resolve()));
    throw new Error("route runtime fixture did not bind a TCP address");
  }

  return {
    baseUrl: `http://127.0.0.1:${address.port}`,
    resetAuditEvents() {
      auditEvents.length = 0;
    },
    async close() {
      await new Promise((resolve, reject) => server.close((error) => error ? reject(error) : resolve()));
    },
  };
}
