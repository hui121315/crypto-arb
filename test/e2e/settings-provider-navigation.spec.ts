import { expect, test, type Page } from "@playwright/test";
import { evidenceRoutes } from "./fixtures/settings-evidence";
import { setup as onchainFixture } from "./fixtures/onchain-workbench";
import { settingsAccountFixture } from "./fixtures/settings-account";
import { API, NOW } from "./fixtures/opportunity-workbench";

const providerRead = "GET /api/onchain/credentials";
const providerSave = "POST /api/onchain/credentials";
const providerClear = "POST /api/onchain/credentials/clear";

async function reviewRoute(page: Page) {
  const executed = await (await page.request.get(`${API}/api/review/executed`)).json();
  const strategyPerformance = await (await page.request.get(`${API}/api/review/strategy-performance`)).json();
  await page.route("**/api/review/runtime", (route) => route.fulfill({ json: { executed, strategyPerformance, generatedAtMs: NOW } }));
}

async function navigate(page: Page, module: string) {
  await page.evaluate((next) => { location.hash = next; }, module);
  await expect(page.locator(`.module-page.${module === "settings" ? "settings-workspace" : module === "onchain" ? "onchain-page" : "stock-arbitrage-page"}`)).toBeVisible();
}

async function leaveSettings(page: Page) {
  await page.evaluate(() => { location.hash = "review"; });
  await expect(page.locator(".settings-workspace")).toHaveCount(0);
}

