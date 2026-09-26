import { expect, test, type Page, type WebSocketRoute } from "@playwright/test";
import { API, NOW, setup } from "./fixtures/opportunity-workbench";
import { settingsFixture } from "./fixtures/settings-workbench";

const appWs = (page: Page) => page.getByTestId("status-app-ws");
const settle = (page: Page) => page.evaluate(() => new Promise<void>(resolve =>
  requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));

async function socketFixture(page: Page) {
  const health = await (await page.request.get(`${API}/api/system/health`)).json();
  const sockets: { socket: WebSocketRoute; channels: Set<string>; closed: boolean }[] = [];
  let version = 0;
  await page.routeWebSocket(/.*/, socket => {
    if (!socket.url().startsWith(API.replace("http:", "ws:"))) return socket.close();
    const entry = { socket, channels: new Set<string>(), closed: false };
    sockets.push(entry);
    socket.onClose(() => { entry.closed = true; });
    socket.onMessage(raw => {
      const message = JSON.parse(raw.toString());
      if (message.type === "subscribe") {
        message.channels.forEach((channel: string) => entry.channels.add(channel));
        socket.send(JSON.stringify({ type: "ack", subscribed: message.channels, requestId: message.requestId }));
      } else if (message.type === "unsubscribe") {
        message.channels.forEach((channel: string) => entry.channels.delete(channel));
      } else if (message.type === "ping") socket.send(JSON.stringify({ type: "pong" }));
    });
  });
  return {
    sockets,
    current: () => sockets.at(-1)!,
    sample: () => sockets.at(-1)!.socket.send(JSON.stringify({ type: "message", channel: "system",
      payload: { ...health.data, risk: "ok", updatedAtMs: NOW + ++version } })),
    error: () => sockets.at(-1)!.socket.send(JSON.stringify({ type: "message", channel: "system", payload: "invalid fixture" })),
  };
}

test("changing login and API clears old WS evidence, including an inactive channel", async ({ page }, info) => {
  const f = await settingsFixture(page, "diagnostics");
  await page.clock.install({ time: NOW });
  const ws = await socketFixture(page);
  await page.route("**/e2e-ws/api/**", async route => {
    const request = route.request();
    if (request.method() !== "GET" && !request.url().endsWith("/api/auth/ws-ticket")) return route.abort();
    return route.fulfill({ response: await route.fetch({ url: request.url().replace("/e2e-ws", "") }) });
  });
  await page.goto("/#settings");
  await expect(appWs(page)).toHaveText(/已订阅/);
  ws.sample();
  await expect(appWs(page)).toHaveAttribute("title", /帧 1 · 错误 0/);
  ws.error();
  await expect(appWs(page)).toHaveAttribute("title", /WS_PAYLOAD_DECODE/);

  // Webhook is no longer active after leaving its tab; its cached metadata must reset too.
  await page.getByRole("tab", { name: "Webhook", exact: true }).click();
  await expect.poll(() => ws.current().channels.has("webhook")).toBe(true);
  ws.current().socket.send(JSON.stringify({ type: "message", channel: "webhook", payload: "invalid old webhook" }));
  await expect(page.getByRole("tabpanel", { name: "Webhook", exact: true })).toContainText("WS_PAYLOAD_DECODE");
  const connectionTab = async () => {
    await page.getByRole("tab", { name: "诊断", exact: true }).click();
    await page.getByRole("tab", { name: "连接", exact: true }).click();
  };
  await connectionTab();
  await expect.poll(() => ws.current().channels.has("webhook")).toBe(false);
  const token = async (value: string) => {
    await page.locator(".settings-api-token-task input").fill(value);
    await page.getByRole("button", { name: "保存 Token", exact: true }).click();
  };
  await token("isolated-second-login");
  await expect.poll(() => ws.sockets.length).toBe(2);
  await expect(appWs(page)).toHaveText(/已订阅/);
  await expect(appWs(page)).toHaveAttribute("title", /帧 0 · 错误 0/);
  await expect(appWs(page)).not.toHaveAttribute("title", /WS_PAYLOAD_DECODE|末次错误时间|last_message_at_ms/);
  await page.getByRole("tab", { name: "Webhook", exact: true }).click();
  await expect(page.getByRole("tabpanel", { name: "Webhook", exact: true })).not.toContainText("WS_PAYLOAD_DECODE");
  await connectionTab();
  ws.error();
  await expect(appWs(page)).toHaveAttribute("title", /错误 1/);
  await token("isolated-fixture-token");
  await expect.poll(() => ws.sockets.length).toBe(3);
  await expect(appWs(page)).toHaveAttribute("title", /帧 0 · 错误 0/);
  ws.sample(); ws.error();
  await expect(appWs(page)).toHaveAttribute("title", /帧 1 · 错误 1/);
  await page.getByRole("textbox", { name: "API Base", exact: true }).fill(`${API}/e2e-ws`);
  await page.getByRole("textbox", { name: "确认应用", exact: true }).fill("apply");
  await page.getByRole("button", { name: "保存并应用", exact: true }).click();
  await expect.poll(() => ws.sockets.length).toBe(4);
  await expect(appWs(page)).toHaveAttribute("title", /帧 0 · 错误 0/);
  await expect(appWs(page)).not.toHaveAttribute("title", /WS_PAYLOAD_DECODE|last_message_at_ms/);
  await expect.poll(() => ws.sockets.filter(socket => !socket.closed).length).toBe(1);
  expect(ws.current().socket.url()).toContain("/e2e-ws/");
  await page.screenshot({ path: info.outputPath("ws-new-source-desktop.png") });
  expect(f.writes).toEqual([]); expect(f.errors).toEqual([]);
});

