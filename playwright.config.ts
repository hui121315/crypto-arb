import { defineConfig, devices } from "@playwright/test";

const apiPort = Number(process.env.CROSSLINE_E2E_API_PORT ?? "18000");
const webPort = Number(process.env.CROSSLINE_E2E_WEB_PORT ?? "18080");
const apiBase = `http://127.0.0.1:${apiPort}`;
const webBase = `http://127.0.0.1:${webPort}`;
const webServerTimeoutMs = process.env.CI ? 300_000 : 120_000;
const qaProfile = process.env.CROSSLINE_E2E_PROFILE ?? "dev";

if (qaProfile !== "dev" && qaProfile !== "release") {
  throw new Error(`CROSSLINE_E2E_PROFILE must be dev or release, got ${qaProfile}`);
}

const frontendCommand = qaProfile === "release"
  ? `python3 -m http.server ${webPort} --bind 127.0.0.1 --directory frontend/dist`
  : `cd frontend && env -u NO_COLOR trunk build --release=false && python3 -m http.server ${webPort} --bind 127.0.0.1 --directory dist`;

export default defineConfig({
  testDir: "test/e2e",
  timeout: 30_000,
  expect: { timeout: 5_000 },
  reporter: [["list"]],
  snapshotPathTemplate: "{testDir}/{testFilePath}-snapshots/{arg}{ext}",
  use: {
    baseURL: webBase,
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
  webServer: [
    {
      command: "node test/e2e/mock_api.mjs",
      url: `${apiBase}/health`,
      timeout: 10_000,
      reuseExistingServer: !process.env.CI,
      env: { CROSSLINE_E2E_API_PORT: String(apiPort), CROSSLINE_E2E_WEB_BASE: webBase },
    },
    {
      command: frontendCommand,
      url: webBase,
      timeout: webServerTimeoutMs,
      reuseExistingServer: !process.env.CI,
    },
  ],
});
