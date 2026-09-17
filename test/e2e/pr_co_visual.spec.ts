import { expect, test } from "@playwright/test";

import {
  CO_VISUAL_SCENES,
  CO_VIEWPORTS,
  coScreenshotMasks,
  collectBrowserErrors,
  expectCoNonblankAndOverflow,
  expectCoStickyHeader,
  openCoVisualScene,
  prepareCoViewport,
  settleCoVisualPage,
} from "./helpers/pr_co_visual";

const API_BASE = process.env.CROSSLINE_E2E_API_BASE ?? "http://127.0.0.1:18000";
const WEB_BASE = process.env.CROSSLINE_E2E_WEB_BASE ?? "http://127.0.0.1:18080";
const CO_VISUAL_NOW = 1_770_000_000_000;

test.describe("PR-CO visual baseline harness", () => {
  test.use({
    storageState: {
      cookies: [],
      origins: [
        {
          origin: WEB_BASE,
          localStorage: [
            { name: "api_base", value: JSON.stringify(API_BASE) },
            { name: "api_auth_token", value: JSON.stringify("e2e-token") },
          ],
        },
      ],
    },
  });

  test.beforeEach(async ({ page }) => {
    await page.addInitScript(
      ({ apiBase, now }) => {
        window.localStorage.setItem("api_base", JSON.stringify(apiBase + "/e2e-large-tables"));
        Date.now = () => now;
      },
      { apiBase: API_BASE, now: CO_VISUAL_NOW },
    );
  });

  test("PR-CO structural visual contract remains deterministic", async ({ page }) => {
    const browserErrors = collectBrowserErrors(page);
    try {
      for (const viewport of CO_VIEWPORTS) {
        await prepareCoViewport(page, viewport);
        for (const scene of CO_VISUAL_SCENES) {
          await openCoVisualScene(page, scene);
          const label = viewport.name + " " + scene.name;
          await expectCoNonblankAndOverflow(page, label);
          await expectCoStickyHeader(page, scene.tableSelector, label);
        }
      }
      browserErrors.assertNone();
    } finally {
      browserErrors.dispose();
    }
  });

  test("futures semantic columns keep header and rows aligned", async ({ page }) => {
    const browserErrors = collectBrowserErrors(page);
    try {
      for (const viewport of CO_VIEWPORTS) {
        await prepareCoViewport(page, viewport);
        await page.goto("/#futures");
        await expect(page.getByRole("heading", { name: "期货套利", level: 1 })).toBeVisible();
        await expect(page.getByText("50 条候选", { exact: true })).toBeVisible();
        const table = page.getByRole("table", { name: "期货套利候选" });
        await expect(table.locator("tbody tr.futures-data-row")).toHaveCount(50);
        await settleCoVisualPage(page);

        const geometry = await table.evaluate((element) => {
          const table = element as HTMLTableElement;
          const header = Array.from(table.tHead?.rows[0]?.cells ?? []);
          const row = table.querySelector<HTMLTableRowElement>("tbody tr.futures-data-row");
          const cells = Array.from(row?.cells ?? []);
          const scroller = table.closest<HTMLElement>(".table-wrap");
          const deltas = header.map((cell, index) => {
            const headerRect = cell.getBoundingClientRect();
            const cellRect = cells[index]?.getBoundingClientRect();
            if (!cellRect) return Number.POSITIVE_INFINITY;
            return Math.max(
              Math.abs(headerRect.left - cellRect.left),
              Math.abs(headerRect.width - cellRect.width),
            );
          });
          return {
            actionPosition: cells.at(-1) ? getComputedStyle(cells.at(-1)!).position : "",
            actionRight: cells.at(-1) ? getComputedStyle(cells.at(-1)!).right : "",
            actionRect: cells.at(-1)?.getBoundingClientRect().right ?? Number.POSITIVE_INFINITY,
            actionWhiteSpace: cells.at(-1) ? getComputedStyle(cells.at(-1)!).whiteSpace : "",
            columnCount: table.querySelectorAll("colgroup col").length,
            headerCount: header.length,
            rowCount: cells.length,
            maxDelta: Math.max(...deltas),
            legWidth: header[2]?.getBoundingClientRect().width ?? 0,
            metricWidth: header[4]?.getBoundingClientRect().width ?? 0,
            tableWidth: table.getBoundingClientRect().width,
            scrollerWidth: scroller?.clientWidth ?? 0,
            rootOverflow: document.documentElement.scrollWidth - window.innerWidth,
          };
        });

        expect(geometry.columnCount).toBe(13);
        expect(geometry.headerCount).toBe(geometry.rowCount);
        expect(geometry.maxDelta).toBeLessThanOrEqual(1);
        expect(geometry.legWidth).toBeGreaterThan(geometry.metricWidth);
        expect(geometry.tableWidth).toBeGreaterThan(geometry.scrollerWidth);
        expect(geometry.actionPosition).toBe("sticky");
        expect(geometry.actionRight).toBe("0px");
        expect(geometry.actionRect).toBeLessThanOrEqual(viewport.width);
        expect(geometry.actionWhiteSpace).toBe("normal");
        expect(geometry.rootOverflow).toBeLessThanOrEqual(2);
      }
      browserErrors.assertNone();
    } finally {
      browserErrors.dispose();
    }
  });

  test("PR-N visual regression baselines remain reproducible", async ({ page }) => {
    const browserErrors = collectBrowserErrors(page);
    try {
      for (const viewport of CO_VIEWPORTS) {
        await prepareCoViewport(page, viewport);
        for (const scene of CO_VISUAL_SCENES) {
          await openCoVisualScene(page, scene);
          await expect(page.locator(".mod-shell")).toHaveScreenshot(
            "pr-co-" + scene.name + "-" + viewport.name + ".png",
            {
              animations: "disabled",
              caret: "hide",
              mask: coScreenshotMasks(page),
              maskColor: "#d9e2ec",
              maxDiffPixelRatio: 0.0003,
              scale: "css",
            },
          );
        }
      }
      browserErrors.assertNone();
    } finally {
      browserErrors.dispose();
    }
  });
});
