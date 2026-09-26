import { expect, test } from "@playwright/test";
import { executionFixture } from "./fixtures/execution-workbench";

test.setTimeout(15_000);

for (const source of ["futures", "opportunities"] as const) {
  test(`${source} hands off raw prices without rounding, including a sub-display price change`, async ({ page }) => {
    const f = await executionFixture(page);
    const prices = source === "futures"
      ? [0.00000000123456789, 0.00000000123498765]
      : [123.4567890123, 123.4567990123];
    const setPrices = () => {
      for (const [i, leg] of [f.rows[0].longLeg, f.rows[0].shortLeg].entries()) {
        leg.price = prices[i];
        leg.marketEvidence.price = prices[i];
      }
    };
    setPrices();
    const select = async () => {
      if (source === "futures") {
        await page.getByRole("button", { name: "创建交易计划", exact: true }).click();
      } else {
        await page.getByRole("table", { name: "机会扫描候选", exact: true })
          .getByRole("button", { name: "构建对冲", exact: true }).first().click();
      }
    };
    await page.goto(`/#${source}`);
    await select();
    await expect.poll(() => f.previews.length).toBe(1);
    expect(f.previews[0].longPrice).toBe(prices[0]);
    expect(f.previews[0].shortPrice).toBe(prices[1]);
    const longPrice = page.locator(".long-leg").getByLabel("预估参考价", { exact: true });
    const shortPrice = page.locator(".short-leg").getByLabel("预估参考价", { exact: true });
    await expect.poll(async () => Number(await longPrice.inputValue())).toBe(prices[0]);
    await expect.poll(async () => Number(await shortPrice.inputValue())).toBe(prices[1]);
    const manual = prices[0] * 1.000001;
    await longPrice.fill(String(manual));
    await expect.poll(() => f.previews.length).toBe(2);
    expect(f.previews[1].longPrice).toBe(manual);
    await page.locator('.module-tabs [data-module="futures"]').click();
    await page.locator('.module-tabs [data-module="execution"]').click();
    await expect.poll(async () => Number(await longPrice.inputValue())).toBe(manual);
    await expect(page.locator(".execution-artifact-status")).toContainText("待校验");
    const previousCount = f.previews.length;
    prices[0] += source === "futures" ? 0.00000000000000001 : 0.0000000001;
    setPrices();
    await page.locator(`.module-tabs [data-module="${source}"]`).click();
    f.partial(false);
    await select();
    await expect.poll(() => f.previews.length).toBeGreaterThan(previousCount);
    expect(f.previews.at(-1).longPrice).toBe(prices[0]);
    expect(f.previews.at(-1).shortPrice).toBe(prices[1]);
    for (const width of [1440, 1024, 390, 320]) {
      await page.setViewportSize({ width, height: 900 });
      await longPrice.scrollIntoViewIfNeeded();
      expect(await longPrice.evaluate(el => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
      for (const field of await page.locator(".leg-field.readonly strong").all()) {
        expect(await field.evaluate(el => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
      }
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
      await longPrice.click({ trial: true });
      expect(await longPrice.evaluate(el => {
        const r = el.getBoundingClientRect();
        return el.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2));
      })).toBe(true);
      await page.screenshot({ path: test.info().outputPath(`${source}-price-${width}.png`) });
      await page.getByRole("button", { name: "刷新预览", exact: true }).click({ trial: true });
    }
    expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
  });
}

test("cleared or invalid order prices stay editable and cannot silently use another price", async ({ page }) => {
  const f = await executionFixture(page);
  await page.goto("/#futures");
  await page.getByRole("button", { name: "创建交易计划", exact: true }).click();
  await expect(page.locator(".execution-artifact-status")).toContainText("待校验");
  const input = page.locator(".long-leg").getByLabel("预估参考价", { exact: true });
  const submit = page.locator(".confirm-action.primary");
  for (const text of ["", "0", "-1", "not-a-price"]) {
    await input.fill(text);
    await expect(page.locator(".execution-actionbar")).toContainText("买入一边的预估参考价须大于 0");
    await expect(input).toHaveValue(text);
    await expect(submit).toBeDisabled();
  }
  expect(f.previews).toHaveLength(1);
  await input.fill("60000.123456789");
  await expect.poll(() => f.previews.length).toBe(2);
  expect(f.previews[1].longPrice).toBe(60000.123456789);
  await expect(page.locator(".execution-artifact-status")).toContainText("待校验");
  await expect(submit).toBeDisabled();
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});

test("planned order prices come from the bound backend plan, not the editable estimate", async ({ page }) => {
  const f = await executionFixture(page);
  let policy = "limit_price", price: number | null = 60000.12345678;
  f.transformPreview(body => {
    // Preserve the actual response shape, including its ticket binding and both leg identities.
    for (const leg of [body.ticketOrderPlans.long, body.ticketOrderPlans.short]) {
      Object.assign(leg.compilePlan, { payloadPricePolicy: policy, payloadPrice: price });
    }
  });
  await page.goto("/#futures");
  await page.getByRole("button", { name: "创建交易计划", exact: true }).click();
  const leg = page.locator(".long-leg");
  const planned = leg.locator(".leg-field.readonly").filter({ has: page.getByText("计划订单价", { exact: true }) });
  await expect(planned).toContainText("限价 60000.12345678");
  policy = "protection_price"; price = 60000.23456789;
  await leg.getByLabel("预估参考价", { exact: true }).fill("61000");
  await expect(planned).toContainText("保护价 60000.23456789");
  await expect(planned).not.toContainText("61000");
  price = null;
  const refresh = page.getByRole("button", { name: "刷新预览", exact: true });
  await refresh.click();
  await expect(planned).toContainText("订单价格待确认");
  policy = "zero_price"; price = 0;
  await refresh.click();
  await expect(planned).toContainText("不指定价格");
  expect(f.errors).toEqual([]); expect(f.writes).toEqual([]);
});
