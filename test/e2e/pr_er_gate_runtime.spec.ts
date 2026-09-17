import { expect, test, type Locator, type Page } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";
const SCENARIO_BASE = `${API_BASE}/e2e-pr-er-gate-runtime`;

async function usePrErGateRuntime(page: Page) {
  await page.addInitScript((apiBase) => {
    window.localStorage.setItem("api_base", JSON.stringify(apiBase));
    window.localStorage.setItem("api_auth_token", JSON.stringify("e2e-token"));
  }, SCENARIO_BASE);
}

async function openGateDiagnostics(page: Page) {
  const operationHealth = page.waitForResponse((response) =>
    response.url().includes("/e2e-pr-er-gate-runtime/api/system/venue-operation-health")
    && response.status() === 200,
  );
  await page.goto("/#settings");
  await expect(page.locator("h1", { hasText: "设置" })).toBeVisible();
  await page.getByRole("tab", { name: "诊断" }).click();
  await operationHealth;
  return page.locator("table.settings-table").filter({
    has: page.getByRole("columnheader", { name: "当前可用" }),
  });
}

async function findHealthRow(page: Page, table: Locator, query: string) {
  await page.getByLabel("搜索状态").fill(query);
  const row = table.locator("tbody tr").filter({ hasText: "gate" });
  await expect(row).toHaveCount(1);
  return row;
}

test.beforeEach(async ({ page }) => {
  await usePrErGateRuntime(page);
});

test("PR-ER Gate diagnostics exposes native contract evidence without implying live credentials", async ({
  page,
}) => {
  const table = await openGateDiagnostics(page);
  const row = await findHealthRow(page, table, "req-pr-er-gate-native-contract");

  await expect(row).toContainText("REST 合约元数据");
  await expect(row).toContainText("正常");
  await expect(row).toContainText("配置存在");
  await expect(row).toContainText("支持");
  await expect(row).toContainText("可用");
  await expect(row.locator("td").last()).toHaveAttribute("title", /settle=usdt/);
  await expect(row.locator("td").last()).toHaveAttribute("title", /native_symbol=BTC_USDT/);
  await expect(row.locator("td").last()).toHaveAttribute("title", /contract_size=0\.0001/);
  await expect(row.locator("td").last()).toHaveAttribute("title", /quanto_multiplier=0\.0001/);
  await expect(row.locator("td").last()).toHaveAttribute("title", /fixture gate\/futures_usdt_contracts_btc_usdt\.json/);
  await expect(row.locator("td").last()).toHaveAttribute("title", /parser official_fixture_closes_native_identity_and_contract_spec/);
  await expect(row.locator("td").last()).toHaveAttribute("title", /builder contract_list_request_binds_the_verified_settle_path/);
  await expect(row.locator("td").last()).toHaveAttribute("title", /credentials_present=false/);
  await expect(row.locator("td").last()).toHaveAttribute("title", /not_real_exchange_live_sample=true/);
});

