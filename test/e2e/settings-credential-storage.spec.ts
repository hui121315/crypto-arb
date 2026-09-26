import { expect, type Page } from "@playwright/test";
import { createHmac } from "node:crypto";
import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { test, SETTINGS_API as API } from "./fixtures/settings-server";

const endpoint = "/api/onchain/credentials";
const hashes = "/__test/settings/credential-fingerprints";
const provider = "jupiter_swap_v2_keyed";
const digest = (value: string) => createHmac("sha256", "isolated-settings-fingerprint").update(value).digest("hex");

async function prepare(page: Page) {
  const errors: string[] = [], writes: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.addInitScript(() => {
    if (location.origin !== "http://127.0.0.1:18080") return;
    localStorage.setItem("crossline.settings.credentialTask", JSON.stringify("onchain-provider"));
  });
  await page.route("**/*", async route => {
    const request = route.request(), url = new URL(request.url());
    expect([API, "http://127.0.0.1:18080"]).toContain(url.origin);
    if (!["GET", "HEAD", "OPTIONS"].includes(request.method()) && url.pathname !== "/api/auth/ws-ticket") {
      expect([endpoint, `${endpoint}/clear`]).toContain(url.pathname);
      writes.push(url.pathname);
    }
    await route.continue();
  });
  return { errors, writes };
}

test.use({ settingsTab: "credentials" });

