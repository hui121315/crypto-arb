import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: ".", testMatch: "paper-cycle.spec.ts", workers: 1, timeout: 45_000,
  reporter: "list", outputDir: "../../output/playwright/paper-cycle",
  use: { baseURL: "http://127.0.0.1:18080", browserName: "chromium", channel: "chrome",
    viewport: { width: 1440, height: 900 }, screenshot: "only-on-failure", trace: "retain-on-failure" },
  webServer: [
    {
      command: 'env -i PATH="$PATH" HOME="$HOME" TMPDIR="${TMPDIR:-/tmp}" CROSSLINE_PAPER_BROWSER=1 cargo test --offline -p api lifecycle::profit_exit::tests::browser_server::serve_paper_browser -- --ignored --exact --nocapture --test-threads=1',
      cwd: "../..", url: "http://127.0.0.1:18000/health", timeout: 300_000, reuseExistingServer: false,
      gracefulShutdown: { signal: "SIGTERM", timeout: 10_000 },
    },
    {
      command: "python3 -m http.server 18080 --bind 127.0.0.1 --directory frontend/dist",
      cwd: "../..", url: "http://127.0.0.1:18080", reuseExistingServer: false,
    },
  ],
});
