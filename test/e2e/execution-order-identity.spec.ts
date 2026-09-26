import { expect, test } from "@playwright/test";
import { submissionFixture, reviewAndSubmit } from "./fixtures/execution-submission";
import { API, NOW } from "./fixtures/opportunity-workbench";

function identify(order: any) {
  const id = order.intent.id;
  order.intent.clientOrderId = `public-${id}`;
  order.identity = {
    internalOrderId: id, publicClientOrderId: order.intent.clientOrderId,
    venueClientOrderId: `native-${id}`, exchangeOrderId: `exchange-${id}`,
    product: "perp", accountScope: `fixture-${order.intent.exchange}`,
  };
  return order;
}

function attach(leg: any, order: any) {
  leg.identity = order.identity;
  leg.orderIds = ids(order);
}

const ids = (order: any) => [order.identity.internalOrderId, order.identity.publicClientOrderId,
  order.identity.venueClientOrderId, order.identity.exchangeOrderId];

test("order aliases count and load once while preserving separate recovery orders", async ({ page }, info) => {
  const f = await submissionFixture(page);
  const run: any = f.makeRun({ idempotencyKey: "alias-history", ticketId: "alias-ticket" });
  const long = identify(f.makeOrder(run.longLeg.orderIds[0], "filled", NOW + 10));
  const short = identify(f.makeOrder(run.shortLeg.orderIds[0], "accepted", NOW + 10));
  const unwind = identify({ ...f.makeOrder("separate-recovery-long", "accepted", NOW + 10),
    intent: { ...long.intent, id: "separate-recovery-long", side: "sell", reduceOnly: true } });
  attach(run.longLeg, long); attach(run.shortLeg, short);
  run.longLeg.orderIds.push(...ids(unwind));
  run.state = "unwinding";
  f.setRuns([run]); f.setOrders([long, unwind]);
  const reads: string[] = [];
  await page.route(`${API}/api/trading/orders/*`, route => {
    const id = decodeURIComponent(new URL(route.request().url()).pathname.split("/").at(-1)!);
    reads.push(id);
    return id === short.intent.id ? route.fulfill({ json: short })
      : route.fulfill({ status: 404, json: { error: { code: "NOT_FOUND", message: "not an internal order id" } } });
  });
  await page.goto(`/#execution?${new URLSearchParams({ run: run.runId, ticket: run.ticketId, opp: run.opportunityId })}`);
  const orders = page.locator(".execution-idle-history");
  await expect(orders).toContainText("明细 3 / 3");
  await expect(orders).not.toContainText("订单明细读取失败");
  expect(reads).toEqual([short.intent.id]);
  await expect(orders.locator('summary[title="展开订单详情"]')).toHaveCount(3);
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    await page.screenshot({ path: info.outputPath(`order-identities-${width}.png`), fullPage: true });
  }
  expect(f.confirms).toEqual([]); expect(f.cancels).toEqual([]);
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});

test("cancel sends one internal id per order and keeps partial fills for position handoff", async ({ page }) => {
  const f = await submissionFixture(page);
  await page.route(`${API}/api/trading/orders/*`, route => {
    if (route.request().method() !== "GET") return route.fallback();
    const id = decodeURIComponent(new URL(route.request().url()).pathname.split("/").at(-1)!);
    const key = f.confirms.at(-1)?.idempotencyKey;
    return [ `${key}-long`, `${key}-short` ].includes(id)
      ? route.fulfill({ json: identify(f.makeOrder(id, "accepted", NOW + 10)) })
      : route.fulfill({ status: 404, json: { error: { code: "NOT_FOUND", message: "not an internal order id" } } });
  });
  await reviewAndSubmit(page);
  const run: any = f.makeRun(f.confirms[0], "second_leg_submitted", NOW + 30);
  const long = identify(f.makeOrder(run.longLeg.orderIds[0], "accepted", NOW + 30));
  const short = identify(f.makeOrder(run.shortLeg.orderIds[0], "partially_filled", NOW + 30));
  short.filledQuantity = 0.4;
  attach(run.longLeg, long); attach(run.shortLeg, short);
  f.emitRecord(long); f.emitRecord(short); f.emitRun(run);
  await expect(page.locator(".execution-order-queue")).toContainText("明细 2 / 2");
  // A cancellation ACK is not a final result, even though all aliases refer to the same order.
  f.setCancelState("cancel_requested");
  await page.getByRole("button", { name: "撤单", exact: true }).click();
  await expect.poll(() => f.cancels.length).toBe(2);
  expect(f.cancels).toEqual([long.intent.id, short.intent.id]);
  await expect(page.getByRole("button", { name: "撤单待确认", exact: true })).toBeDisabled();
  const saved = await page.evaluate(() => Object.entries(sessionStorage)
    .find(([key]) => key.startsWith("crossline.execution.pendingCancel.v1:"))!);
  const completedLong = { ...long, state: "cancelled", filledQuantity: 0, updatedAtMs: NOW + 50 };
  const completedShort = { ...short, state: "cancelled", updatedAtMs: NOW + 50 };
  f.emitRecord(completedLong); f.emitRecord(completedShort);
  await expect(page.getByRole("status", { name: "撤单处理提示" })).toContainText("已有成交；撤单不等于平仓");
  await expect(page.getByRole("link", { name: "去持仓平仓", exact: true })).toHaveAttribute("href",
    `#positions?${new URLSearchParams({ run: run.runId, ticket: run.ticketId, opp: run.opportunityId })}`);
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    const notice = page.getByRole("status", { name: "撤单处理提示" });
    await notice.scrollIntoViewIfNeeded();
    expect(await notice.evaluate(el => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    await page.screenshot({ path: test.info().outputPath(`partial-cancel-${width}.png`) });
  }
  expect(f.cancels).toEqual([long.intent.id, short.intent.id]);
  // Recover a batch persisted by the old UI. Keep request IDs; only resolve aliases
  // backed by this batch's exact internal order and the same venue/symbol.
  const oldBatch = JSON.parse(saved[1]);
  oldBatch.requests = oldBatch.requests.flatMap((request: any, index: number) => {
    const order = index === 0 ? long : short;
    return ids(order).map((id, aliasIndex) => ({ ...request, order_id: id,
      context: { request_id: `fixture-old-cancel-${index}-${aliasIndex}`,
        idempotency_key: `execution-cancel:${run.runId}:${id}:fixture-old-attempt` } }));
  });
  const wrongScope = structuredClone(oldBatch);
  wrongScope.requests[1].exchange = "wrong-venue";
  await page.evaluate(([key, value]) => sessionStorage.setItem(key, value), [saved[0], JSON.stringify(wrongScope)]);
  await page.reload();
  const recovery = page.getByRole("region", { name: "原撤单核对" });
  await expect(recovery).toContainText("原撤单结果待核对");
  expect(f.cancels).toHaveLength(2);
  await page.evaluate(([key, value]) => sessionStorage.setItem(key, value), [saved[0], JSON.stringify(oldBatch)]);
  await page.reload();
  await expect(recovery).toContainText("1 笔已有成交");
  await expect(recovery).toContainText("撤单不等于平仓");
  await expect.poll(() => page.evaluate(key => sessionStorage.getItem(key), saved[0])).toBe(null);
  expect(f.cancels).toEqual([long.intent.id, short.intent.id]);
  expect(f.confirms).toHaveLength(1);
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});