test("provider operations stay attached to their target across Settings, onchain and stock editors", async ({ page }) => {
  const base = await onchainFixture(page);
  const f = await evidenceRoutes(page);
  await reviewRoute(page);
  await page.addInitScript(() => {
    localStorage.setItem("crossline.settings.activeTab", JSON.stringify("credentials"));
    localStorage.setItem("crossline.settings.credentialTask", JSON.stringify("onchain-provider"));
  });
  await page.route(/\/api\/stocks(?:\/|$)/, (route) => {
    if (route.request().method() !== "GET") return route.fallback();
    const path = new URL(route.request().url()).pathname;
    return route.fulfill({ json: path === "/api/stocks" ? {
      security: null, tokens: [], books: [], connected: false, reference: null, problem: null, observedAtMs: NOW,
    } : { rows: [], recoveryProblem: null, observedAtMs: NOW } });
  });
  await page.clock.install({ time: NOW });
  await page.goto("/#settings");
  const form = page.locator(".provider-credentials-body");
  await expect(form.locator("input")).toBeEnabled();
  const initialReads = f.calls.filter((r) => r.key === providerRead).length;
  f.holdEvidence(providerRead); f.failEvidence(providerRead);
  await form.getByRole("button", { name: "刷新凭证状态" }).click();
  await expect.poll(() => f.calls.filter((r) => r.key === providerRead).length).toBe(initialReads + 1);
  await leaveSettings(page);
  f.failEvidence(providerRead, false);
  await navigate(page, "settings");
  await expect(form.locator("input")).toBeEnabled();
  await expect(form.getByRole("button", { name: "刷新凭证状态" })).toBeEnabled();
  const late = page.waitForResponse((r) => r.url().endsWith("/onchain/credentials") && r.status() === 503);
  f.releaseEvidence(providerRead); await (await late).finished();
  await expect(form.getByRole("alert")).toHaveCount(0);
  await form.locator("input").fill("fixture-provider-secret");
  f.holdEvidence(providerSave); f.failEvidence(providerSave, true, 400);
  await form.getByRole("button", { name: "保存新凭证" }).click();
  await expect.poll(() => f.calls.filter((r) => r.key === providerSave).length).toBe(1);
  await leaveSettings(page); await navigate(page, "settings");
  await expect(form.getByRole("button", { name: "处理中…", exact: true })).toBeDisabled();
  await expect(form.locator("input")).toHaveValue("");
  await expect(form.locator("input")).toBeDisabled();
  await expect(page.getByRole("tab", { name: "Backpack", exact: true })).toBeDisabled();
  f.releaseEvidence(providerSave);
  await expect(form.getByRole("alert")).toContainText("fixture evidence unavailable");
  await expect(form).toContainText("离页已清空");
  f.failEvidence(providerSave, false); f.holdEvidence(providerSave);
  await form.locator("input").fill("fixture-provider-secret");
  await expect(form).not.toContainText("离页已清空");
  await form.getByRole("button", { name: "保存新凭证" }).click();
  await navigate(page, "onchain");
  await page.getByRole("tab", { name: /^接入/ }).click();
  await page.getByRole("combobox", { name: "Provider", exact: true }).selectOption("jupiter_swap_v2_keyed");
  const quote = page.locator(".onchain-access-credential").filter({ hasText: "报价 API" });
  const signer = page.locator(".onchain-access-credential").filter({ hasText: "当前链签名器" });
  await quote.locator("summary").click();
  await signer.locator("summary").click();
  await expect(quote.locator("input")).toBeDisabled();
  await expect(signer.locator("input")).toBeDisabled();
  f.releaseEvidence(providerSave);
  await expect(page.locator(".onchain-access-credentials").getByRole("status")).toContainText("Jupiter：fixture saved");
  await expect(quote.locator("input")).toBeEnabled();
  await signer.locator("input").fill("fixture-wallet-draft");
  await quote.locator("input").fill("fixture-next-key");
  await quote.getByRole("button", { name: "保存新凭证" }).click();
  await expect(quote.locator("input")).toHaveValue("");
  await expect(signer.locator("input")).toHaveValue("fixture-wallet-draft");
  await navigate(page, "settings");
  await page.getByRole("tab", { name: "Backpack", exact: true }).click();
  await expect(form.getByRole("status")).toHaveCount(0);
  await form.locator("input").first().fill("fixture-backpack-key");
  f.holdEvidence(providerSave);
  await form.getByRole("button", { name: "保存新凭证" }).click();
  await expect.poll(() => f.calls.filter((r) => r.key === providerSave).length).toBe(4);
  await navigate(page, "stocks");
  await page.getByRole("button", { name: "询价与执行", exact: true }).click();
  const stock = page.locator(".stock-arbitrage-page .provider-credentials");
  await stock.locator("summary").first().click();
  await expect(stock.getByRole("button", { name: "处理中…", exact: true })).toBeDisabled();
  f.releaseEvidence(providerSave);
  await expect(stock.getByRole("status")).toContainText("Backpack：fixture saved");
  await navigate(page, "settings");
  await expect(form.locator("input").first()).toBeEnabled();
  await form.getByRole("button", { name: "清除当前凭证" }).click();
  await leaveSettings(page); await navigate(page, "settings");
  await expect(form.getByRole("button", { name: "清除当前凭证" })).toBeEnabled();
  expect(f.calls.filter((r) => r.key === providerClear)).toHaveLength(0);
  await form.getByRole("button", { name: "清除当前凭证" }).click();
  f.holdEvidence(providerClear);
  await form.getByRole("button", { name: "再次点击确认清除" }).click();
  await leaveSettings(page); await navigate(page, "settings");
  await expect(form.getByRole("button", { name: "处理中…", exact: true })).toBeDisabled();
  f.releaseEvidence(providerClear);
  await expect(form.getByRole("status")).toContainText("fixture cleared");
  await expect(page.locator(".provider-credentials-header")).toContainText("0/2 已配置");
  await leaveSettings(page);
  const after = f.calls.filter((r) => r.key === providerRead).length;
  await page.clock.runFor(25_000);
  expect(f.calls.filter((r) => r.key === providerRead)).toHaveLength(after);
  await navigate(page, "settings");
  await page.screenshot({ path: test.info().outputPath("provider-desktop.png") });
  await page.setViewportSize({ width: 390, height: 844 });
  await form.getByRole("button", { name: "保存新凭证" }).scrollIntoViewIfNeeded();
  await expect(form.getByRole("button", { name: "保存新凭证" })).toBeInViewport();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
  await page.screenshot({ path: test.info().outputPath("provider-mobile.png") });
  expect(await page.evaluate(() => JSON.stringify({ ...localStorage, ...sessionStorage }))).not.toContain("fixture-provider-secret");
  expect(f.calls.filter((r) => r.key === providerSave).map((r) => r.body.provider)).toEqual([
    "jupiter_swap_v2_keyed", "jupiter_swap_v2_keyed", "jupiter_swap_v2_keyed", "backpack_stocks",
  ]);
  expect(base.errors).toEqual([]); expect(base.writes).toEqual([]);
});

