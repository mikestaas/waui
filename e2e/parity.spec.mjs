// Producer parity, the pixel layer: the three producers of each demo must
// render each state identically.
//
// demos.spec.mjs asserts the DOM-level parity (the rendered tree against
// the selftest's expectations); its serializer only sees the kind, the id,
// the content, and the style tokens. This is the pixel-level twin: a
// producer that diverges in anything else — a layout shift, a color, a
// glyph — fails here.
//
// The comparison is within one browser (the three pages, one machine), so
// it runs everywhere, unlike the CI-only screenshot goldens.

import { expect, test } from '@playwright/test';
import { writeFileSync } from 'node:fs';
import { demoPages, demoStates, pixelDiff, producers, settle } from './helpers.mjs';

// The iced host's window size (crates/host-iced's WINDOW_WIDTH/HEIGHT) —
// the goldens' viewport.
test.describe('producer parity', () => {
  test.use({ viewport: { width: 360, height: 340 } });

  for (const [demo, paths] of Object.entries(demoPages)) {
    for (const state of Object.keys(demoStates[demo])) {
      test(`${demo} ${state}: the producers render identically`, async ({ context }, testInfo) => {
        const pages = await Promise.all(paths.map(() => context.newPage()));
        try {
          await Promise.all(paths.map((path, i) => demoStates[demo][state](pages[i], path)));
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

          // On divergence, write the evidence into the test's output
          // directory so the CI's test-results artifact carries it: each
          // page's screenshot and its rendered DOM (the demo tree plus the
          // badge) — a pixel diff with the DOM in hand shows what the
          // serializer's view misses.
          if (diverged) {
            for (let i = 0; i < pages.length; i++) {
              const name = producers[i].toLowerCase();
              writeFileSync(testInfo.outputPath(`screenshot-${name}.png`), shots[i]);
              const dom = await pages[i].evaluate(() => ({
                app: document.getElementById('app').outerHTML,
                badge: document.getElementById('badge').textContent,
              }));
              writeFileSync(testInfo.outputPath(`dom-${name}.json`), JSON.stringify(dom, null, 2));
            }
          }
        } finally {
          await Promise.all(pages.map((p) => p.close()));
        }
      });
    }
  }
});
