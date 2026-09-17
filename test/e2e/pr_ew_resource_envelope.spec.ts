import { expect, test, type Page } from "@playwright/test";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";

type ResourceEnvelope<T> = {
  data: T;
  status: string;
  source: string;
  coverage?: {
    expected: number;
    observed: number;
    coveragePct: number;
    truncated: boolean;
  };
  problems: unknown[];
};

async function useApiBase(page: Page) {
  await page.addInitScript((apiBase) => {
    window.localStorage.setItem("api_base", JSON.stringify(apiBase));
    window.localStorage.setItem("api_auth_token", JSON.stringify("e2e-token"));
  }, API_BASE);
}

test("PR-EW shared resource envelopes drive system health and the action ledger", async ({ page }) => {
  await useApiBase(page);
  const systemResponse = page.waitForResponse((response) =>
    response.url().endsWith("/api/system/health") && response.status() === 200,
  );

  await page.goto("/#settings");
  const system = await (await systemResponse).json() as ResourceEnvelope<{
    orderElapsedMs: number;
  }>;
  expect(system).toMatchObject({
    status: "ready",
    source: "system-health-snapshot",
    data: { orderElapsedMs: 24 },
    problems: [],
  });

  const actionResponse = page.waitForResponse((response) =>
    response.url().endsWith("/api/trading/action-runs") && response.status() === 200,
  );
  await page.getByRole("tab", { name: "动作账本", exact: true }).click();
  const actions = await (await actionResponse).json() as ResourceEnvelope<unknown[]>;

  expect(actions).toMatchObject({
    status: "ready",
    source: "action-run-registry",
    coverage: {
      expected: 1,
      observed: 1,
      coveragePct: 1,
      truncated: false,
    },
    problems: [],
  });
  expect(actions.data).toHaveLength(1);
  await expect(page.getByRole("heading", { name: "设置", level: 1 })).toBeVisible();
  await expect(page.getByRole("cell", { name: /提交订单/ })).toBeVisible();
  await expect(page.getByRole("cell", { name: "成功", exact: true })).toBeVisible();
  await expect(page.getByRole("cell", { name: "req-pr-ew-action-run", exact: true })).toBeVisible();
});
