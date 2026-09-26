import { test as base, expect, type APIRequestContext } from "@playwright/test";
import { spawn, type ChildProcess } from "node:child_process";
import { mkdtemp, mkdir, readFile, rename, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

export const SETTINGS_API = "http://127.0.0.1:18000";
const token = "isolated-settings-browser";
export const settingsHeaders = { Authorization: `Bearer ${token}` };

class SettingsServer {
  private child?: ChildProcess;
  private output = "";
  private readyAt = 0;
  readonly checkpoint: string;
  constructor(readonly directory: string, private request: APIRequestContext, private binary: string) {
    this.checkpoint = join(directory, "execution-events.trading-runtime.json");
  }

  private launch() {
    expect(this.child).toBeUndefined();
    this.output = "";
    this.child = spawn(this.binary, [
      "services::trading_runtime_config::browser_server::serve_settings_browser",
      "--ignored", "--exact", "--nocapture", "--test-threads=1",
    ], {
      cwd: this.directory,
      env: { PATH: process.env.PATH, HOME: this.directory, TMPDIR: tmpdir(),
        CROSSLINE_SETTINGS_BROWSER_DIR: this.directory },
      stdio: ["ignore", "pipe", "pipe"],
    });
    this.child.stdout?.on("data", chunk => this.output += chunk.toString());
    this.child.stderr?.on("data", chunk => this.output += chunk.toString());
    this.child.on("error", error => this.output += error.message);
    return this.child;
  }

  async start() {
    this.launch();
    await expect.poll(async () => {
      if (this.child?.exitCode != null) throw new Error(this.output);
      return this.request.get(`${SETTINGS_API}/health`, { timeout: 500 }).then(r => r.status()).catch(() => 0);
    }, { timeout: 20_000, message: "isolated settings API becomes ready" }).toBe(200);
    this.readyAt = Date.now();
  }

  async startRejected() {
    const child = this.launch();
    await expect.poll(() => child.exitCode, { timeout: 10_000 }).not.toBeNull();
    expect(child.exitCode).not.toBe(0);
    expect(child.signalCode).toBeNull();
    this.child = undefined;
    expect(await this.request.get(`${SETTINGS_API}/health`, { timeout: 500 }).then(() => true).catch(() => false)).toBe(false);
    expect(this.output).not.toContain("partial environment import detected");
    return this.output;
  }

  async stop() {
    const child = this.child;
    if (!child) return;
    if (child.exitCode == null && child.signalCode == null) {
      // Product shutdown ignores signals during its first two seconds of startup.
      const grace = Math.max(0, 2_200 - (Date.now() - this.readyAt));
      if (grace) await new Promise(resolve => setTimeout(resolve, grace));
      const closed = new Promise<void>(resolve => child.once("exit", () => resolve()));
      child.kill("SIGTERM");
      const timer = setTimeout(() => child.kill("SIGKILL"), 8_000);
      await closed;
      clearTimeout(timer);
      expect(child.signalCode, this.output).toBeNull();
      expect(child.exitCode, this.output).toBe(0);
    }
    this.child = undefined;
  }

  async restart() { await this.stop(); await this.start(); }
  async snapshot() { return JSON.parse(await readFile(this.checkpoint, "utf8")); }
  async blockCheckpoint() {
    await rename(this.checkpoint, `${this.checkpoint}.saved`);
    await mkdir(this.checkpoint);
  }
  async unblockCheckpoint() {
    await rm(this.checkpoint, { recursive: true });
    await rename(`${this.checkpoint}.saved`, this.checkpoint);
  }
  async blockFile(name: string) {
    const path = join(this.directory, name);
    await rename(path, `${path}.saved`);
    await mkdir(path);
  }
  async unblockFile(name: string) {
    const path = join(this.directory, name);
    await rm(path, { recursive: true });
    await rename(`${path}.saved`, path);
  }
  async get(path: string) {
    const response = await this.request.get(`${SETTINGS_API}${path}`, { headers: settingsHeaders });
    expect(response.status()).toBe(200);
    return response.json();
  }
  async status() { return (await this.request.get(`${SETTINGS_API}/api/trading/status`, { headers: settingsHeaders })).json(); }
  async action(id: string) { return (await this.request.get(`${SETTINGS_API}/api/trading/action-runs/${id}`, { headers: settingsHeaders })).json(); }
  async write(path: string, data: object, key: string, method = "POST") {
    return this.patch(`/api/trading/${path}`, data, key, method);
  }
  async patch(path: string, data: object, key: string, method = "PATCH") {
    return this.request.fetch(`${SETTINGS_API}${path}`, {
      method, data, headers: { ...settingsHeaders, "idempotency-key": key, "x-request-id": `request-${key}` },
    });
  }
  async audit() { return readFile(join(this.directory, "audit.jsonl"), "utf8"); }
  async receipt(key: string) {
    const runs = (await this.audit()).trim().split("\n")
      .map(line => JSON.parse(line).detail?.actionRun).filter(run => run?.idempotencyKey === key);
    expect(runs.length).toBeGreaterThan(0);
    return runs.at(-1);
  }
}

export const test = base.extend<{ server: SettingsServer; settingsTab: string }>({
  settingsTab: ["risk", { option: true }],
  server: async ({ request, page, settingsTab }, use, info) => {
    const binary = process.env.CROSSLINE_E2E_API_BINARY;
    if (!binary) throw new Error("Build the API test binary once and set CROSSLINE_E2E_API_BINARY");
    const directory = await mkdtemp(join(tmpdir(), "crossline-settings-e2e-"));
    await writeFile(join(directory, "isolated-settings-fixture"), "fixture-only");
    await writeFile(join(directory, ".env"), "# isolated settings only\n");
    const server = new SettingsServer(directory, request, resolve(binary));
    await page.addInitScript(({ api, token, settingsTab }) => {
      if (location.origin !== "http://127.0.0.1:18080") return;
      localStorage.setItem("api_base", JSON.stringify(api));
      localStorage.setItem("api_auth_token", JSON.stringify(token));
      localStorage.setItem("crossline.settings.activeTab", JSON.stringify(settingsTab));
    }, { api: SETTINGS_API, token, settingsTab });
    try {
      await server.start();
      await use(server);
    } finally {
      await page.goto("about:blank").catch(() => {});
      await server.stop();
      await info.attach("isolated-audit", { body: await server.audit().catch(() => ""), contentType: "text/plain" });
      await rm(directory, { recursive: true, force: true });
    }
  },
});
