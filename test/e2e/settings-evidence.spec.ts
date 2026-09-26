import { expect, test } from "@playwright/test";
import { settingsEvidenceFixture, evidenceRoutes } from "./fixtures/settings-evidence";
import { setup as onchainFixture } from "./fixtures/onchain-workbench";
import { API, NOW } from "./fixtures/opportunity-workbench";

const providerRead = "GET /api/onchain/credentials";
const providerSave = "POST /api/onchain/credentials";
const providerClear = "POST /api/onchain/credentials/clear";
const actionRead = "GET /api/trading/action-runs";

test("provider draft survives refresh and failure, clears only on success and keeps readable state on read failure", async ({ page }) => {
  const f = await settingsEvidenceFixture(page);
  await page.goto("/#settings");
  const form = page.locator(".provider-credentials-body"), input = form.locator("input");
  await input.fill("fixture-api-key");
  f.holdEvidence(providerRead);
  await form.getByRole("button", { name: "刷新凭证状态" }).click();
  await expect.poll(() => f.calls.filter((r) => r.key === providerRead).length).toBe(2);
  await input.focus();
  f.releaseEvidence(providerRead);
  await expect(input).toBeFocused();
  await expect(input).toHaveValue("fixture-api-key");
  f.failEvidence(providerSave, true, 400);
  await form.getByRole("button", { name: "保存新凭证" }).click();
  await expect(form.getByRole("alert")).toContainText("fixture evidence unavailable");
  await expect(input).toHaveValue("fixture-api-key");
  f.failEvidence(providerSave, false); f.holdEvidence(providerSave); f.failEvidence(providerRead);
  await form.getByRole("button", { name: "保存新凭证" }).click();
  await expect(input).toBeDisabled();
  await expect(form.getByRole("tab", { name: "0x", exact: true })).toBeDisabled();
  await expect.poll(() => f.calls.filter((r) => r.key === providerSave).length).toBe(2);
  f.releaseEvidence(providerSave);
  await expect(form.getByRole("status")).toContainText("Jupiter：fixture saved");
  await expect(input).toHaveValue("");
  await expect(form).toContainText("凭证状态刷新失败");
  await expect(page.locator(".provider-credentials-header")).toContainText("陈旧");
  await expect(form.getByRole("button", { name: "清除当前凭证" })).toBeDisabled();
  f.failEvidence(providerRead, false);
  await form.getByRole("button", { name: "重新读取", exact: true }).click();
  await expect(input).toBeEnabled();
  await input.fill("next-fixture-draft");
  await form.getByRole("button", { name: "刷新凭证状态" }).click();
  await expect(input).toHaveValue("next-fixture-draft");
  await form.getByRole("tab", { name: "Backpack", exact: true }).click();
  await expect(form.locator("input")).toHaveCount(2);
  await page.screenshot({ path: test.info().outputPath("provider-desktop.png"), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: test.info().outputPath("provider-mobile.png"), fullPage: true });
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(390);
  await page.reload();
  await expect(form.getByRole("tab", { name: "Backpack", exact: true })).toHaveAttribute("aria-selected", "true");
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});

test("provider initial load failure can retry and clear needs a second click", async ({ page }) => {
  const f = await settingsEvidenceFixture(page);
  f.failEvidence(providerRead);
  await page.goto("/#settings");
  const form = page.locator(".provider-credentials-body");
  await expect(form).toContainText("凭证状态读取失败");
  await expect(form.getByRole("button", { name: "保存新凭证" })).toBeDisabled();
  f.failEvidence(providerRead, false);
  await form.getByRole("button", { name: "重新读取", exact: true }).click();
  await form.getByRole("button", { name: "清除当前凭证" }).click();
  expect(f.calls.filter((r) => r.key === providerClear)).toHaveLength(0);
  await form.getByRole("button", { name: "再次点击确认清除" }).click();
  await expect(form.getByRole("status")).toContainText("fixture cleared");
  await expect(page.locator(".provider-credentials-header")).toContainText("0/1 已配置");
  expect(f.calls.filter((r) => r.key === providerClear)).toHaveLength(1);
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});

