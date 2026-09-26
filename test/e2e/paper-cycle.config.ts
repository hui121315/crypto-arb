import { defineConfig } from "@playwright/test";

const prebuiltApi = process.env.CROSSLINE_E2E_API_BINARY;
const liquidationFixture = process.env.CROSSLINE_E2E_LIQUIDATION === "1" ? " CROSSLINE_PAPER_LIQUIDATION=1" : "";
const compensationFixture = process.env.CROSSLINE_E2E_COMPENSATION === "1" ? " CROSSLINE_PAPER_COMPENSATION=1" : "";
const stockCapacityFixture = process.env.CROSSLINE_E2E_STOCK_CAPACITY === "1" ? " CROSSLINE_PAPER_STOCK_CAPACITY=1" : "";
const baseServerCommand = prebuiltApi
  ? `env -i PATH="$PATH" HOME="$HOME" TMPDIR="\${TMPDIR:-/tmp}" CROSSLINE_PAPER_BROWSER=1 ${JSON.stringify(prebuiltApi)} lifecycle::profit_exit::tests::browser_server::serve_paper_browser --ignored --exact --nocapture --test-threads=1`
  : 'env -i PATH="$PATH" HOME="$HOME" TMPDIR="${TMPDIR:-/tmp}" CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-target}" CROSSLINE_PAPER_BROWSER=1 cargo test --offline -p api lifecycle::profit_exit::tests::browser_server::serve_paper_browser -- --ignored --exact --nocapture --test-threads=1';

const serverCommand = baseServerCommand.replace("CROSSLINE_PAPER_BROWSER=1", `CROSSLINE_PAPER_BROWSER=1${liquidationFixture}${compensationFixture}${stockCapacityFixture}`);

export default defineConfig({
  testDir: ".", testMatch: ["paper-cycle.spec.ts", "paper-execution-context.spec.ts", "paper-liquidation.spec.ts", "paper-compensation.spec.ts", "paper-stock-failures.spec.ts", "paper-stock-capacity.spec.ts", "stocks-batch.spec.ts", "stocks-batch-recovery.spec.ts", "stocks-monitor.spec.ts", "stocks-plan-flow.spec.ts", "webhook-recovery.spec.ts", "onchain-config-recovery.spec.ts", "automation-recovery.spec.ts"], workers: 1, timeout: 45_000,
  reporter: "list", outputDir: "../../output/playwright/paper-cycle",
  use: { baseURL: "http://127.0.0.1:18080", browserName: "chromium", channel: "chrome",
    viewport: { width: 1440, height: 900 }, screenshot: "only-on-failure", trace: "retain-on-failure" },
  webServer: [
    {
      command: serverCommand,
      cwd: "../..", url: "http://127.0.0.1:18000/health", timeout: prebuiltApi ? 30_000 : 600_000, reuseExistingServer: false,
      gracefulShutdown: { signal: "SIGTERM", timeout: 10_000 },
    },
    {
      command: "python3 -m http.server 18080 --bind 127.0.0.1 --directory frontend/dist",
      cwd: "../..", url: "http://127.0.0.1:18080", reuseExistingServer: false,
    },
  ],
});
