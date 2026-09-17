import { expect, test } from "@playwright/test";
import {
  ALWAYS_ON_BEARER_ROUTE_RUNTIME_POLICY,
  LOCAL_FIXTURE_SECRET,
  LOCAL_FIXTURE_TOKEN,
  ROUTE_RUNTIME_POLICY,
  sampleRouteRuntimePath,
  startRouteRuntimeFixture,
} from "./fixtures/route_runtime_policy.mjs";

let runtimeFixture;

async function browserFetch(page, baseUrl, path, init = {}) {
  return await page.evaluate(async ({ baseUrl, path, init }) => {
    const response = await fetch(`${baseUrl}${path}`, init);
    const body = await response.json();
    return {
      body,
      headers: Object.fromEntries(response.headers.entries()),
      status: response.status,
    };
  }, { baseUrl, path, init });
}

test.describe("route registry runtime policy", () => {
  test.beforeAll(async () => {
    runtimeFixture = await startRouteRuntimeFixture();
  });

  test.afterAll(async () => {
    await runtimeFixture.close();
  });

  test.beforeEach(async ({ page }) => {
    await page.goto("/");
  });

  test("core route metadata is authenticated and request-correlated", async ({ page }) => {
    const requestId = "route-registry-metadata-rid";
    const unauthorized = await browserFetch(page, runtimeFixture.baseUrl, "/api/e2e/route-runtime-policy/metadata", {
      headers: { "x-request-id": requestId },
    });

    expect(unauthorized.status).toBe(401);
    expect(unauthorized.headers["x-request-id"]).toBe(requestId);
    expect(unauthorized.body.error).toMatchObject({
      code: "UNAUTHORIZED",
      requestId,
    });

    const metadata = await browserFetch(page, runtimeFixture.baseUrl, "/api/e2e/route-runtime-policy/metadata", {
      headers: { Authorization: `Bearer ${LOCAL_FIXTURE_TOKEN}` },
    });

    expect(metadata.status).toBe(200);
    expect(metadata.body.routes).toEqual(ROUTE_RUNTIME_POLICY);
    expect(metadata.body.alwaysOnBearerRoutes).toEqual(ALWAYS_ON_BEARER_ROUTE_RUNTIME_POLICY);
    expect(JSON.stringify(metadata.body)).not.toContain(LOCAL_FIXTURE_SECRET);
  });

  test("every always-on bearer route preserves browser auth and typed error boundaries", async ({ page }) => {
    expect(ALWAYS_ON_BEARER_ROUTE_RUNTIME_POLICY.length).toBeGreaterThanOrEqual(60);
    const routeClasses = new Set(ALWAYS_ON_BEARER_ROUTE_RUNTIME_POLICY.map((route) => route.class));
    for (const routeClass of ["main_p0", "diagnostic", "legacy", "readiness"]) {
      expect(routeClasses).toContain(routeClass);
    }

    for (const [index, route] of ALWAYS_ON_BEARER_ROUTE_RUNTIME_POLICY.entries()) {
      const matrixPath = `${sampleRouteRuntimePath(route.path)}?routeRuntimeMatrix=1`;
      const missingRequestId = `route-runtime-matrix-missing-${index + 1}`;
      const missing = await browserFetch(page, runtimeFixture.baseUrl, matrixPath, {
        headers: { "x-request-id": missingRequestId },
        method: route.method,
      });

      expect(missing.status).toBe(401);
      expect(missing.headers["x-request-id"]).toBe(missingRequestId);
      expect(missing.body.error).toMatchObject({
        code: "UNAUTHORIZED",
        requestId: missingRequestId,
      });

      const authenticatedRequestId = `route-runtime-matrix-auth-${index + 1}`;
      const authenticated = await browserFetch(page, runtimeFixture.baseUrl, matrixPath, {
        headers: {
          Authorization: `Bearer ${LOCAL_FIXTURE_TOKEN}`,
          "x-request-id": authenticatedRequestId,
        },
        method: route.method,
      });

      expect(authenticated.headers["x-request-id"]).toBe(authenticatedRequestId);
      if (route.method === "GET") {
        expect(authenticated.status).toBe(200);
        expect(authenticated.body.route).toMatchObject(route);
      } else {
        expect(authenticated.status).toBe(422);
        expect(authenticated.body.error).toMatchObject({
          code: "REQUEST_BODY_INVALID",
          details: { route: `${route.method} ${route.path}` },
          requestId: authenticatedRequestId,
          status: 422,
        });
      }
    }
  });

  test("gated diagnostic routes stay absent even with local bearer credentials", async ({ page }) => {
    const disabledRoutes = ROUTE_RUNTIME_POLICY.filter(
      (route) => route.defaultExposure === "default_off" && route.method === "GET",
    );

    expect(disabledRoutes).toEqual(expect.arrayContaining([
      expect.objectContaining({
        label: "disabled_spot_diagnostic",
        class: "diagnostic",
        featureFlag: "api_surface.spot_v1",
      }),
      expect.objectContaining({
        label: "disabled_strategy_diagnostic",
        class: "diagnostic",
        featureFlag: "api_surface.strategy_v1",
      }),
    ]));

    for (const route of disabledRoutes) {
      const disabled = await browserFetch(page, runtimeFixture.baseUrl, route.path, {
        headers: { Authorization: `Bearer ${LOCAL_FIXTURE_TOKEN}` },
      });

      expect(disabled.status).toBe(404);
      expect(disabled.body.error).toMatchObject({ code: "ROUTE_DISABLED" });
      expect(route.featureFlag).toMatch(/^api_surface\./);
    }
  });

  test("high-risk action and secret mutation persist correlated redacted audit pairs", async ({ page }) => {
    runtimeFixture.resetAuditEvents();
    const actionRequestId = "route-registry-action-rid";
    const actionIdempotencyKey = "route-registry-action-idem";
    const actionPayload = {
      active: true,
      apiKey: "local-route-runtime-fixture-key",
      apiSecret: LOCAL_FIXTURE_SECRET,
    };
    const missingAuth = await browserFetch(page, runtimeFixture.baseUrl, "/api/trading/kill-switch", {
      body: JSON.stringify(actionPayload),
      headers: { "content-type": "application/json", "x-request-id": actionRequestId },
      method: "POST",
    });

    expect(missingAuth.status).toBe(401);
    expect(missingAuth.body.error).toMatchObject({ code: "UNAUTHORIZED", requestId: actionRequestId });

    const accepted = await browserFetch(page, runtimeFixture.baseUrl, "/api/trading/kill-switch", {
      body: JSON.stringify(actionPayload),
      headers: {
        Authorization: `Bearer ${LOCAL_FIXTURE_TOKEN}`,
        "content-type": "application/json",
        "idempotency-key": actionIdempotencyKey,
        "x-request-id": actionRequestId,
      },
      method: "POST",
    });

    expect(accepted.status).toBe(200);
    expect(accepted.body.actionRun).toEqual({
      id: "action-run-kill-switch",
      status: "succeeded",
    });
    expect(JSON.stringify(accepted.body)).not.toContain(actionPayload.apiKey);
    expect(JSON.stringify(accepted.body)).not.toContain(LOCAL_FIXTURE_SECRET);

    const secretRequestId = "route-registry-secret-rid";
    const secretIdempotencyKey = "route-registry-secret-idem";
    const secretPayload = {
      venue: "okx",
      fields: [
        { key: "api_key", value: "local-route-runtime-secret-key" },
        { key: "api_secret", value: LOCAL_FIXTURE_SECRET },
      ],
    };
    const denied = await browserFetch(page, runtimeFixture.baseUrl, "/api/exchanges/credentials", {
      body: JSON.stringify(secretPayload),
      headers: {
        Authorization: `Bearer ${LOCAL_FIXTURE_TOKEN}`,
        "content-type": "application/json",
        "idempotency-key": secretIdempotencyKey,
        "x-request-id": secretRequestId,
      },
      method: "POST",
    });

    expect(denied.status).toBe(400);
    expect(denied.body.error).toMatchObject({
      code: "CREDENTIAL_PERMISSION_DENIED",
      details: {
        actionRunId: "action-run-venue-credentials",
        idempotencyKey: secretIdempotencyKey,
      },
      requestId: secretRequestId,
      status: 400,
    });
    expect(JSON.stringify(denied.body)).not.toContain(secretPayload.fields[0].value);
    expect(JSON.stringify(denied.body)).not.toContain(LOCAL_FIXTURE_SECRET);

    const auditEvents = await browserFetch(page, runtimeFixture.baseUrl, "/api/e2e/route-runtime-policy/audit-events", {
      headers: { Authorization: `Bearer ${LOCAL_FIXTURE_TOKEN}` },
    });
    expect(auditEvents.status).toBe(200);
    expect(auditEvents.body.events).toHaveLength(5);
    expect(auditEvents.body.events).toEqual(expect.arrayContaining([
      expect.objectContaining({
        action: "trading.kill_switch.set",
        actor: "unknown",
        outcome: "denied",
        problemCode: "UNAUTHORIZED",
        requestId: actionRequestId,
        status: 401,
      }),
      expect.objectContaining({
        action: "trading.kill_switch.set",
        actionRunId: accepted.body.actionRun.id,
        idempotencyKey: actionIdempotencyKey,
        outcome: "accepted",
        redactedFields: ["apiKey", "apiSecret"],
        requestId: actionRequestId,
      }),
      expect.objectContaining({
        action: "trading.kill_switch.set",
        actionRunId: accepted.body.actionRun.id,
        idempotencyKey: actionIdempotencyKey,
        outcome: "success",
        requestId: actionRequestId,
      }),
      expect.objectContaining({
        action: "venue_credentials.update",
        actionRunId: denied.body.error.details.actionRunId,
        idempotencyKey: secretIdempotencyKey,
        outcome: "accepted",
        redactedFields: ["fields.api_key", "fields.api_secret"],
        requestId: secretRequestId,
      }),
      expect.objectContaining({
        action: "venue_credentials.update",
        actionRunId: denied.body.error.details.actionRunId,
        idempotencyKey: secretIdempotencyKey,
        outcome: "denied",
        requestId: secretRequestId,
      }),
    ]));
    expect(auditEvents.body.events
      .filter((event) => event.actor !== "unknown")
      .every((event) => event.actor === "api-token:local-route-runtime"))
      .toBe(true);
    expect(JSON.stringify(auditEvents.body)).not.toContain(actionPayload.apiKey);
    expect(JSON.stringify(auditEvents.body)).not.toContain(secretPayload.fields[0].value);
    expect(JSON.stringify(auditEvents.body)).not.toContain(LOCAL_FIXTURE_SECRET);
  });
});