for (const op of ["read", "save", "clear"]) test(`provider ${op} response is safe after leaving Settings`, async ({ page }) => {
  const f = await settingsEvidenceFixture(page);
  await page.goto("/#settings");
  const form = page.locator(".provider-credentials-body");
  await expect(form.locator("input")).toBeEnabled();
  const key = op === "read" ? providerRead : op === "save" ? providerSave : providerClear;
  f.holdEvidence(key);
  if (op === "read") await form.getByRole("button", { name: "刷新凭证状态" }).click();
  else if (op === "save") { await form.locator("input").fill("fixture-key"); await form.getByRole("button", { name: "保存新凭证" }).click(); }
  else { await form.getByRole("button", { name: "清除当前凭证" }).click(); await form.getByRole("button", { name: "再次点击确认清除" }).click(); }
  await expect.poll(() => f.calls.filter((r) => r.key === key).length).toBe(op === "read" ? 2 : 1);
  await page.getByRole("tab", { name: "Webhook", exact: true }).click();
  const response = page.waitForResponse((r) => `${r.request().method()} ${new URL(r.url()).pathname}` === key);
  f.releaseEvidence(key); await (await response).finished();
  await page.getByRole("tab", { name: "凭证", exact: true }).click();
  await expect(form.locator("input")).toHaveValue("");
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});

test("action ledger preserves history page, focused row and expanded receipt across updates and failures", async ({ page }) => {
  const f = await settingsEvidenceFixture(page, "action-runs");
  await page.goto("/#settings");
  const table = page.locator(".action-runs-table"), refresh = page.getByRole("button", { name: "刷新", exact: true });
  const first = table.locator('[data-action-id="fixture-action-0"]');
  await expect(first).toContainText("已提交");
  await expect(first.locator(".status-pill")).toHaveClass(/pending/);
  await page.getByRole("button", { name: "下一页", exact: true }).click();
  const row = table.locator('[data-action-id="fixture-action-12"]');
  await row.getByRole("button", { name: "详情" }).click();
  const detail = page.getByRole("region", { name: "动作详情", exact: true });
  await expect(detail).toContainText("fixture receipt 12");
  await expect(detail).not.toContainText("提交结果解码失败");
  await detail.locator("summary").click();
  f.actions.data[12].updatedAtMs = NOW + 60000; f.actions.data[12].message = "fixture updated receipt";
  f.holdEvidence(actionRead);
  await refresh.click();
  await expect.poll(() => f.calls.filter((r) => r.key === actionRead).length).toBe(2);
  await row.getByRole("button", { name: "详情" }).focus();
  f.releaseEvidence(actionRead);
  await expect(row.getByRole("button", { name: "详情" })).toBeFocused();
  await expect(detail).toContainText("fixture updated receipt");
  await expect(detail.locator("details")).toHaveAttribute("open", "");
  await expect(page.locator(".table-pager")).toContainText("第 2 / 2 页");
  f.failEvidence(actionRead); f.failEvidence("GET /api/trading/action-runs/fixture-action-12");
  await refresh.click();
  await expect(page.locator(".settings-workspace")).toContainText("动作账本刷新失败");
  await expect(detail).toContainText("fixture updated receipt");
  await expect(detail.locator("details")).toHaveAttribute("open", "");
  await page.screenshot({ path: test.info().outputPath("ledger-desktop.png"), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: test.info().outputPath("ledger-mobile.png"), fullPage: true });
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(390);
  const box = await row.getByRole("button", { name: "详情" }).boundingBox();
  expect(box!.x + box!.width).toBeLessThanOrEqual(390);
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});

test("action detail ignores an older selection and recovers a failed initial read", async ({ page }) => {
  const f = await settingsEvidenceFixture(page, "action-runs");
  f.failEvidence(actionRead);
  await page.goto("/#settings");
  await expect(page.locator(".settings-workspace")).toContainText("读取动作账本失败");
  f.failEvidence(actionRead, false);
  await page.getByRole("button", { name: "刷新", exact: true }).click();
  const path = "GET /api/trading/action-runs/fixture-action-0";
  f.holdEvidence(path);
  await page.locator('[data-action-id="fixture-action-0"]').getByRole("button").click();
  await expect.poll(() => f.calls.filter((r) => r.key === path).length).toBe(1);
  await page.locator('[data-action-id="fixture-action-1"]').getByRole("button").click();
  const detail = page.getByRole("region", { name: "动作详情", exact: true });
  await expect(detail).toContainText("fixture receipt 1");
  const response = page.waitForResponse((r) => r.url().endsWith("/fixture-action-0"));
  f.releaseEvidence(path); await (await response).finished();
  await expect(detail).toContainText("fixture receipt 1");
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});

