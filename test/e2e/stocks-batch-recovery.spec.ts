import { expect, test } from "@playwright/test";

const API = "http://127.0.0.1:18000", WEB = "http://127.0.0.1:18080";

test("real BP batch reload recovers original receipt and keeps newer settings without replay", async ({ page, request }) => {
  const headers = { Authorization: "Bearer isolated-paper-browser" };
  const writes: any[] = [], errors: string[] = [], unexpected: string[] = [];
  let loseReply = true, release = () => {};
  const current = async () => (await request.get(API + "/api/stocks/peer/plans", { headers })).json();
  await page.addInitScript(api => {
    localStorage.setItem("api_base", JSON.stringify(api));
    localStorage.setItem("api_auth_token", JSON.stringify("isolated-paper-browser"));
  }, API);
  page.on("pageerror", error => errors.push(error.message));
  await page.route("**/*", async route => {
    const req = route.request(), url = new URL(req.url());
    if (![API, WEB].includes(url.origin) || (!["GET", "HEAD"].includes(req.method())
      && !["/api/auth/ws-ticket", "/api/stocks/batch"].includes(url.pathname))) {
      unexpected.push(`${req.method()} ${url.pathname}`); return route.abort();
    }
    if (url.pathname === "/api/stocks/batch" && req.method() === "POST") {
      const entry = { body: req.postDataJSON(), headers: req.headers(), result: null as any };
      writes.push(entry);
      const response = await route.fetch();
      expect(response.status()).toBe(200);
      entry.result = await response.json();
      if (loseReply) {
        loseReply = false;
        await new Promise<void>(resolve => release = resolve);
        return route.abort().catch(() => {});
      }
      return route.fulfill({ response });
    }
    return route.continue();
  });
  await page.goto("/#stocks");
  const panel = page.getByRole("region", { name: "批量链上监控" });
  const budget = panel.getByLabel("批量询价金额");
  const recovery = panel.getByRole("alert", { name: "设置操作待核对" });
  await expect(panel).toContainText("已选 2 / 32");
  await budget.fill("12.345");
  await page.reload();
  await expect(budget).toHaveValue("12.345");
  await panel.getByRole("button", { name: "开始批量轮询", exact: true }).click();
  await expect.poll(() => writes[0]?.result?.batch.request.budgetUsdc).toBe("12.345");
  await page.reload(); release();
  await expect(recovery).toContainText("保存股票批量监控结果待核对");
  await expect(budget).toBeDisabled();
  const original = writes[0];
  const newer = { ...original.body.request, enabled: false, budgetUsdc: "77.25", intervalSecs: 60 };
  const changed = await request.post(API + "/api/stocks/batch", { headers: { ...headers,
    "x-request-id": "bp-newer-config", "idempotency-key": "bp-newer-config" },
    data: { request: newer, expectedRevision: original.result.batch.revision } });
  expect(changed.ok()).toBe(true);
  await recovery.getByRole("button", { name: "核对上次操作", exact: true }).click();
  await expect(recovery).toBeHidden();
  await expect(panel.locator(".stock-batch-state")).toHaveText("已暂停");
  await expect(budget).toHaveValue("12.345");
  await expect(budget).toBeEnabled();
  const conflict = panel.getByRole("alert", { name: "批量参数冲突" });
  await expect(conflict).toContainText("77.25 USDC");
  expect((await current()).batch.request).toEqual(newer);
  const replay = await request.post(API + "/api/stocks/batch", { headers: { ...headers,
    "x-request-id": original.headers["x-request-id"], "idempotency-key": original.headers["idempotency-key"] },
    data: { ...original.body, request: { ...original.body.request, budgetUsdc: "999" } } });
  expect(replay.ok()).toBe(true);
  expect(await replay.json()).toEqual(original.result);
  expect((await current()).batch.request).toEqual(newer);
  expect(writes).toHaveLength(1);
  const runs = await request.get(API + "/api/trading/action-runs", { headers });
  const run = (await runs.json()).data.find((row: any) => row.requestId === original.headers["x-request-id"]);
  expect(run).toMatchObject({ kind: "stock_batch_update", status: "succeeded", target: "stocks-batch" });
  expect(run.result.batch.rows).toEqual([]);
  expect(run.result.security).toBeNull();
  const stale = await request.post(API + "/api/stocks/batch", { headers: { ...headers,
    "x-request-id": "bp-stale-client", "idempotency-key": "bp-stale-client" }, data: original.body });
  expect(stale.status()).toBe(409);
  expect((await stale.json()).error.code).toBe("STOCK_BATCH_CHANGED");
  expect((await current()).batch.request).toEqual(newer);
  await conflict.getByRole("button", { name: "保留草稿待应用", exact: true }).click();
  await expect(panel).toContainText("有未应用的更改 · 后台监控仍已暂停");
  await panel.getByRole("button", { name: "开始批量轮询", exact: true }).click();
  await expect.poll(async () => (await current()).batch.request.budgetUsdc).toBe("12.345");
  await panel.getByRole("button", { name: "暂停", exact: true }).click();
  await expect(panel.locator(".stock-batch-state")).toHaveText("已暂停");
  expect(writes).toHaveLength(3);
  // Two independent clients race the same version; exactly one may apply it.
  const beforeRace = await current();
  expect(beforeRace.observedAtMs).toBeGreaterThan(0);
  expect(beforeRace.batch.revision).not.toBe(original.body.expectedRevision);
  const clients = ["15.25", "16.75"].map((budgetUsdc, index) => ({
    key: `bp-client-${index}`, body: { expectedRevision: beforeRace.batch.revision,
      request: { ...beforeRace.batch.request, budgetUsdc } },
  }));
  const outcomes = await Promise.all(clients.map(client => request.post(API + "/api/stocks/batch", {
    headers: { ...headers, "x-request-id": client.key, "idempotency-key": client.key }, data: client.body,
  })));
  expect(outcomes.map(r => r.status()).sort()).toEqual([200, 409]);
  const winner = outcomes.findIndex(r => r.status() === 200);
  const winnerReceipt = await outcomes[winner].json();
  expect((await current()).batch.request).toEqual(clients[winner].body.request);
  expect((await current()).batch.revision).toBe(winnerReceipt.batch.revision);
  expect(errors).toEqual([]); expect(unexpected).toEqual([]);
});
