import { expect, test } from "@playwright/test";
import { executionFixture, openExecution } from "./fixtures/execution-workbench";
import { NOW } from "./fixtures/opportunity-workbench";

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
