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
import { demoPages, demoStates, pixelDiff, producers, settle } from './helpers.mjs';

// The iced host's window size (crates/host-iced's WINDOW_WIDTH/HEIGHT) —
// the goldens' viewport.
test.describe('producer parity', () => {
  test.use({ viewport: { width: 360, height: 340 } });

  for (const [demo, paths] of Object.entries(demoPages)) {
    for (const state of Object.keys(demoStates[demo])) {
      test(`${demo} ${state}: the producers render identically`, async ({ context }) => {
        const pages = await Promise.all(paths.map(() => context.newPage()));
        try {
          await Promise.all(paths.map((path, i) => demoStates[demo][state](pages[i], path)));
          await Promise.all(pages.map(settle));
          const shots = await Promise.all(pages.map((p) => p.screenshot()));

          for (let i = 1; i < shots.length; i++) {
            const diff = await pixelDiff(pages[0], shots[0], shots[i]);
            if (diff.sizeA !== null) {
              expect
                .soft(
                  false,
                  `${producers[i]} renders at ${diff.sizeA.join('x')}, the first at ${diff.sizeB.join('x')}`,
                )
                .toBe(true);
            } else {
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
        } finally {
          await Promise.all(pages.map((p) => p.close()));
        }
      });
    }
  }
});
