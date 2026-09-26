import { expect, test } from "@playwright/test";
import { API, NOW, setup } from "./fixtures/opportunity-workbench";

for (const module of ["futures", "opportunities"] as const) {
  test(`${module} bounds first WS snapshot waiting and recovers without REST polling`, async ({ page }) => {
    await page.clock.install({ time: NOW });
    const f = await setup(page, false, module === "futures" ? "silent" : "no-ack");
    let ticketCount = 0;
    let holdTickets = true;
    const heldTickets: (() => void)[] = [];
    await page.route(`${API}/api/auth/ws-ticket`, async (route) => {
      ticketCount++;
      if (holdTickets) await new Promise<void>((resolve) => { heldTickets.push(resolve); });
      await route.fulfill({ response: await route.fetch() });
    });
    await page.route(`${API}/api/v3/arbitrage/opportunities/*/detail?**`, async (route) => {
      const body = await (await route.fetch()).json();
      body.opportunity.id = new URL(route.request().url()).pathname.split("/").at(-2);
      await route.fulfill({ json: body });
    });
    await page.goto(`/#${module}`);
    await expect.poll(() => ticketCount).toBeGreaterThan(0);
    const feed = page.locator(".futures-feed-status");
    const recovery = page.locator(".opportunity-stream-recovery");
    const retry = page.getByRole("button", { name: "重试机会连接", exact: true });
    const rows = page.locator(module === "futures" ? ".futures-data-row" : ".opportunity-table tbody tr[id]");
    const build = page.getByRole("button", { name: module === "futures" ? "构建新双腿" : "构建对冲", exact: true }).first();
    await expect(feed.locator("summary")).toContainText("候选连接中");
    await page.clock.fastForward(5_100);
    await expect(feed.locator("summary")).toContainText("候选读取失败");
    await expect(recovery).toContainText("尚不能判断有无机会");
    await expect(rows).toHaveCount(0);
    await expect(page.locator(".opportunity-eligibility-strip")).toHaveText("候选数量待确认");
    await expect(page.locator(".table-pager strong")).toHaveText("分页数据待确认");
    await expect(retry).toBeEnabled();
    if (module === "opportunities") await expect(page.locator(".scan-kpis strong")).toHaveText(["—", "—", "—"]);
    holdTickets = false;
    await retry.click();
    await expect(retry).toBeDisabled();
    await expect.poll(() => f.sockets.size).toBe(1);
    heldTickets.forEach((release) => release());
    const ticketRequestsAfterRetry = ticketCount;
    await page.clock.fastForward(5_100);
    await expect(retry).toBeEnabled();
    await expect(feed.locator("summary")).toContainText("候选读取失败");
    await expect(rows).toHaveCount(0);
    expect(ticketCount).toBe(ticketRequestsAfterRetry);
    await page.setViewportSize({ width: 390, height: 844 });
    await recovery.scrollIntoViewIfNeeded();
    await expect(retry).toBeInViewport();
    expect(await recovery.evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
    await page.screenshot({ path: test.info().outputPath(`${module}-first-frame-390.png`), fullPage: true });
    await page.setViewportSize({ width: 1440, height: 900 });

    // A received frame without its referenced rows is not recovery or a zero-result market.
    f.streamMode("live");
    f.incompleteStream(true);
    await retry.click();
    await feed.locator("summary").click();
    await expect(feed).toContainText("机会快照不完整");
    await expect(rows).toHaveCount(0);
    await page.clock.fastForward(5_100);
    f.incompleteStream(false);
    await retry.click();
    await expect(rows).toHaveCount(module === "futures" ? 2 : 10);
    await expect(build).toBeEnabled();
    await expect(recovery).toHaveCount(0);
    await expect(feed.locator("summary")).toContainText("候选实时更新");
    await feed.locator("summary").click();

    // Silence retains the last prices, never treats an ACK as a new quote, and replays only this channel.
    await page.clock.fastForward(10_100);
    await expect(recovery).toContainText("保留上次报价");
    await expect(rows).toHaveCount(module === "futures" ? 2 : 10);
    await expect(build).toBeDisabled();
    const connections = f.connections();
    const commands = f.subscriptions.length;
    f.streamMode("silent");
    await retry.click();
    await expect.poll(() => f.subscriptions.length).toBe(commands + 1);
    expect(f.subscriptions.at(-1)).toMatchObject({ channels: ["arbitrage"], replay: true });
    await expect(build).toBeDisabled();
    await page.clock.fastForward(5_100);
    await expect(retry).toBeEnabled();
    f.streamMode("live");
    await retry.click();
    await expect(build).toBeEnabled();
    await expect(recovery).toHaveCount(0);
    expect(f.connections()).toBe(connections);
    expect(f.sockets.size).toBe(1);
    expect(f.listRequests).toEqual([]);
    await page.screenshot({ path: test.info().outputPath(`${module}-stream-recovered.png`), fullPage: true });
    await page.getByRole("tab", { name: "现货跨所", exact: true }).click();
    await expect(rows).toHaveCount(2);
    f.rows.splice(0);
    f.tick();
    if (module === "futures") {
      await page.clock.fastForward(2_600);
      f.tick(2_600);
    }
    await expect(rows).toHaveCount(0);
    await expect(recovery).toHaveCount(0);
    await expect(page.locator(".opportunity-eligibility-count")).toHaveText("显示 0 / 0");
    await expect(page.locator(".table-pager strong")).toHaveText("第 1 / 1 页 · 0 条");
    await page.goto(module === "futures" ? "/#opportunities" : "/#futures");
    await expect(page.locator(".futures-feed-status summary")).toContainText("候选实时更新");
    expect(f.sockets.size).toBe(1);
    expect(f.listRequests).toEqual([]);
    expect(f.errors).toEqual([]);
    expect(f.writes).toEqual([]);
  });
}
