import { type Page } from "@playwright/test";
import { settingsFixture } from "./settings-workbench";
import { API, NOW } from "./opportunity-workbench";

export async function riskFixture(page: Page) {
  const base = await settingsFixture(page, "risk");
  const status = await (await page.request.get(`${API}/api/trading/status`)).json();
  status.risk.maxOrderNotional = 10;
  status.risk.maxOpenOrders = 10;
  status.risk.autoProfitClose = { enabled: false, minNetProfitUsd: 0.5, minRoiBps: 10,
    exitBufferBps: 5, stopLossEnabled: false, maxNetLossUsd: 2, maxLossRoiBps: 100,
    liquidationGuardEnabled: false, liquidationExitDistancePct: 5, confirmationSamples: 2, cooldownSecs: 60 };
  const calls: { body: any; key?: string; requestId?: string; path: string }[] = [];
  const reads: number[] = [];
  let hold = false, fail = false, reject = false, release: (() => void) | undefined;
  await page.route("**/api/trading/status", (route) => {
    reads.push(reads.length);
    return route.fulfill({ json: status });
  });
  await page.route(/\/api\/trading\/(risk-config|kill-switch)$/, async (route) => {
    const request = route.request(), body = request.postDataJSON(), path = new URL(request.url()).pathname;
    const requestId = request.headers()["x-request-id"], idempotencyKey = request.headers()["idempotency-key"];
    calls.push({ body, key: idempotencyKey, requestId, path });
    const kill = path.endsWith("kill-switch");
    const run = { id: `fixture-risk-${calls.length}`, kind: kill ? "trading_kill_switch" : "trading_risk_config_update",
      target: kill ? (body.active ? "kill-switch:on" : "kill-switch:off") : "risk-config",
      requestId, idempotencyKey, status: "accepted", actor: "fixture", message: "fixture accepted",
      startedAtMs: NOW, updatedAtMs: NOW, result: null as any, problem: null as any };
    base.actions.data.unshift(run);
    const lostResponse = fail, rejected = reject;
    if (hold) { hold = false; await new Promise<void>((resolve) => { release = resolve; }); }
    if (rejected) {
      const problem = { code: "FIXTURE_REJECTED", message: "fixture config was not applied", status: 503 };
      Object.assign(run, { status: "failed", problem });
      return route.fulfill({ status: 503, json: { error: problem } });
    }
    if (kill) status.risk.killSwitchActive = body.active;
    else for (const [key, value] of Object.entries(body)) {
      if (value == null) continue;
      if (key === "autoProfitClose") {
        for (const [field, update] of Object.entries(value as object))
          if (update != null) status.risk.autoProfitClose[field] = update;
      } else status.risk[key] = value;
    }
    const receipt = { ...structuredClone(status), requestId, actionRunId: run.id, idempotencyKey };
    const result = kill ? { status: receipt, summary: { previousActive: body.expectedActive,
      active: body.active, openOrderCount: status.openOrderCount, expectedOpenOrderCount: body.expectedOpenOrderCount,
      reason: body.reason, checkedAtMs: NOW }, requestId, actionRunId: run.id, idempotencyKey } : receipt;
    Object.assign(run, { status: "succeeded", result: structuredClone(result), message: "fixture confirmed" });
    return lostResponse
      ? route.fulfill({ status: 504, json: { error: { code: "TIMEOUT", message: "fixture receipt lost" } } })
      : route.fulfill({ json: result });
  });
  return { ...base, status, calls, reads, hold: () => { hold = true; }, fail: (v: boolean) => { fail = v; },
    reject: (v: boolean) => { reject = v; }, release: () => release?.() };
}
