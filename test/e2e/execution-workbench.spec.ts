import { expect, test } from "@playwright/test";
import { executionFixture, openExecution } from "./fixtures/execution-workbench";
import { NOW } from "./fixtures/opportunity-workbench";

test("ticket clock survives skew rollback replay and delayed evidence without renewing expiry", async ({ page }) => {
  await page.clock.install({ time: NOW });
  const f = await executionFixture(page);
  await page.clock.setFixedTime(NOW + 86_400_000);
  await page.setViewportSize({ width: 1440, height: 900 });
  await openExecution(page);
  const artifact = page.locator(".execution-artifact");
  const status = artifact.locator(".execution-artifact-status");
  const review = artifact.getByRole("checkbox");
  const validate = artifact.getByRole("button", { name: "校验票据", exact: true });
  const refresh = page.getByRole("button", { name: "刷新预览", exact: true });
  const submit = page.locator(".confirm-action.primary");
  await validate.click();
  await review.check();
  await expect(submit).toBeEnabled();

  f.holdValidation();
  await validate.click();
  await expect.poll(() => f.validations.length).toBe(2);
  await page.clock.setFixedTime(NOW - 86_400_000);
  await page.clock.runFor(31_000);
  await expect(status).toContainText("已过期");
  const lateValidation = page.waitForResponse("**/execution-artifacts/validate");
  f.releaseValidation();
  await (await lateValidation).finished();
  await expect(review).toBeDisabled();
  await expect(review).not.toBeChecked();
  await expect(submit).toBeDisabled();
  await expect(artifact).toContainText("上次测算净收益");
  await expect(page.locator(".execution-risk-summary")).toContainText("上次测算净收益");
  await expect(page.locator(".execution-evidence-details summary")).toContainText("上次交易检查 · 已过期");
  await expect(page.locator(".slippage-section")).toContainText("上次盘口 · 已过期");
  expect(f.previews).toHaveLength(1);
  expect(f.builds).toHaveLength(1);

  await page.clock.setFixedTime(NOW + 86_400_000);
  f.replayPreview();
  await refresh.click();
  await expect.poll(() => f.builds.length).toBe(2);
  await expect(status).toContainText("已过期");
  await expect(validate).toBeDisabled();
  f.setServerTime(NOW + 31_000);
  await refresh.click();
  await expect(status).toContainText("待校验");
  await expect(review).not.toBeChecked();

  f.holdPreview();
  f.setServerTime(NOW + 32_000);
  await refresh.click();
  await expect.poll(() => f.previews.length).toBe(4);
  await page.clock.runFor(31_000);
  f.releasePreview();
  await expect.poll(() => f.builds.length).toBe(4);
  await expect(status).toContainText("已过期");
  await expect(validate).toBeDisabled();
  await artifact.scrollIntoViewIfNeeded();
  await page.screenshot({ path: test.info().outputPath("execution-expired-clock-desktop.png") });

  f.setServerTime(0);
  await refresh.click();
  await expect(page.locator(".execution-actionbar")).toContainText("票据缺少有效时间");
  await expect(submit).toBeDisabled();
  expect(f.builds).toHaveLength(4);
  f.setServerTime(NOW + 64_000);
  await refresh.click();
  await expect(status).toContainText("待校验");
  await validate.click();
  await review.check();
  await expect(submit).toBeEnabled();
  // A forward correction that expires evidence cannot be undone by moving back.
  await page.clock.setFixedTime(NOW + 86_400_000 + 31_000);
  await page.clock.runFor(1_000);
  await expect(status).toContainText("已过期");
  await page.clock.setFixedTime(NOW + 86_400_000);
  await page.clock.runFor(1_000);
  await expect(status).toContainText("已过期");
  await expect(review).not.toBeChecked();
  await expect(submit).toBeDisabled();
  await page.setViewportSize({ width: 390, height: 844 });
  await artifact.scrollIntoViewIfNeeded();
  for (const control of [artifact, validate, review, submit]) {
    const box = await control.boundingBox();
    expect(box!.x).toBeGreaterThanOrEqual(0);
    expect(box!.x + box!.width).toBeLessThanOrEqual(390);
  }
  await page.screenshot({ path: test.info().outputPath("execution-expired-clock-mobile.png") });
  const handoff = f.handoffCode();
  await page.goto("/#execution");
  await page.reload();
  await expect(page.locator(".execution-ticket")).toHaveCount(0);
  const inbox = page.locator(".execution-artifact-inbox");
  await inbox.locator("summary").click();
  await inbox.getByLabel("Webhook 校验码").fill(handoff);
  await inbox.getByRole("button", { name: "校验提醒票据", exact: true }).click();
  await expect(inbox).toContainText("提醒票据校验通过 · 未下单");
  await page.clock.setFixedTime(NOW - 86_400_000);
  await page.clock.runFor(31_000);
  await expect(inbox).toContainText("提醒票据已过期");
  await expect(page.locator(".confirm-action.primary")).toHaveCount(0);
  expect(f.builds).toHaveLength(5);
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("ticket review stays stable across validation and clock updates on desktop and mobile", async ({ page }) => {
  const f = await executionFixture(page);
  await openExecution(page);
  const artifact = page.locator(".execution-artifact");
  const details = artifact.locator("details");
  const confirm = artifact.getByRole("checkbox");
  const submit = page.locator(".confirm-action.primary");
  await expect(submit).toBeDisabled();
  await details.locator("summary").click();
  await artifact.getByRole("button", { name: "校验票据" }).click();
  await expect(confirm).toBeEnabled();
  await confirm.check();
  await expect(submit).toBeEnabled();
  await expect(details).toHaveAttribute("open", "");
  await confirm.focus();
  await page.clock.setFixedTime(NOW + 2000);
  await expect(artifact.locator(".execution-artifact-metrics")).toContainText("28 秒");
  await expect(confirm).toBeFocused();
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await artifact.scrollIntoViewIfNeeded();
    for (const control of [artifact, confirm, artifact.getByRole("button", { name: "校验票据" }), submit]) {
      const box = await control.boundingBox();
      expect(box!.x).toBeGreaterThanOrEqual(0);
      expect(box!.x + box!.width).toBeLessThanOrEqual(width);
    }
    await page.screenshot({ path: test.info().outputPath(`execution-${width}.png`), fullPage: true });
    if (width === 390) {
      await confirm.uncheck();
      await expect(submit).toBeDisabled();
      await confirm.check();
      await expect(submit).toBeEnabled();
      await submit.scrollIntoViewIfNeeded();
      expect(await submit.evaluate((element) => {
        const box = element.getBoundingClientRect();
        return element.contains(document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2));
      })).toBe(true);
      await page.screenshot({ path: test.info().outputPath("execution-mobile-viewport.png") });
    }
  }
  expect(f.previews).toHaveLength(1);
  expect(f.validations).toHaveLength(1);
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("editing immediately blocks submission and ignores validation for the previous ticket", async ({ page }) => {
  const f = await executionFixture(page);
  await openExecution(page);
  const artifact = page.locator(".execution-artifact");
  await artifact.getByRole("button", { name: "校验票据" }).click();
  await artifact.getByRole("checkbox").check();
  await expect(page.locator(".confirm-action.primary")).toBeEnabled();
  const capital = page.getByRole("textbox", { name: "计划本金 USD", exact: true });
  await capital.evaluate((input: HTMLInputElement) => {
    input.value = "40";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    (document.querySelector(".confirm-action.primary") as HTMLButtonElement).click();
  });
  await expect(page.locator(".confirm-action.primary")).toBeDisabled();
  await expect.poll(() => f.builds.length).toBe(2);
  f.holdValidation();
  await artifact.getByRole("button", { name: "校验票据" }).click();
  await expect.poll(() => f.validations.length).toBe(2);
  await capital.fill("50");
  await expect.poll(() => f.builds.length).toBe(3);
  const old = page.waitForResponse("**/execution-artifacts/validate");
  f.releaseValidation();
  await (await old).finished();
  await expect(artifact.locator(".execution-artifact-status")).toContainText("待校验");
  await expect(artifact.getByRole("checkbox")).toBeDisabled();
  await artifact.getByRole("button", { name: "校验票据" }).click();
  await artifact.getByRole("checkbox").check();
  await expect(page.locator(".confirm-action.primary")).toBeEnabled();
  expect(f.previews.at(-1).capitalUsd).toBe(50);
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("expired artifacts revoke review and flow success without adding backend polling", async ({ page }) => {
  const f = await executionFixture(page);
  await openExecution(page);
  const artifact = page.locator(".execution-artifact");
  await artifact.getByRole("button", { name: "校验票据" }).click();
  await artifact.getByRole("checkbox").check();
  await page.locator(".execution-flow-details summary").click();
  await page.clock.setFixedTime(NOW + 30000);
  await expect(artifact.locator(".execution-artifact-status")).toContainText("已过期");
  await expect(artifact.getByRole("checkbox")).not.toBeChecked();
  await expect(page.locator(".confirm-action.primary")).toBeDisabled();
  await expect(page.locator(".execution-flow-details")).toHaveAttribute("open", "");
  await expect(page.locator(".execution-flow-current")).toContainText("EXPIRED");
  expect(f.previews).toHaveLength(1);
  expect(f.builds).toHaveLength(1);
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("failed revalidation cannot reuse a previous successful check", async ({ page }) => {
  const f = await executionFixture(page);
  await openExecution(page);
  const artifact = page.locator(".execution-artifact");
  await artifact.getByRole("button", { name: "校验票据" }).click();
  await artifact.getByRole("checkbox").check();
  f.failValidation();
  await artifact.getByRole("button", { name: "校验票据" }).click();
  await expect(artifact.locator(".execution-artifact-status")).toContainText("校验失败");
  await expect(artifact.getByRole("checkbox")).toBeDisabled();
  await expect(page.locator(".confirm-action.primary")).toBeDisabled();
  await expect(page.locator(".execution-flow-current")).toContainText("validation unavailable");
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("late build and validation callbacks are safe after leaving execution", async ({ page }) => {
  const f = await executionFixture(page);
  await openExecution(page);
  f.holdValidation();
  await page.getByRole("button", { name: "校验票据" }).click();
  await expect.poll(() => f.validations.length).toBe(1);
  f.holdBuild();
  await page.getByRole("textbox", { name: "计划本金 USD", exact: true }).fill("40");
  await expect.poll(() => f.builds.length).toBe(2);
  await page.getByRole("button", { name: "切换到复盘", exact: true }).click();
  const build = page.waitForResponse("**/execution-artifacts/build");
  const validate = page.waitForResponse("**/execution-artifacts/validate");
  f.releaseBuild();
  f.releaseValidation();
  await (await build).finished();
  await (await validate).finished();
  await expect(page.locator(".execution-artifact")).toHaveCount(0);
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("artifact returned for another ticket cannot enable review", async ({ page }) => {
  const f = await executionFixture(page);
  f.mismatch();
  await page.goto("/#futures");
  await page.getByRole("button", { name: "构建新双腿", exact: true }).click();
  await expect(page.locator(".execution-artifact")).toContainText("与当前票据不一致");
  await expect(page.locator(".confirm-action.primary")).toBeDisabled();
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("empty or out-of-range inputs cannot silently reuse reviewed defaults", async ({ page }) => {
  const f = await executionFixture(page);
  await openExecution(page);
  await page.getByRole("button", { name: "校验票据" }).click();
  await page.locator(".execution-artifact").getByRole("checkbox").check();
  const capital = page.getByRole("textbox", { name: "计划本金 USD", exact: true });
  await capital.evaluate((input: HTMLInputElement) => {
    input.value = "";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    (document.querySelector(".confirm-action.primary") as HTMLButtonElement).click();
  });
  expect(f.errors).toEqual([]);
  await expect(page.locator(".execution-actionbar")).toContainText("计划本金须大于 0");
  await expect(page.locator(".confirm-action.primary")).toBeDisabled();
  expect(f.previews).toHaveLength(1);
  await capital.fill("30");
  await expect.poll(() => f.builds.length).toBe(2);
  await expect(page.locator(".execution-artifact").getByRole("checkbox")).toBeDisabled();
  await page.getByRole("textbox", { name: "杠杆", exact: true }).fill("99");
  await expect(page.locator(".execution-actionbar")).toContainText("杠杆须在 0.1 至 20 之间");
  expect(f.previews).toHaveLength(2);
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});

test("a late preview cannot roll back the latest amount or bind its artifact", async ({ page }) => {
  const f = await executionFixture(page);
  await openExecution(page);
  f.holdPreview();
  const capital = page.getByRole("textbox", { name: "计划本金 USD", exact: true });
  await capital.fill("40");
  await expect.poll(() => f.previews.length).toBe(2);
  await capital.fill("50");
  await expect.poll(() => f.builds.length).toBe(2);
  expect(f.builds[1].ticketId).toBe("ticket-3");
  const old = page.waitForResponse("**/fixture-perp_cross-BTC/preview");
  f.releasePreview();
  await (await old).finished();
  await page.locator(".execution-artifact-details summary").click();
  await expect(page.locator(".execution-artifact-identifiers")).toContainText("ticket-3");
  await expect(capital).toHaveValue("50");
  expect(f.builds).toHaveLength(2);
  expect(f.errors).toEqual([]);
  expect(f.writes).toEqual([]);
});