test("PR-ER Gate private runtime evidence stays unavailable when credentials and live samples are absent", async ({
  page,
}) => {
  const table = await openGateDiagnostics(page);

  const finality = await findHealthRow(page, table, "req-pr-er-gate-finality");
  await expect(finality).toContainText("订单终态回查");
  await expect(finality).toContainText("待验证");
  await expect(finality).toContainText("配置缺失");
  await expect(finality).toContainText("不可用");
  await expect(finality).toContainText("GATE_ORDER_FINALITY_EVIDENCE_UNAVAILABLE");
  await expect(finality.locator("td").last()).toHaveAttribute("title", /live_order_query_sample=missing/);
  await expect(finality.locator("td").last()).toHaveAttribute("title", /fixture gate\/futures_usdt_get_order_ioc\.json/);
  await expect(finality.locator("td").last()).toHaveAttribute("title", /parser gate_get_order_ioc_fixture_preserves_partial_fill_as_terminal_cancel/);

  const privateWs = await findHealthRow(page, table, "req-pr-er-gate-private-ws");
  await expect(privateWs).toContainText("私有订单流");
  await expect(privateWs).toContainText("阻断");
  await expect(privateWs).toContainText("配置缺失");
  await expect(privateWs).toContainText("不可用");
  await expect(privateWs.locator("td").last()).toHaveAttribute("title", /channel=futures\.orders/);
  await expect(privateWs.locator("td").last()).toHaveAttribute("title", /credentials_present=false/);
  await expect(privateWs.locator("td").last()).toHaveAttribute("title", /parser ws_order_update_accepts_known_open_gtc/);
  await expect(privateWs.locator("td").last()).toHaveAttribute("title", /builder subscribe_payload_matches_gate_authenticated_schema/);

  const fee = await findHealthRow(page, table, "req-pr-er-gate-fee");
  await expect(fee).toContainText("待验证");
  await expect(fee).toContainText("不可用");
  await expect(fee.locator("td").last()).toHaveAttribute("title", /fee_evidence=missing/);
  await expect(fee.locator("td").last()).toHaveAttribute("title", /zero_fee_assumption=false/);
  await expect(fee.locator("td").last()).toHaveAttribute("title", /fixture gate\/futures_usdt_my_trades_order\.json/);
  await expect(fee.locator("td").last()).toHaveAttribute("title", /parser parses_official_my_trades_fixture_without_combining_fee_units/);

  const maintenance = await findHealthRow(page, table, "req-pr-er-gate-maintenance");
  await expect(maintenance).toContainText("待验证");
  await expect(maintenance).toContainText("不可用");
  await expect(maintenance.locator("td").last()).toHaveAttribute("title", /maintenance_evidence=missing/);
  await expect(maintenance.locator("td").last()).toHaveAttribute("title", /margin_utilization_estimated=true/);
  await expect(maintenance.locator("td").last()).toHaveAttribute("title", /fixture gate\/futures_usdt_positions\.json/);
  await expect(maintenance.locator("td").last()).toHaveAttribute("title", /parser parse_positions_maps_official_maintenance_rate/);
});

test("PR-ER Gate AccountState renders missing fee and maintenance evidence fail closed", async ({
  page,
}) => {
  const snapshot = page.waitForResponse((response) =>
    response.url().includes("/e2e-pr-er-gate-runtime/api/trading/portfolio/snapshot")
    && response.status() === 200,
  );
  await page.goto("/#positions");
  await expect(page.getByRole("heading", { name: "持仓/风控" })).toBeVisible();
  await snapshot;

  const layout = page.locator(".positions-layout");
  await expect(layout).toContainText("GATE_ACCOUNT_EVIDENCE_INCOMPLETE");
  await expect(layout).toContainText("Gate fee and maintenance evidence is incomplete");

  const setup = page.locator(".account-setup");
  await expect(setup).toContainText("尚未配置交易所 API 凭证");
  await expect(setup).toContainText("API Key + Secret");
  await expect(setup).toContainText("读取余额、持仓和订单");
  await expect(setup).toContainText("待配置：GATE");
  await expect(page.getByText("余额等待账户接入", { exact: true })).toBeVisible();
  await expect(page.locator(".risk-panel")).toContainText("风险指标等待账户接入");
  await expect(page.locator(".balance-panel .balance-evidence-chip")).toHaveCount(0);
  await expect(page.locator(".risk-panel .balance-evidence-chip")).toHaveCount(0);

  const unverified = page.locator(".runtime-problems");
  await expect(unverified).toHaveAttribute("title", /req-pr-er-gate-account-evidence/);
  await expect(unverified).toHaveAttribute("title", /GATE_ACCOUNT_EVIDENCE_INCOMPLETE/);
  await expect(unverified).toHaveAttribute("title", /feeEvidence.*missing/);
  await expect(unverified).toHaveAttribute("title", /maintenanceEvidence.*missing/);
  await expect(unverified).not.toHaveAttribute("title", /hmac-sha256|credentialFingerprint/);
});
