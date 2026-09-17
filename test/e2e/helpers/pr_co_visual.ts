import { expect, type Locator, type Page } from "@playwright/test";

export const CO_VIEWPORTS = [
  { name: "desktop", width: 1440, height: 900 },
  { name: "mobile", width: 390, height: 844 },
] as const;

const STABILITY_STYLE = [
  "*,",
  "*::before,",
  "*::after {",
  "  animation-delay: 0ms !important;",
  "  animation-duration: 0.001ms !important;",
  "  animation-iteration-count: 1 !important;",
  "  caret-color: transparent !important;",
  "  transition-delay: 0ms !important;",
  "  transition-duration: 0ms !important;",
  "}",
  "html { scroll-behavior: auto !important; }",
].join("\n");

export type CoVisualScene = {
  name: "opportunities" | "review" | "settings";
  hash: string;
  heading: string;
  readyText: string;
  tableSelector: string;
  rowSelector: string;
  rowCount: number;
  beforeReady?: (page: Page) => Promise<void>;
};

export const CO_VISUAL_SCENES: readonly CoVisualScene[] = [
  {
    name: "opportunities",
    hash: "#opportunities",
    heading: "机会扫描",
    readyText: "50 条机会",
    tableSelector: ".opportunity-layout table.clean-table",
    rowSelector: ".opportunity-layout table.clean-table tbody tr",
    rowCount: 50,
  },
  {
    name: "review",
    hash: "#review",
    heading: "复盘",
    readyText: "执行账本 · 50/1000 行 · 还有下一页",
    tableSelector: ".review-table",
    rowSelector: ".review-table tbody tr",
    rowCount: 50,
  },
  {
    name: "settings",
    hash: "#settings",
    heading: "设置",
    readyText: "运行态矩阵",
    tableSelector: "table.settings-table",
    rowSelector: "table.settings-table tbody tr",
    rowCount: 59,
    beforeReady: async (page) => {
      await page.getByRole("tab", { name: "诊断" }).click();
    },
  },
];

export async function prepareCoViewport(
  page: Page,
  viewport: (typeof CO_VIEWPORTS)[number],
): Promise<void> {
  await page.setViewportSize({ width: viewport.width, height: viewport.height });
  await page.emulateMedia({ colorScheme: "light", reducedMotion: "reduce" });
}

export async function settleCoVisualPage(page: Page): Promise<void> {
  await page.waitForFunction(() => document.readyState === "complete");
  await page.addStyleTag({ content: STABILITY_STYLE });
  await page.evaluate(async () => {
    const fonts = document.fonts;
    if (fonts) await fonts.ready;
    await Promise.all(
      Array.from(document.images, async (image) => {
        if (image.complete) return;
        await image.decode().catch(() => undefined);
      }),
    );
  });
  await page.waitForTimeout(50);
}

export function collectBrowserErrors(page: Page) {
  const errors: string[] = [];
  const onPageError = (error: Error) => errors.push(error.message);
  const onConsole = (message: { type(): string; text(): string }) => {
    if (message.type() === "error") errors.push(message.text());
  };
  page.on("pageerror", onPageError);
  page.on("console", onConsole);

  return {
    assertNone: () => expect(errors, "browser errors").toEqual([]),
    dispose: () => {
      page.off("pageerror", onPageError);
      page.off("console", onConsole);
    },
  };
}

export async function openCoVisualScene(page: Page, scene: CoVisualScene): Promise<void> {
  await page.goto("/" + scene.hash);
  await expect(
    page.getByRole("heading", { name: scene.heading, level: 1, exact: true }),
  ).toBeVisible();
  await scene.beforeReady?.(page);
  await expect(page.getByText(scene.readyText, { exact: true })).toBeVisible();
  await expect(page.locator(scene.rowSelector)).toHaveCount(scene.rowCount);
  await settleCoVisualPage(page);
}

export async function expectCoNonblankAndOverflow(page: Page, label: string): Promise<void> {
  const report = await page.evaluate(() => {
    const body = document.body;
    const rootOverflow = Math.max(
      document.documentElement.scrollWidth,
      body?.scrollWidth ?? 0,
    ) - window.innerWidth;
    const visibleSurfaces = Array.from(document.querySelectorAll(".surface"))
      .filter((element) => {
        const rect = element.getBoundingClientRect();
        return rect.width > 0 && rect.height > 0;
      })
      .map((element) => ({
        className: element.className,
        text: (element.textContent ?? "").trim().length,
      }));
    return {
      rootOverflow,
      visibleSurfaces,
      shellText: (document.querySelector(".mod-shell")?.textContent ?? "").trim().length,
    };
  });

  expect(report.rootOverflow, label + " document horizontal overflow").toBeLessThanOrEqual(2);
  expect(report.shellText, label + " nonblank shell").toBeGreaterThan(0);
  expect(
    report.visibleSurfaces.filter((surface) => surface.text === 0),
    label + " blank visible surfaces",
  ).toEqual([]);
}

export async function expectCoStickyHeader(
  page: Page,
  tableSelector: string,
  label: string,
): Promise<void> {
  const before = await tableHeaderGeometry(page, tableSelector);
  expect(before.visible, label + " table header visibility").toBeTruthy();
  if (!before.scrollable) return;

  await page.evaluate((selector) => {
    const table = document.querySelector(selector);
    const scroller = table?.closest(".table-wrap");
    if (scroller) scroller.scrollTop = 220;
  }, tableSelector);
  await page.waitForTimeout(50);

  const after = await tableHeaderGeometry(page, tableSelector);
  expect(after.visible, label + " sticky header visibility").toBeTruthy();
  expect(
    Math.abs(after.headerTop - before.headerTop),
    label + " sticky header position",
  ).toBeLessThanOrEqual(2);
}

export function coScreenshotMasks(page: Page): Locator[] {
  return [
    page.getByTestId("top-status-bar"),
    page.locator(".settings-message"),
    page.locator(".reason-pill"),
  ];
}

async function tableHeaderGeometry(page: Page, tableSelector: string) {
  return await page.evaluate((selector) => {
    const table = document.querySelector(selector);
    const scroller = table?.closest(".table-wrap");
    const header = table?.querySelector("thead th");
    if (!scroller || !header) {
      return {
        headerTop: Number.NaN,
        scrollable: false,
        visible: false,
      };
    }
    const scrollerRect = scroller.getBoundingClientRect();
    const headerRect = header.getBoundingClientRect();
    return {
      headerTop: headerRect.top,
      scrollable: scroller.scrollHeight > scroller.clientHeight + 1,
      visible: headerRect.bottom > scrollerRect.top
        && headerRect.top < scrollerRect.bottom
        && headerRect.left < scrollerRect.right
        && headerRect.right > scrollerRect.left,
    };
  }, tableSelector);
}