test("provider recovery bounds reads and resolves only the original mutation without resending secrets", async ({ page }) => {
  const base = await onchainFixture(page);
  const f = await evidenceRoutes(page, true);
  await reviewRoute(page);
  await page.addInitScript(() => {
    localStorage.setItem("crossline.settings.activeTab", JSON.stringify("credentials"));
    localStorage.setItem("crossline.settings.credentialTask", JSON.stringify("onchain-provider"));
  });
  await page.clock.install({ time: NOW });
  await page.goto("/#settings");
  const form = page.locator(".provider-credentials-body");
  const refresh = form.getByRole("button", { name: "刷新凭证状态" });
  const recovery = form.getByRole("alert", { name: "凭证结果待核对" });
  const recheck = recovery.getByRole("button", { name: "核对上次操作" });
  await expect(form.locator("input")).toBeEnabled();
  const initialReads = f.calls.filter((r) => r.key === providerRead).length;
  f.holdEvidence(providerRead);
  await refresh.click();
  await expect.poll(() => f.calls.filter((r) => r.key === providerRead).length).toBe(initialReads + 1);
  await page.clock.runFor(10_001);
  await expect(form).toContainText("超过 10 秒未响应");
  await expect(refresh).toBeEnabled();
  f.releaseEvidence(providerRead);
  await refresh.click();
  await expect(form.getByRole("alert")).toHaveCount(0);

  await form.locator("input").fill("fixture-recovery-secret");
  f.holdEvidence(providerSave);
  await form.getByRole("button", { name: "保存新凭证" }).click();
  await expect.poll(() => f.calls.filter((r) => r.key === providerSave).length).toBe(1);
  await page.clock.runFor(20_001);
  await expect(recovery).toContainText("MUTATION_TIMEOUT");
  await expect(recheck).toBeEnabled();
  await expect(form.locator("input")).toBeDisabled();
  await expect(form.getByRole("button", { name: "清除当前凭证" })).toBeDisabled();
  await leaveSettings(page); await navigate(page, "settings");
  await expect(form.locator("input")).toHaveValue("");
  await expect(recheck).toBeEnabled();

  const run = f.actions.data.shift();
  expect(run.kind).toBe("onchain_provider_credentials_update");
  expect(run.requestId).toBeTruthy(); expect(run.idempotencyKey).toBeTruthy();
  await recheck.click();
  await expect(recovery).toContainText("PROVIDER_RECEIPT_NOT_FOUND");
  await expect(form.locator("input")).toBeDisabled();
  f.actions.data.unshift(run);
  await recheck.click();
  await expect(recovery).toContainText("后端已受理，尚未完成");
  const detail = `GET /api/trading/action-runs/${run.id}`;
  f.holdEvidence(detail);
  await recheck.click();
  await expect.poll(() => f.calls.filter((r) => r.key === detail).length).toBe(1);
  await page.clock.runFor(10_001);
  await expect(recovery).toContainText("读取原凭证处理结果超过 10 秒");
  await expect(recheck).toBeEnabled();
  f.releaseEvidence(detail);
  run.target = "backpack_stocks";
  await recheck.click();
  await expect(recovery).toContainText("PROVIDER_RECEIPT_MISMATCH");
  await expect(form.locator("input")).toBeDisabled();
  await page.screenshot({ path: test.info().outputPath("provider-recovery-desktop.png") });
  await page.setViewportSize({ width: 390, height: 844 });
  await recheck.scrollIntoViewIfNeeded();
  await expect(recheck).toBeInViewport();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
  await page.screenshot({ path: test.info().outputPath("provider-recovery-mobile.png") });
  await page.setViewportSize({ width: 1280, height: 720 });
  run.target = "jupiter_swap_v2_keyed";
  f.releaseEvidence(providerSave);
  await expect.poll(() => run.status).toBe("succeeded");
  // A late HTTP success is not completion after the local timeout; verify its receipt.
  await expect(recovery).toBeVisible();
  run.result.provider = "backpack_stocks";
  await recheck.click();
  await expect(recovery).toContainText("PROVIDER_RECEIPT_MISMATCH");
  run.result.provider = "jupiter_swap_v2_keyed";
  await recheck.click();
  await expect(recovery).toHaveCount(0);
  await expect(form.getByRole("status")).toContainText("fixture saved");
  await expect(form.locator("input")).toBeEnabled();

  f.failEvidence(providerClear);
  await form.getByRole("button", { name: "清除当前凭证" }).click();
  await form.getByRole("button", { name: "再次点击确认清除" }).click();
  await expect(recovery).toContainText("EVIDENCE_FIXTURE_FAILED");
  await recheck.click();
  await expect(recovery).toHaveCount(0);
  await expect(form.getByRole("alert")).toContainText("原操作已确认失败");
  await expect(form.locator("input")).toBeEnabled();
  await expect(page.locator(".provider-credentials-header")).toContainText("配置完整");
  await expect(form).toContainText("已保存 1 项，无缺失必填字段");
  expect(f.calls.filter((r) => r.key === providerSave)).toHaveLength(1);
  expect(f.calls.filter((r) => r.key === providerClear)).toHaveLength(1);
  expect(JSON.stringify(f.actions)).not.toContain("fixture-recovery-secret");
  expect(await page.evaluate(() => JSON.stringify({ ...localStorage, ...sessionStorage }))).not.toContain("fixture-recovery-secret");
  expect(base.errors).toEqual([]); expect(base.writes).toEqual([]);
});