test("diagnostics search keeps every typed character and focus; spot query remains read-only after leaving", async ({ page }) => {
  const f = await settingsEvidenceFixture(page, "diagnostics");
  await page.goto("/#settings");
  await page.getByRole("tab", { name: "运行数据依据", exact: true }).click();
  const search = page.getByRole("textbox", { name: "搜索状态" });
  await search.pressSequentially("binance", { delay: 30 });
  await expect(search).toHaveValue("binance"); await expect(search).toBeFocused();
  f.failEvidence("GET /api/system/venue-operation-health");
  await page.getByRole("button", { name: "刷新全部诊断" }).click();
  await expect(page.getByRole("tabpanel", { name: "运行数据依据诊断" })).toContainText("刷新失败");
  await expect(search).toHaveValue("binance");
  await page.getByRole("combobox", { name: "状态过滤" }).selectOption("blocked");
  await page.getByRole("tab", { name: "行情", exact: true }).last().click();
  const symbol = page.getByRole("textbox", { name: "Symbol（可空=全部）" });
  await symbol.fill("SOL");
  const path = "GET /api/v1/spot/ticks";
  f.holdEvidence(path);
  await page.getByRole("button", { name: "查询 Spot Ticks" }).click();
  await expect(symbol).toBeDisabled();
  await expect.poll(() => f.calls.filter((r) => r.key === path).length).toBe(1);
  await page.getByRole("tab", { name: "Webhook", exact: true }).click();
  const response = page.waitForResponse((r) => r.url().includes("/api/v1/spot/ticks"));
  f.releaseEvidence(path); await (await response).finished();
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});

test("shared onchain provider save never clears a sibling wallet draft", async ({ page }) => {
  const base = await onchainFixture(page);
  const f = await evidenceRoutes(page);
  await page.goto("/#onchain");
  await page.getByRole("tab", { name: /^接入/ }).click();
  const signer = page.locator(".onchain-access-credential").filter({ hasText: "当前链签名器" });
  await signer.locator("summary").click();
  await signer.locator("input").fill("fixture-wallet-draft");
  const quote = page.locator(".onchain-access-credential").filter({ hasText: "报价 API" });
  // Change only the local quote-provider draft; no wallet or onchain request is sent.
  await page.getByRole("combobox", { name: "Provider", exact: true }).selectOption("jupiter_swap_v2_keyed");
  await quote.locator("summary").click();
  await quote.locator("input").fill("fixture-quote-key");
  await quote.getByRole("button", { name: "保存新凭证" }).click();
  await expect(quote.locator("input")).toHaveValue("");
  await expect(signer.locator("input")).toHaveValue("fixture-wallet-draft");
  expect(f.calls.filter((r) => r.key === providerSave)).toHaveLength(1);
  expect(base.errors).toEqual([]);
});

test("risk kill switch failure keeps the backend fact and successful retry applies its receipt", async ({ page }) => {
  const f = await settingsEvidenceFixture(page, "risk");
  const status = await (await page.request.get(`${API}/api/trading/status`)).json();
  const calls: { body: any; key: string | undefined }[] = [];
  let fail = true, release: (() => void) | undefined;
  await page.route("**/api/trading/status", (route) => route.fulfill({ json: status }));
  await page.route("**/api/trading/kill-switch", async (route) => {
    const body = route.request().postDataJSON();
    calls.push({ body, key: route.request().headers()["idempotency-key"] });
    if (fail) return route.fulfill({ status: 504, json: { error: { code: "TIMEOUT", message: "fixture unconfirmed" } } });
    await new Promise<void>((resolve) => { release = resolve; });
    status.risk.killSwitchActive = body.active;
    return route.fulfill({ json: { status, summary: { previousActive: body.expectedActive, active: body.active,
      openOrderCount: status.openOrderCount, expectedOpenOrderCount: body.expectedOpenOrderCount, reason: body.reason, checkedAtMs: NOW },
      actionRunId: "fixture-kill", requestId: "fixture-kill-request" } });
  });
  await page.goto("/#settings");
  const button = page.getByRole("button", { name: "切换 Kill Switch" });
  const runtime = page.locator('[data-settings-risk-scope="runtime-readonly"]');
  await expect(runtime).toContainText("Kill Switch 关闭");
  await button.click();
  await expect(page.locator('[data-settings-risk-scope="kill-switch-action"]')).toContainText("更新失败");
  await expect(runtime).toContainText("Kill Switch 关闭");
  fail = false;
  await button.click();
  await expect(page.getByRole("button", { name: "更新中", exact: true })).toBeDisabled();
  await expect(page.getByRole("button", { name: "保存风控" })).toBeDisabled();
  await expect.poll(() => calls.length).toBe(2);
  expect(calls[0].body.expectedActive).toBe(false);
  expect(calls[1].key).toBe(calls[0].key);
  release!();
  await expect(runtime).toContainText("Kill Switch 开启");
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});
