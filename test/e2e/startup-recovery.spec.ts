import { expect, test, type Page } from "@playwright/test";
import { NOW, setup } from "./fixtures/opportunity-workbench";
import { riskFixture } from "./fixtures/settings-risk";

const startup = (page: Page) => page.getByRole("region", { name: /工作台|加载时间较长/ });
const nav = (page: Page) => page.getByRole("navigation", { name: "功能模块", exact: true });
const records = (page: Page) => page.evaluate(() => Object.entries(sessionStorage)
  .filter(([key]) => key.startsWith("crossline.settings.pending.v1:")));

async function capture(page: Page, name: string) {
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    const retry = page.getByRole("button", { name: "重新加载", exact: true });
    await expect(retry).toBeInViewport({ ratio: 1 });
    await retry.click({ trial: true });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    await page.screenshot({ path: test.info().outputPath(`${name}-${width}.png`) });
  }
  await page.setViewportSize({ width: 1440, height: 900 });
}

test("startup catches stylesheet, script and WASM failures and manually recovers the original page", async ({ page }) => {
  await page.clock.install({ time: NOW });
  const f = await setup(page);
  const documents: string[] = [];
  page.on("request", request => {
    if (request.resourceType() === "document") documents.push(request.url());
  });
  const failures = [
    { pattern: /\/[^/]+\.css$/, code: "FRONTEND_STYLE_UNAVAILABLE" },
    { pattern: /\/crypto-arb-frontend-[^/]+\.js$/, code: "FRONTEND_SCRIPT_UNAVAILABLE" },
    { pattern: /\/crypto-arb-frontend-[^/]+_bg\.wasm$/, code: "FRONTEND_WASM_START_FAILED" },
  ];
  for (const [index, failure] of failures.entries()) {
    await test.step(failure.code, async () => {
      let releaseStyle!: () => void;
      const heldStyle = new Promise<void>(resolve => { releaseStyle = resolve; });
      await page.route(failure.pattern, async route => {
        if (index === 0) await heldStyle;
        return route.abort("connectionfailed");
      });
      // Change the query to force a document load, not same-hash navigation.
      await page.goto(`/?startup-check=${index}#positions`, { waitUntil: "commit" });
      if (index === 0) {
        await expect(startup(page)).toHaveAttribute("data-state", "loading");
        await page.clock.fastForward(12001);
        await expect(startup(page)).toHaveAttribute("data-state", "slow");
        await page.getByRole("button", { name: "重新加载", exact: true }).click({ trial: true });
        releaseStyle();
      }
      await expect(startup(page)).toHaveAttribute("data-state", "failed");
      await expect(startup(page)).toContainText(failure.code);
      await expect(startup(page)).toContainText("页面未打开不代表后台任务已停止");
      await expect(nav(page)).toHaveCount(0);
      const loads = documents.length;
      await page.clock.fastForward(15000);
      expect(documents).toHaveLength(loads);
      await expect(startup(page)).toHaveAttribute("data-state", "failed");
      if (index !== 1) await capture(page, failure.code.toLowerCase());
      await page.unroute(failure.pattern);
      await page.getByRole("button", { name: "重新加载", exact: true }).click();
      await expect(page.locator("#crossline-startup")).toHaveCount(0);
      await expect(nav(page)).toHaveCount(1);
      await expect(nav(page).locator('[data-module="positions"]')).toHaveAttribute("aria-current", "page");
      await expect(page).toHaveURL(new RegExp(`\\?startup-check=${index}#positions$`));
      expect(documents).toHaveLength(loads + 1);
    });
  }
  await page.screenshot({ path: test.info().outputPath("positions-recovered.png") });
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("slow startup can finish and failed reload preserves the pending risk request without replay", async ({ page }) => {
  await page.clock.install({ time: NOW });
  const f = await riskFixture(page);
  await page.goto("/#settings");
  const amount = page.getByLabel(/单笔名义上限 USD/);
  const save = page.getByRole("button", { name: "保存风控", exact: true });
  await expect(amount).toHaveValue("10");
  await amount.fill("12.75");
  f.hold();
  await save.click();
  await expect.poll(() => f.calls.length).toBe(1);
  const pending = await records(page);
  expect(pending).toHaveLength(1);
  const run = f.actions.data[0];
  const pattern = /\/crypto-arb-frontend-[^/]+_bg\.wasm$/;
  let releaseAsset!: () => void;
  const held = new Promise<void>(resolve => { releaseAsset = resolve; });
  const requests: string[] = [];
  await page.route(pattern, async route => {
    requests.push(route.request().url());
    await held;
    return route.fallback();
  });
  await page.reload({ waitUntil: "domcontentloaded" });
  await expect(startup(page)).toHaveAttribute("data-state", "loading");
  await expect(page.getByRole("button", { name: "重新加载", exact: true })).toBeEnabled();
  await expect.poll(() => requests.length).toBeGreaterThan(0);
  const attempted = requests.length;
  await page.clock.fastForward(12001);
  await expect(startup(page)).toHaveAttribute("data-state", "slow");
  expect(requests).toHaveLength(attempted);
  expect(await records(page)).toEqual(pending);
  await capture(page, "slow-startup");
  releaseAsset();
  await expect(page.locator("#crossline-startup")).toHaveCount(0);
  await expect(nav(page)).toHaveCount(1);
  const recovery = page.getByRole("alert", { name: "设置操作待核对" });
  await expect(recovery).toContainText("保存风控结果待核对");
  await expect(save).toBeDisabled();

  await page.unroute(pattern);
  await page.route(pattern, route => route.abort("connectionfailed"));
  await page.reload();
  await expect(startup(page)).toHaveAttribute("data-state", "failed");
  expect(await records(page)).toEqual(pending);
  expect(f.calls).toHaveLength(1);
  await page.unroute(pattern);
  await page.getByRole("button", { name: "重新加载", exact: true }).click();
  await expect(recovery).toContainText("保存风控结果待核对");
  await expect(save).toBeDisabled();
  await recovery.getByRole("button").click();
  await expect(recovery).toContainText("后端已受理");
  await expect(save).toBeDisabled();
  expect(f.calls).toHaveLength(1);
  f.release();
  await expect.poll(() => run.status).toBe("succeeded");
  // The original receipt resolves the lock; current state supplies the value.
  f.status.risk.maxOrderNotional = 20;
  await recovery.getByRole("button").click();
  await expect(recovery).toHaveCount(0);
  await expect(amount).toHaveValue("20");
  await expect(save).toBeEnabled();
  expect(await records(page)).toEqual([]);
  expect(f.calls).toHaveLength(1);
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});
