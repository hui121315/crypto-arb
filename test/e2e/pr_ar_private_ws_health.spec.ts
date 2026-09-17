import { expect, test, type Page, type Route } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";
const WEB_BASE = process.env.CROSSLINE_E2E_WEB_BASE ?? "http://127.0.0.1:18080";

const corsHeaders = {
  "access-control-allow-origin": WEB_BASE,
  "access-control-allow-methods": "GET,POST,OPTIONS",
  "access-control-allow-headers": "authorization,accept,content-type,x-request-id",
  "content-type": "application/json; charset=utf-8",
  vary: "origin",
};

async function fulfillJson(route: Route, body: unknown) {
  if (route.request().method() === "OPTIONS") {
    await route.fulfill({ status: 204, headers: corsHeaders, body: "" });
    return;
  }
  await route.fulfill({ status: 200, headers: corsHeaders, body: JSON.stringify(body) });
}

function privateWsDirtySnapshot() {
  const message =
    "venue=okx; scope=positions; reason=position_snapshot_incomplete_or_incremental; bounded REST refetch pending";
  return {
    rows: [
      {
        venue: "okx",
        operation: "private_ws_account_stream",
        status: "warn",
        source: "private_ws_runtime",
        message,
        supported: true,
        configured: true,
        requested: 1,
        rows: 0,
        freshnessMs: 25,
        error: message,
        evidence: {
          method: "WS",
          path: "wss://ws.okx.com:8443/ws/v5/private#account+positions",
          checkedAt: "2026-07-18",
          docVersion: "okx-v5-private-ws-account-positions",
          schemaHash: "not_recorded",
          fixtureId: "pr-ar-account-dirty",
          parserTest: "eight_venue_account_dirty_matrix_preserves_scope_and_reason",
          requestBuilderTest: "private_ws_subscription_registry",
          authKind: "login",
          requestId: "req-pr-ar-dirty",
          requestContext: [
            "runtime_operation=private_ws_account_stream",
            "account_dirty_venue=okx",
            "account_dirty_scope=positions",
            "account_dirty_reason=position_snapshot_incomplete_or_incremental",
            "account_dirty_refetch=bounded_rest_on_next_read",
          ],
          docUrls: ["https://www.okx.com/docs-v5/en/#trading-account-websocket-positions-channel"],
          useCases: ["PrivateRead"],
          dataKinds: ["Account"],
          rateScopes: ["private_ws"],
          weight: 0,
        },
        problem: {
          code: "PRIVATE_WS_RUNTIME_FAILED",
          message,
          requestId: "req-pr-ar-dirty",
          source: "private_ws_runtime",
          details: {
            accountDirty: {
              venue: "okx",
              scope: "positions",
              reason: "position_snapshot_incomplete_or_incremental",
              refetch: "bounded_rest_on_next_read",
            },
          },
        },
        observedAtMs: 1_789_000_000_000,
      },
    ],
    generatedAtMs: 1_789_000_000_025,
    rowCount: 1,
    attentionCount: 1,
  };
}

async function usePrArScenario(page: Page) {
  await page.addInitScript((apiBase) => {
    window.localStorage.setItem("api_base", JSON.stringify(apiBase));
    window.localStorage.setItem("api_auth_token", JSON.stringify("e2e-token"));
  }, API_BASE);
  await page.route("**/api/auth/ws-ticket**", (route) =>
    fulfillJson(route, { ticket: "pr-ar-ticket", expiresAtMs: Date.now() + 60_000 }),
  );
  await page.route("**/api/system/venue-operation-health**", (route) =>
    fulfillJson(route, privateWsDirtySnapshot()),
  );
  await page.routeWebSocket("**/ws", (socket) => socket.close());
}

test("PR-AR surfaces scoped AccountDirty and bounded refetch evidence in Settings", async ({ page }) => {
  await usePrArScenario(page);
  await page.goto("/#settings");
  await page.getByRole("tab", { name: "诊断" }).click();

  const table = page.getByRole("table").filter({
    has: page.getByRole("columnheader", { name: "操作", exact: true }),
  });
  const row = table.getByRole("row").filter({ hasText: "private_ws_account_stream" });
  await expect(row).toHaveCount(1);
  await expect(row).toContainText("PRIVATE_WS_RUNTIME_FAILED");
  await expect(row).toContainText("venue=okx");
  await expect(row).toContainText("scope=positions");
  await expect(row.locator("td").last()).toHaveAttribute(
    "title",
    /account_dirty_refetch=bounded_rest_on_next_read/,
  );

  await page.getByPlaceholder("交易所 / 操作 / 来源 / 问题").fill("account_dirty_scope=positions");
  await expect(row).toHaveCount(1);
});
