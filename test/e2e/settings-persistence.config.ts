import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: ".", testMatch: ["settings-persistence.spec.ts", "settings-delivery-persistence.spec.ts", "settings-restore-failure.spec.ts", "settings-credential-storage.spec.ts"], workers: 1, timeout: 60_000,
  reporter: "list", outputDir: "../../output/playwright/settings-persistence",
  use: { baseURL: "http://127.0.0.1:18080", browserName: "chromium", channel: "chrome",
    viewport: { width: 1440, height: 900 }, screenshot: "only-on-failure", trace: "retain-on-failure" },
  webServer: {
    command: "python3 -m http.server 18080 --bind 127.0.0.1 --directory frontend/dist",
    cwd: "../..", url: "http://127.0.0.1:18080", reuseExistingServer: false,
  },
});
