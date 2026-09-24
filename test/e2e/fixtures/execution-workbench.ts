import { expect, type Page } from "@playwright/test";
import { API, NOW, setup } from "./opportunity-workbench";

export async function executionFixture(page: Page) {
  const f = await setup(page);
  const opportunity = f.rows[0];
  opportunity.longLeg.venue = "hyperliquid:km";
  opportunity.longLeg.action = "hyperliquid:km 做多永续";
  opportunity.longLeg.marketEvidence.venue = "hyperliquid:km";
  opportunity.shortLeg.venue = "kucoin";
  opportunity.shortLeg.action = "kucoin 做空永续";
  opportunity.shortLeg.marketEvidence.venue = "kucoin";
  const seed = await (await page.request.get(`${API}/api/arbitrage/opportunities/mock-mu-perp/preview`)).json();
  let version = 0;
  let holdValidation = false;
  let releaseValidation: (() => void) | undefined;
  let holdBuild = false;
  let releaseBuild: (() => void) | undefined;
  let failValidation = false;
  let mismatch = false;
  let expiry = NOW + 60_000;
  let holdPreview = false;
  let releasePreview: (() => void) | undefined;
  let reboundSnapshot: string | undefined, requestedSnapshotOverride: string | undefined;
  const previews: any[] = [];
  const builds: any[] = [];
  const validations: any[] = [];
  const artifacts = new Map<string, any>();
  const makeArtifact = (request: any) => ({
    schemaVersion: "crossline.execution-artifact.v1", artifactId: `artifact-${request.ticketId}`,
    ...request, opportunityId: "fixture-perp_cross-BTC", symbol: "BTC", environment: "paper",
    strategy: "perp_cross", generatedAtMs: NOW, expiresAtMs: expiry, status: "ready",
    expectedGrossEdgeUsd: 1.7, expectedTotalCostUsd: 0.46, expectedNetEdgeUsd: 1.24,
    capitalUsd: previews.at(-1)?.capitalUsd ?? 750, maxLossUsd: 0.45,
    legs: [
      { role: "long", venue: "hyperliquid:km", symbol: "BTC", side: "buy", targetNotionalUsd: 750, marketObservedAtMs: NOW },
      { role: "short", venue: "kucoin", symbol: "BTC", side: "sell", targetNotionalUsd: 750, marketObservedAtMs: NOW },
    ], evidence: [{ key: "snapshot", label: "快照绑定", passed: true, detail: "isolated fixture" }],
    invalidationConditions: ["票据过期", "参数改变"], blockers: [], checksum: `checksum-${request.ticketId}`,
    validationCommand: `curl -X POST http://127.0.0.1:18000/api/automation/execution-artifacts/validate --data '{"ticketId":"${request.ticketId}"}'`,
  });
  await page.route("**/api/**", async (route) => {
    const url = new URL(route.request().url());
    if (url.origin !== API) return route.fallback();
    if (url.pathname.endsWith("/preview") && route.request().method() === "POST") {
      const input = route.request().postDataJSON();
      previews.push(input);
      const response = JSON.parse(JSON.stringify(seed).replaceAll("MU", "BTC"));
      version++;
      Object.assign(response, { opportunityId: input.opportunityId,
        opportunitySnapshotId: reboundSnapshot ?? input.opportunitySnapshotId,
        requestedOpportunitySnapshotId: requestedSnapshotOverride ?? input.opportunitySnapshotId,
        idempotencyKey: `preview-${version}`,
        estimatedGrossEdgeUsd: 1.7, usedCapitalUsd: input.capitalUsd });
      Object.assign(response.ticket, { ticketId: `ticket-${version}`, opportunityId: input.opportunityId,
        createdAtMs: NOW, expiresAtMs: NOW + 300_000 });
      response.ticketOrderPlans.ticketId = response.ticket.ticketId;
      response.longRisk.computedNotional = input.longNotionalUsd;
      response.shortRisk.computedNotional = input.shortNotionalUsd;
      response.ticket.longLeg.referencePrice = input.longPrice;
      response.ticket.shortLeg.referencePrice = input.shortPrice;
      if (holdPreview) {
        holdPreview = false;
        await new Promise<void>((resolve) => { releasePreview = resolve; });
      }
      return route.fulfill({ json: response });
    }
    if (url.pathname === "/api/automation/execution-artifacts/build") {
      const request = route.request().postDataJSON();
      builds.push(request);
      const artifact = makeArtifact(request);
      artifacts.set(request.ticketId, artifact);
      if (holdBuild) await new Promise<void>((resolve) => { releaseBuild = resolve; });
      return route.fulfill({ json: mismatch ? { ...artifact, ticketId: "wrong-ticket" } : artifact });
    }
    if (url.pathname === "/api/automation/execution-artifacts/validate") {
      const request = route.request().postDataJSON();
      validations.push(request);
      const artifact = structuredClone(artifacts.get(request.ticketId));
      if (holdValidation) await new Promise<void>((resolve) => { releaseValidation = resolve; });
      if (failValidation) return route.fulfill({ status: 503, json: { code: "VALIDATION_UNAVAILABLE", message: "fixture: validation unavailable" } });
      return route.fulfill({ json: { valid: true, status: "ready", checkedAtMs: NOW,
        expiresAtMs: artifact.expiresAtMs, artifact, blockers: [] } });
    }
    if (url.pathname === "/api/trading/execution-runs") {
      const response = await (await route.fetch()).json();
      return route.fulfill({ json: { ...response, rows: [] } });
    }
    return route.fallback();
  });
  return { ...f, previews, builds, validations,
    holdValidation: () => { holdValidation = true; },
    releaseValidation: () => { holdValidation = false; releaseValidation?.(); },
    holdBuild: () => { holdBuild = true; },
    releaseBuild: () => { holdBuild = false; releaseBuild?.(); },
    failValidation: () => { failValidation = true; },
    mismatch: () => { mismatch = true; },
    expireAt: (value: number) => { expiry = value; },
    holdPreview: () => { holdPreview = true; },
    releasePreview: () => releasePreview?.(),
    rebindSnapshot: (snapshot: string, requested?: string) => { reboundSnapshot = snapshot; requestedSnapshotOverride = requested; },
  };
}

export async function openExecution(page: Page) {
  await page.goto("/#futures");
  await page.getByRole("button", { name: "构建新双腿", exact: true }).click();
  await expect(page.locator(".execution-artifact-status")).toContainText(/READY|待校验/);
}