test("same-source reconnect waits for a fresh frame and retains unresolved data errors", async ({ page }, info) => {
  const f = await setup(page);
  await page.clock.install({ time: NOW });
  const ws = await socketFixture(page);
  await page.addInitScript(() => {
    const Native = window.WebSocket;
    (window as any).__testSockets = [];
    window.WebSocket = class extends Native {
      constructor(url: string | URL, protocols?: string | string[]) {
        super(url, protocols);
        (window as any).__testSockets.push(this);
      }
    };
  });
  await page.goto("/#futures");
  await expect(appWs(page)).toHaveText(/已订阅/);
  ws.sample();
  await expect(appWs(page)).toHaveAttribute("title", /帧 1 · 错误 0/);
  ws.error();
  await expect(appWs(page)).toHaveAttribute("title", /WS_PAYLOAD_DECODE/);
  await ws.current().socket.close();
  await expect(appWs(page)).toHaveAttribute("title", /subscription_ack=false/);
  await page.clock.runFor(5_100);
  await expect.poll(() => ws.sockets.length).toBe(2);
  await expect(appWs(page)).toHaveAttribute("title", /subscription_ack=true/);
  await expect(appWs(page)).toHaveAttribute("title", /帧 1 · 错误 1/);
  await expect(appWs(page)).toHaveAttribute("title", /WS_PAYLOAD_DECODE/);
  await expect(appWs(page)).not.toHaveAttribute("title", /last_message_at_ms/);
  await page.locator(".futures-feed-status > summary").click();
  await expect(page.locator(".futures-diagnostics")).toContainText("套利WS已订阅 · 等待首帧");
  ws.sample();
  await expect(appWs(page)).toHaveText(/已订阅/);
  await expect(appWs(page)).toHaveAttribute("title", /帧 2 · 错误 1/);

  // A malformed envelope degrades data without inventing a physical disconnect.
  ws.current().socket.send("{broken fixture");
  await expect(appWs(page)).toHaveAttribute("title", /WS_DECODE/);
  await expect(appWs(page)).toHaveAttribute("title", /subscription_ack=true/);
  ws.sample();
  await expect(appWs(page)).toHaveText(/已订阅/);
  await expect(appWs(page)).toHaveAttribute("title", /帧 3 · 错误 2/);
  // Two already-queued close notifications must not schedule two replacement sockets.
  await page.evaluate(() => {
    const socket = (window as any).__testSockets.at(-1) as WebSocket;
    socket.dispatchEvent(new CloseEvent("close"));
    socket.dispatchEvent(new CloseEvent("close"));
  });
  await page.clock.runFor(5_100);
  await expect.poll(() => ws.sockets.length).toBe(3);
  await expect(appWs(page)).toHaveText(/已订阅/);
  await settle(page);
  expect(ws.sockets.length).toBe(3);
  await expect(appWs(page)).not.toHaveAttribute("title", /last_message_at_ms/);
  ws.sample();
  await expect(appWs(page)).toHaveAttribute("title", /帧 4 · 错误 2/);
  await page.locator(".status-summary").click();
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(appWs(page)).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
  await page.screenshot({ path: info.outputPath("ws-reconnected-mobile.png") });
  ws.current().socket.send(JSON.stringify({ type: "error", code: "WS_UNAUTHORIZED", message: "fixture authorization rejected" }));
  await expect(appWs(page)).toHaveAttribute("title", /WS_UNAUTHORIZED/);
  await expect(appWs(page)).toHaveAttribute("title", /subscription_ack=false/);
  expect(f.writes).toEqual([]); expect(f.errors).toEqual([]);
});
