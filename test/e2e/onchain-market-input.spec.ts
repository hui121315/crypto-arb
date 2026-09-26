import { expect, test, type Page, type Route } from "@playwright/test";
import { API, NOW, WEB, setup, snapshot } from "./fixtures/onchain-workbench";

type Pending = { body: Record<string, string>; reply: (json: object) => Promise<void> };

function hold(route: Route, body: Record<string, string>, pending: Pending[]) {
  return new Promise<void>((resolve) => {
    pending.push({ body, reply: async (json) => { await route.fulfill({ json }); resolve(); } });
  });
}

function token(address: string, symbol: string | null, decimals = 6) {
  return { chain: "solana", address, decimals, precisionSource: "solana_mainnet_rpc",
    precisionEvidenceUrl: "https://solana.com/docs/rpc/http/gettokensupply", observedAtMs: NOW,
    identity: symbol ? { chain: "solana", address, symbol, decimals, name: null,
      source: "jupiter_tokens_v2", evidenceUrl: "https://dev.jup.ag/docs/token-api/v2",
      verified: true, native: false, observedAtMs: NOW } : null,
    identityProblem: symbol ? null : "fixture: metadata pending; precision read from RPC" };
}

function catalog(venue: string, base: string, quote = "USDC") {
  return { venue, baseToken: base, observedAtMs: NOW, problem: null,
    pairs: [{ venue, baseToken: base, quoteToken: quote, cexSymbol: `${base}/${quote}`,
      nativeSymbol: `${base}${quote}`, quality: "fresh", source: "ws_push", freshnessMs: 10, observedAtMs: NOW }] };
}

async function nextInput(page: Page, label: string, value: string, pending: Pending[], count: number) {
  await page.getByLabel(label, { exact: true }).fill(value);
  await page.clock.runFor(500);
  await expect.poll(() => pending.length).toBe(count);
}