test("connection probes preserve their target, reject obsolete auth results and recover native timeout without resubmitting", async ({ page }) => {
  const f = await settingsAccountFixture(page, "diagnostics");
  await reviewRoute(page);
  const health = await (await page.request.get(`${API}/api/system/health`)).json();
  health.data.apiVersion = "2.2.0-rc.2";
  let release!: () => void, hold = true;
  const probes: string[] = [];
  await page.route(`${API}/candidate/**`, async (route) => {
    probes.push(route.request().url());
    if (route.request().url().endsWith("/system/health")) {
      if (hold) { hold = false; await new Promise<void>((resolve) => { release = resolve; }); }
      return route.fulfill({ json: health });
    }
    return route.fulfill({ json: { ticket: "fixture-probe", expiresAtMs: NOW + 60_000 } });
  });
  await page.clock.install({ time: NOW });
  await page.goto("/#settings");
  const input = page.getByRole("textbox", { name: "API Base", exact: true });
  const probe = page.getByRole("button", { name: "验证连通" });
  const feedback = page.locator(".settings-api-task").first().locator(".settings-api-feedback");
  await input.fill(`${API}/candidate`);
  await probe.click();
  await expect.poll(() => probes.length).toBe(1);
  await leaveSettings(page); await navigate(page, "settings");
  await expect(input).toHaveValue(`${API}/candidate`);
  await expect(input).toBeDisabled();
  await expect(probe).toBeDisabled();
  release();
  await expect(feedback).toContainText("API Base 可达");
  await expect(feedback).toContainText(`${API}/candidate`);
  await expect(input).toBeEnabled();
  hold = true;
  await probe.click();
  await expect.poll(() => probes.length).toBe(3);
  await page.getByLabel("Token", { exact: true }).fill("fixture-updated-auth");
  await page.getByRole("button", { name: "保存 Token", exact: true }).click();
  await expect(input).toBeEnabled();
  const obsolete = page.waitForResponse((r) => r.url().endsWith("/candidate/api/auth/ws-ticket"));
  release(); await (await obsolete).finished();
  await expect(feedback).not.toContainText("API Base 可达");
  await probe.click();
  await expect(feedback).toContainText("API Base 可达");
  await input.fill(API);
  await expect(feedback).not.toContainText("API Base 可达");
  await page.screenshot({ path: test.info().outputPath("diagnostics-desktop.png") });
  await page.setViewportSize({ width: 390, height: 844 });
  await probe.scrollIntoViewIfNeeded();
  await expect(probe).toBeInViewport();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
  await page.screenshot({ path: test.info().outputPath("diagnostics-mobile.png") });
  await page.setViewportSize({ width: 1280, height: 720 });

  // This is the actual 20-second frontend timeout, not a forged HTTP TIMEOUT response.
  await page.getByRole("tab", { name: "执行环境", exact: true }).click();
  const path = "POST /api/trading/adapters/select";
  f.holdAccount(path);
  await page.getByRole("button", { name: "启用实盘", exact: true }).click();
  await page.getByRole("button", { name: "确认启用实盘" }).click();
  await expect.poll(() => f.calls.filter((r) => r.key === path).length).toBe(1);
  await page.clock.runFor(20_001);
  await expect(page.getByRole("button", { name: "核对上次切换" })).toBeEnabled();
  await expect(page.locator(".settings-content-panel")).toContainText("MUTATION_TIMEOUT");
  f.releaseAccount(path);
  await leaveSettings(page); await navigate(page, "settings");
  await page.getByRole("button", { name: "核对上次切换" }).click();
  await expect(page.getByRole("group", { name: "执行环境", exact: true })).toContainText("实盘");
  const attempts = f.calls.filter((r) => r.key === path);
  expect(attempts).toHaveLength(1);
  expect(attempts[0].idempotency).toBeTruthy();
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});