test("credential save and clear preserve live values on storage failure and round-trip exact values across restart", async ({ page, server }) => {
  const file = join(server.directory, ".env");
  const initial = '# unrelated settings stay verbatim\nexport JUPITER_API_KEY="old-synthetic-key"\nJUPITER_API_KEY=older-duplicate\nCROSSLINE_LITERAL_PROBE="kept\nmultiline # value"\n';
  await server.stop();
  await writeFile(file, initial);
  await server.start();
  expect((await server.get(hashes)).JUPITER_API_KEY).toBe(digest("old-synthetic-key"));
  const { errors, writes } = await prepare(page);
  await page.goto("/#settings");
  const panel = page.locator(".provider-credentials-body");
  const input = panel.getByPlaceholder("JUPITER_API_KEY", { exact: true });
  const save = panel.getByRole("button", { name: "保存新凭证", exact: true });
  const changed = 'synthetic-$CROSSLINE_LITERAL_PROBE-${HOME}-"quoted"-\\slash#end';
  await expect(input).toHaveValue("");
  await expect(panel.locator(".provider-credential-field")).toContainText("已配置");
  await server.blockFile(".env");
  await input.fill(changed);
  const failedSave = page.waitForResponse(r => r.url().endsWith(endpoint) && r.request().method() === "POST");
  await save.click();
  expect((await failedSave).status()).toBe(503);
  await panel.getByRole("button", { name: "核对上次操作", exact: true }).click();
  await expect(input).toBeEnabled();
  await expect(input).toHaveValue(changed);
  expect((await server.get(hashes)).JUPITER_API_KEY).toBe(digest("old-synthetic-key"));
  expect(await readFile(`${file}.saved`, "utf8")).toBe(initial);
  await server.unblockFile(".env");
  const saved = page.waitForResponse(r => r.url().endsWith(endpoint) && r.request().method() === "POST");
  await save.click();
  expect((await saved).status()).toBe(200);
  await expect(input).toHaveValue("");
  expect((await server.get(hashes)).JUPITER_API_KEY).toBe(digest(changed));
  const persisted = await readFile(file, "utf8");
  expect(persisted.match(/JUPITER_API_KEY=/g)).toHaveLength(1);
  expect(persisted).toContain('CROSSLINE_LITERAL_PROBE="kept\nmultiline # value"\n');
  expect(persisted).toContain("# unrelated settings stay verbatim");
  await page.goto("about:blank");
  await server.restart();
  expect((await server.get(hashes)).JUPITER_API_KEY).toBe(digest(changed));
  expect((await server.get(hashes)).CROSSLINE_LITERAL_PROBE).toBe(digest("kept\nmultiline # value"));
  await page.goto("/#settings");
  await expect(input).toHaveValue("");

  const broken = persisted + 'BROKEN="private-malformed-content';
  await writeFile(file, broken);
  await panel.getByRole("button", { name: "清除当前凭证", exact: true }).click();
  const failedClear = page.waitForResponse(r => r.url().endsWith(`${endpoint}/clear`));
  await panel.getByRole("button", { name: "再次点击确认清除", exact: true }).click();
  expect((await failedClear).status()).toBe(503);
  await panel.getByRole("button", { name: "核对上次操作", exact: true }).click();
  await expect(input).toBeEnabled();
  await expect(panel.getByRole("alert")).toContainText("原文件与当前凭证未更改");
  await expect(panel).not.toContainText("private-malformed-content");
  await expect(panel).toContainText("字段状态与上次操作结果分别显示");
  await expect(panel).not.toContainText("已按未配置处理");
  expect((await server.get(hashes)).JUPITER_API_KEY).toBe(digest(changed));
  expect(await readFile(file, "utf8")).toBe(broken);
  await page.screenshot({ path: test.info().outputPath("credential-storage-failure-desktop.png") });
  await page.setViewportSize({ width: 390, height: 844 });
  const problem = panel.getByRole("alert");
  await problem.evaluate(el => el.scrollIntoView({ block: "center" }));
  await expect(problem).toBeInViewport({ ratio: 1 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
  await page.screenshot({ path: test.info().outputPath("credential-storage-failure-mobile.png") });
  await page.setViewportSize({ width: 1440, height: 900 });
  await writeFile(file, persisted);
  await panel.getByRole("button", { name: "清除当前凭证", exact: true }).click();
  const cleared = page.waitForResponse(r => r.url().endsWith(`${endpoint}/clear`));
  await panel.getByRole("button", { name: "再次点击确认清除", exact: true }).click();
  expect((await cleared).status()).toBe(200);
  await expect(panel.locator(".provider-credential-field")).toContainText("未配置");
  expect((await server.get(hashes)).JUPITER_API_KEY).toBeNull();
  expect(await readFile(file, "utf8")).not.toContain("JUPITER_API_KEY");
  expect(writes).toEqual([endpoint, endpoint, `${endpoint}/clear`, `${endpoint}/clear`]);

  // Two other callers share the same file transaction; no upstream probes or signing.
  const values = ["synthetic-okx-key", "synthetic-secret$variable", "synthetic\npassphrase\r#literal"];
  const concurrent = await Promise.all([
    server.patch(endpoint, { provider: "zeroex_swap_v2", fields: [{ key: "api_key", value: "synthetic-zeroex" }] }, "zeroex-save", "POST"),
    server.patch(endpoint, { provider: "okx_dex_v6", fields: ["api_key", "secret_key", "passphrase"].map((key, i) => ({ key, value: values[i] })) }, "okx-save", "POST"),
  ]);
  for (const response of concurrent) expect(response.status()).toBe(200);
  const beforeInvalid = await readFile(file, "utf8");
  const invalid = await server.patch(endpoint, { provider, fields: [{ key: "api_key", value: "invalid\0private-value" }] }, "invalid-nul", "POST");
  expect(invalid.status()).toBe(503);
  expect(await invalid.text()).not.toContain("private-value");
  expect(await readFile(file, "utf8")).toBe(beforeInvalid);
  await page.goto("about:blank");
  await server.restart();
  const restored = await server.get(hashes);
  expect(restored.JUPITER_API_KEY).toBeNull();
  expect(restored.ZEROX_API_KEY).toBe(digest("synthetic-zeroex"));
  ["OKX_DEX_API_KEY", "OKX_DEX_SECRET_KEY", "OKX_DEX_PASSPHRASE"].forEach((key, i) => expect(restored[key]).toBe(digest(values[i])));
  expect((await server.patch(`${endpoint}/clear`, { provider: "okx_dex_v6", fields: ["secret_key"] }, "partial-clear", "POST")).status()).toBe(200);
  await server.restart();
  expect((await server.get(hashes)).OKX_DEX_SECRET_KEY).toBeNull();
  expect((await server.get(hashes)).OKX_DEX_API_KEY).toBe(digest(values[0]));
  const audit = await server.audit();
  for (const secret of [changed, ...values, "synthetic-zeroex", "private-malformed-content"]) expect(audit).not.toContain(secret);
  expect(errors).toEqual([]);
});

test("startup rejects the entire malformed dotenv without partial import and recovers after repair", async ({ page, server }) => {
  const file = join(server.directory, ".env");
  await server.stop();
  const bad = 'CROSSLINE_STARTUP_PROBE=synthetic-staged\nJUPITER_API_KEY=synthetic-private-key\nBROKEN="hidden-broken-value';
  await writeFile(file, bad);
  const output = await server.startRejected();
  expect(output).toContain("环境配置文件格式无效");
  expect(output).not.toContain("hidden-broken-value");
  expect(output).not.toContain("synthetic-private-key");
  expect(await readFile(file, "utf8")).toBe(bad);
  const invalidUtf8 = Buffer.concat([Buffer.from("CROSSLINE_STARTUP_PROBE=synthetic-staged\n"), Buffer.from([0xff])]);
  await writeFile(file, invalidUtf8);
  expect(await server.startRejected()).toContain("环境配置文件读取失败");
  expect(await readFile(file)).toEqual(invalidUtf8);
  await rm(file);
  await mkdir(file);
  // The fixture refuses a non-file before loading anything, just as the product loader does.
  expect(await server.startRejected()).not.toContain("synthetic-staged");
  await rm(file, { recursive: true });
  await writeFile(file, '\ufeffCROSSLINE_STARTUP_PROBE=first\nCROSSLINE_STARTUP_PROBE=second\nCROSSLINE_LITERAL_PROBE=${CROSSLINE_STARTUP_PROBE}\n');
  await server.start();
  const restored = await server.get(hashes);
  expect(restored.CROSSLINE_STARTUP_PROBE).toBe(digest("first"));
  expect(restored.CROSSLINE_LITERAL_PROBE).toBe(digest("first"));
  const { errors, writes } = await prepare(page);
  await page.goto("/#settings");
  await expect(page.getByPlaceholder("JUPITER_API_KEY", { exact: true })).toBeEnabled();
  await expect(page.locator(".provider-credential-field")).toContainText("未配置");
  expect(writes).toEqual([]);
  expect(errors).toEqual([]);
});