test("token resolution preserves amounts and rejects obsolete or mismatched identities before saving", async ({ page }, info) => {
  await page.clock.install({ time: NOW });
  const fixture = await setup(page);
  const pending: Pending[] = [];
  const patches: Record<string, unknown>[] = [];
  await page.route(`${API}/api/onchain/token/resolve`, (route) => hold(route, route.request().postDataJSON(), pending));
  await page.route(`${API}/api/onchain/comparison/config`, async (route) => {
    patches.push(route.request().postDataJSON());
    await route.fallback();
  });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto(`${WEB}/#onchain`);
  const quote = snapshot().config.quoteMint;
  const other = snapshot().config.baseMint;
  const amount = page.getByLabel(/^统一对比资金 /);
  const quoteStatus = page.locator(".onchain-token-field").filter({ has: page.getByLabel("Quote 合约 / Mint", { exact: true }) });
  await expect(amount).toHaveValue("100");

  await nextInput(page, "Quote 合约 / Mint", quote, pending, 1);
  await nextInput(page, "Quote 合约 / Mint", other, pending, 2);
  await nextInput(page, "Quote 合约 / Mint", quote, pending, 3);
  await amount.fill("12.345");
  await pending[2].reply(token(quote, null));
  await expect(quoteStatus).toContainText("精度 6 已读取");
  await expect(amount).toHaveValue("12.345");
  await pending[0].reply(token(quote, "OBSOLETE", 9));
  await pending[1].reply(token(other, "WRONG", 9));
  await expect(quoteStatus).not.toContainText("OBSOLETE");
  await expect(quoteStatus).toContainText("精度 6 已读取");

  await page.clock.fastForward(30_001);
  await page.clock.runFor(500);
  await expect.poll(() => pending.length).toBe(4);
  await pending[3].reply(token(quote, "USDC"));
  await expect(quoteStatus).toContainText("registry 已验证");
  await expect(amount).toHaveValue("12.345");
  await page.getByRole("button", { name: "应用变更", exact: true }).click();
  await expect.poll(() => patches.length).toBe(1);
  expect(patches[0]).toMatchObject({ quoteMint: quote, quoteDecimals: 6,
    quoteAmountRaw: "12345000", quoteIdentityResolved: true });
  await expect(page.locator(".onchain-rail-header-tools .read-only-flag")).not.toHaveText("处理中");

  // Solana base58 addresses are case-sensitive, including inside nested metadata.
  const caseChanged = `e${quote.slice(1)}`;
  await nextInput(page, "Quote 合约 / Mint", caseChanged, pending, 5);
  await amount.fill("0.125");
  const mismatch = token(caseChanged, "WRONG");
  mismatch.identity!.address = quote;
  await pending[4].reply(mismatch);
  await expect(quoteStatus).toContainText("识别结果不一致");
  await expect(quoteStatus).not.toContainText("registry 已验证");
  await expect(page.getByRole("button", { name: "应用变更", exact: true })).toBeDisabled();
  await expect(amount).toHaveValue("0.125");

  await nextInput(page, "Quote 合约 / Mint", quote, pending, 6);
  await page.getByRole("button", { name: "清除 Quote 合约", exact: true }).click();
  await pending[5].reply(token(quote, "OBSOLETE"));
  await expect(page.getByLabel("Quote 合约 / Mint", { exact: true })).toBeEmpty();
  await expect(amount).toBeEmpty();
  await nextInput(page, "Quote 合约 / Mint", quote, pending, 7);
  await amount.fill("2.500001");
  await pending[6].reply(token(quote, "USDC"));
  await expect(amount).toHaveValue("2.500001");
  await expect(quoteStatus).toContainText("registry 已验证");
  await expect(page.getByRole("button", { name: "应用变更", exact: true })).toBeEnabled();

  await page.locator("#onchain-config-tab-connectivity").click();
  await page.getByRole("combobox", { name: /^节点模式/ }).selectOption("custom");
  await page.getByLabel(/^RPC URL/).fill("https://first.example.test/rpc");
  await page.clock.runFor(500);
  await expect.poll(() => pending.length).toBe(9);
  await page.getByLabel(/^RPC URL/).fill("https://second.example.test/rpc");
  await page.clock.runFor(500);
  await expect.poll(() => pending.length).toBe(11);
  for (const entry of pending.slice(9, 11)) {
    expect(entry.body.customRpcUrl).toBe("https://second.example.test/rpc");
    await entry.reply(token(entry.body.address, entry.body.address === quote ? "USDC" : "SOL", entry.body.address === quote ? 6 : 9));
  }
  for (const entry of pending.slice(7, 9)) await entry.reply(token(entry.body.address, "OBSOLETE"));
  await page.getByRole("combobox", { name: /^节点模式/ }).selectOption("provider_managed");
  await page.clock.runFor(500);
  await expect.poll(() => pending.length).toBe(13);
  for (const entry of pending.slice(11)) {
    expect(entry.body.customRpcUrl ?? null).toBeNull();
    await entry.reply(token(entry.body.address, entry.body.address === quote ? "USDC" : "SOL", entry.body.address === quote ? 6 : 9));
  }
  await page.locator("#onchain-config-tab-market").click();
  await expect(quoteStatus).toContainText("registry 已验证");
  await expect(page.locator(".onchain-token-grid")).not.toContainText("OBSOLETE");
  await expect(amount).toHaveValue("2.500001");
  await amount.fill("0.0000001");
  await expect(page.getByRole("button", { name: "应用变更", exact: true })).toBeDisabled();
  await amount.fill("2.500001");
  await expect(page.getByRole("button", { name: "应用变更", exact: true })).toBeEnabled();
  await page.getByRole("button", { name: "应用变更", exact: true }).click();
  await expect.poll(() => patches.length).toBe(2);
  expect(patches[1]).toMatchObject({ quoteMint: quote, quoteDecimals: 6,
    quoteAmountRaw: "2500001", quoteIdentityResolved: true });
  // Saving configuration cannot refresh a quote that aged during metadata recovery.
  await expect(page.locator(".onchain-rail-header-tools .read-only-flag")).toHaveText("报价陈旧");
  expect(await amount.evaluate((input: HTMLInputElement) => {
    const style = getComputedStyle(input);
    const canvas = document.createElement("canvas").getContext("2d")!;
    canvas.font = `${style.fontSize} ${style.fontFamily}`;
    return canvas.measureText(input.value).width + parseFloat(style.paddingLeft) + parseFloat(style.paddingRight) + 20 < input.clientWidth;
  })).toBeTruthy();
  await page.getByRole("complementary", { name: "链上套利监控配置" }).screenshot({ path: info.outputPath("token-input-1440.png") });
  await page.setViewportSize({ width: 390, height: 1000 });
  await page.getByRole("navigation", { name: "链上套利工作区" }).getByRole("button", { name: "接入", exact: true }).click();
  await expect(amount).toBeVisible();
  expect(await page.locator(".onchain-page").evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBeTruthy();
  await page.getByRole("complementary", { name: "链上套利监控配置" }).screenshot({ path: info.outputPath("token-input-390.png") });
  expect(patches).toHaveLength(2);
  expect(fixture.requests.filter((request) => request.startsWith("POST") && request.includes("/execution/"))).toEqual([]);
  expect(fixture.errors).toEqual([]);
  expect(fixture.writes).toEqual([]);
});

test("CEX catalog recovers after navigation and retains exact scope and input focus", async ({ page }, info) => {
  await page.clock.install({ time: NOW });
  const fixture = await setup(page);
  const pending: Pending[] = [];
  await page.route(`${API}/api/onchain/cex-pairs**`, (route) =>
    hold(route, Object.fromEntries(new URL(route.request().url()).searchParams), pending));
  await page.goto(`${WEB}/#onchain`);
  await expect.poll(() => pending.length).toBe(1);
  await page.locator('.module-tabs button[data-module="futures"]').click();
  await page.locator('.module-tabs button[data-module="onchain"]').click();
  await expect.poll(() => pending.length).toBe(2);
  const pair = page.getByLabel("交易对（可输入）", { exact: true });
  const options = page.locator("#onchain-cex-pair-options option");
  await pair.focus();
  await pair.evaluate((input: HTMLInputElement) => input.setSelectionRange(0, 3));
  await pending[1].reply(catalog("binance", "SOL"));
  await expect(options).toHaveAttribute("value", "SOL/USDC");
  await expect(pair).toBeFocused();
  expect(await pair.evaluate((input: HTMLInputElement) => [input.selectionStart, input.selectionEnd])).toEqual([0, 3]);
  await pending[0].reply(catalog("binance", "SOL", "OLD"));
  await expect(options).toHaveAttribute("value", "SOL/USDC");

  await pair.fill("PUPS/USD");
  await expect.poll(() => pending.length).toBe(3);
  await pair.fill("TON/USD");
  await expect.poll(() => pending.length).toBe(4);
  await pair.fill("PUPS/USDT");
  await expect.poll(() => pending.length).toBe(5);
  expect(pending[4].body).toMatchObject({ venue: "binance", baseToken: "PUPS" });
  await pending[4].reply(catalog("binance", "PUPS", "USDT"));
  await expect(options).toHaveAttribute("value", "PUPS/USDT");
  await expect(pair).toBeFocused();
  await pending[2].reply(catalog("binance", "PUPS", "OLD"));
  await pending[3].reply(catalog("binance", "TON", "USD"));
  await expect(options).toHaveAttribute("value", "PUPS/USDT");
  await expect(pair).toHaveValue("PUPS/USDT");

  await pair.fill("BTC/USD");
  await expect.poll(() => pending.length).toBe(6);
  const invalid = catalog("binance", "BTC", "USD");
  invalid.pairs[0].venue = "kraken";
  await pending[5].reply(invalid);
  await expect(page.locator(".onchain-pair-evidence-details summary")).toContainText("目录读取失败");
  await expect(options).toHaveCount(0);
  await expect(pair).toHaveValue("BTC/USD");
  await expect(pair).toBeFocused();
  await page.clock.runFor(1_600);
  await expect.poll(() => pending.length).toBe(7);
  await pending[6].reply(catalog("binance", "BTC", "USD"));
  await expect(options).toHaveAttribute("value", "BTC/USD");
  await expect(page.locator(".onchain-pair-evidence-details summary")).toContainText("官方已挂牌");
  await expect(pair).toBeFocused();
  await page.setViewportSize({ width: 390, height: 1000 });
  await page.getByRole("navigation", { name: "链上套利工作区" }).getByRole("button", { name: "接入", exact: true }).click();
  await expect(pair).toBeVisible();
  expect(await page.locator(".onchain-page").evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBeTruthy();
  await page.getByRole("complementary", { name: "链上套利监控配置" }).screenshot({ path: info.outputPath("catalog-restored-390.png") });
  expect(fixture.errors).toEqual([]);
  expect(fixture.writes).toEqual([]);
  expect(fixture.requests.some((request) => request.startsWith("PATCH") || (request.startsWith("POST") && request.includes("/execution/")))).toBeFalsy();
});
