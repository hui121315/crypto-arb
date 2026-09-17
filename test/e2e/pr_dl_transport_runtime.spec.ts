import { expect, test, type Page } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";

type SubscribeCommand = {
  channels: string[];
  requestId: string;
};

async function useWsCorrelationScenario(
  page: Page,
  ackMode: "echo" | "mismatch",
  commands: SubscribeCommand[],
) {
  await page.addInitScript((apiBase) => {
    window.localStorage.setItem("api_base", JSON.stringify(apiBase));
    window.localStorage.setItem("api_auth_token", JSON.stringify("e2e-token"));
  }, API_BASE);
  await page.routeWebSocket("**/ws", (socket) => {
    socket.onMessage((raw) => {
      const message = JSON.parse(raw.toString());
      if (message.type !== "subscribe") return;

      commands.push({
        channels: message.channels ?? [],
        requestId: message.requestId ?? "",
      });
      socket.send(
        JSON.stringify({
          type: "ack",
          subscribed: message.channels ?? [],
          requestId:
            ackMode === "echo" ? message.requestId : `${message.requestId}-stale`,
        }),
      );
    });
  });
}

function arbitrageRequestIds(commands: SubscribeCommand[]) {
  return commands
    .filter((command) => command.channels.includes("arbitrage"))
    .map((command) => command.requestId);
}

function subscribeRequestIds(commands: SubscribeCommand[]) {
  return commands.map((command) => command.requestId);
}

function arbitrageRequestId(commands: SubscribeCommand[]) {
  return arbitrageRequestIds(commands).at(-1) ?? "";
}

test("PR-DL websocket subscribe correlates browser request id before marking connected", async ({
  page,
}) => {
  const commands: SubscribeCommand[] = [];
  await useWsCorrelationScenario(page, "echo", commands);

  await page.goto("/#opportunities");
  await expect(page.getByRole("heading", { name: "机会扫描" })).toBeVisible();
  await expect(page.locator(".clean-table tbody tr").filter({ hasText: "MU" })).toHaveCount(1);
  await expect.poll(() => arbitrageRequestId(commands)).toMatch(/^web-[0-9a-f]+$/);
  await expect(page.locator(".settings-message").filter({ hasText: "套利WS已订阅" })).toBeVisible();
  await expect(
    page.locator(".settings-message.is-error").filter({ hasText: "requestId does not match" }),
  ).toHaveCount(0);
});

test("PR-DL mismatched websocket ack stays typed without hiding REST fallback rows", async ({
  page,
}) => {
  const commands: SubscribeCommand[] = [];
  await useWsCorrelationScenario(page, "mismatch", commands);

  await page.goto("/#opportunities");
  await expect(page.getByRole("heading", { name: "机会扫描" })).toBeVisible();
  await expect.poll(() => arbitrageRequestId(commands)).toMatch(/^web-[0-9a-f]+$/);

  const error = page.locator(".settings-message.is-error").filter({ hasText: "套利WS异常" });
  await expect(error).toContainText("subscribe ack requestId does not match a pending command");
  await expect(error).toContainText("retry 5000ms");
  await expect(error).toContainText(/帧 0 · 错误 [1-9]\d*/);
  await expect
    .poll(async () => {
      const text = await error.textContent();
      return subscribeRequestIds(commands).some((id) => text?.includes(`request_id ${id}-stale`));
    })
    .toBeTruthy();
  await expect(page.locator(".clean-table tbody tr").filter({ hasText: "MU" })).toHaveCount(1);
});
