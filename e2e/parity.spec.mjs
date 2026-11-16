// Producer parity, the pixel layer: the three producers of each demo must
// render each state identically.
//
// demos.spec.mjs asserts the DOM-level parity (the rendered tree against
// the selftest's expectations); its serializer only sees the kind, the id,
// the content, and the style tokens. This is the pixel-level twin: a
// producer that diverges in anything else — a layout shift, a color, a
// glyph — fails here.
//
// The desktop gets the same treatment: its grid and the windows it opens,
// at the desktop's viewport.
//
// The comparison is within one browser (the three pages, one machine), so
// it runs everywhere, unlike the CI-only screenshot goldens.

import { expect, test } from '@playwright/test';
import { writeFileSync } from 'node:fs';
import config from '../playwright.config.mjs';
import {
  demoPages,
  demoStates,
  desktopPages,
  desktopStates,
  pixelDiff,
  producers,
  settle,
} from './helpers.mjs';

// One context per page, driven into the state, settled, screenshotted, and
// compared. Manually created contexts do not inherit the config's `use`
// options, so the viewport and baseURL are passed explicitly; the per-page
// contexts also keep any page.clock mock (the stopwatch's) from accumulating
// across pages. On divergence, write the evidence into the test's output
// directory so the CI's test-results artifact carries it: each page's
// screenshot and its rendered DOM — a pixel diff with the DOM in hand shows
// what the serializer's view misses.
async function assertParity(browser, testInfo, viewport, paths, drive, domOf) {
  const options = { viewport, baseURL: config.use.baseURL };
  const contexts = await Promise.all(paths.map(() => browser.newContext(options)));
  const pages = await Promise.all(contexts.map((c) => c.newPage()));
  try {
    await Promise.all(paths.map((path, i) => drive(pages[i], path)));
    await Promise.all(pages.map(settle));
    const shots = await Promise.all(pages.map((p) => p.screenshot()));

    let diverged = false;
    for (let i = 1; i < shots.length; i++) {
      const diff = await pixelDiff(pages[0], shots[0], shots[i]);
      if (diff.sizeA !== null) {
        diverged = true;
        expect
          .soft(
            false,
            `${producers[i]} renders at ${diff.sizeA.join('x')}, the first at ${diff.sizeB.join('x')}`,
          )
          .toBe(true);
      } else if (diff.n > 0) {
        diverged = true;
        expect
          .soft(
            diff.n,
            `${producers[i]} differs from ${producers[0]}: ${diff.n} pixels differ${
              diff.box ? `, in the box (${diff.box.join(', ')})` : ''
            }`,
          )
          .toBe(0);
      }
    }

    if (diverged) {
      for (let i = 0; i < pages.length; i++) {
        const name = producers[i].toLowerCase();
        writeFileSync(testInfo.outputPath(`screenshot-${name}.png`), shots[i]);
        writeFileSync(
          testInfo.outputPath(`dom-${name}.json`),
          JSON.stringify(await domOf(pages[i]), null, 2),
        );
      }
    }
  } finally {
    await Promise.all(pages.map((p) => p.close()));
    await Promise.all(contexts.map((c) => c.close()));
  }
}

test.describe('producer parity', () => {
  // The iced host's window size (crates/host-iced's WINDOW_WIDTH/HEIGHT) —
  // the goldens' viewport.
  for (const [demo, paths] of Object.entries(demoPages)) {
    for (const state of Object.keys(demoStates[demo])) {
      test(`${demo} ${state}: the producers render identically`, async ({ browser }, testInfo) => {
        await assertParity(
          browser,
          testInfo,
          { width: 360, height: 340 },
          paths,
          demoStates[demo][state],
          (page) =>
            page.evaluate(() => ({
              app: document.getElementById('app').outerHTML,
              badge: document.getElementById('badge').textContent,
            })),
        );
      });
    }
  }

  // The desktop, at the config's default viewport (the visual spec keeps the
  // desktop at 1280x720).
  for (const state of Object.keys(desktopStates)) {
    test(`desktop ${state}: the producers render identically`, async ({ browser }, testInfo) => {
      await assertParity(
        browser,
        testInfo,
        { width: 1280, height: 720 },
        desktopPages,
        desktopStates[state],
        (page) =>
          page.evaluate(() => ({
            count: document.getElementById('window-count').textContent,
            grid: document.getElementById('grid').outerHTML,
            windows: document.getElementById('windows').outerHTML,
          })),
      );
    });
  }
});
