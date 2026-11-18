// The two browser host pages in a real browser: the TUI host
// (crates/host-tui as wasm, drawing SGR into an xterm.js terminal) and
// the iced host (crates/host-iced as wasm, drawing to a canvas through
// iced's Web runtime).
//
// Each host is one page: ?demo= and ?producer= select the demo and its
// precompiled artifact (the nine combos per host). The component runs on
// the Pulley interpreter; the browser has no JIT.
//
// The TUI assertions read the terminal's rendered text and drive the
// keyboard the way a user would (xterm's write-strings are parsed into
// key names inside the host). The iced assertions read the live canvas's
// pixels: the theme's background, and content drawn on top of it.

import { expect, test } from '@playwright/test';

const demos = ['counter', 'stopwatch', 'todo'];
const producers = ['rust', 'as', 'wat'];

// The terminal's rendered text, one line per row.
async function terminalText(page) {
  return page.evaluate(() => document.querySelector('.xterm-rows')?.innerText ?? '');
}

// Whether any terminal line matches `re`.
const hasLine = (text, re) => text.split('\n').some((l) => re.test(l));

// Focus the terminal's input and send one key, as a user's keydown does.
async function press(page, key) {
  await page.locator('.xterm-helper-textarea').click();
  await page.keyboard.press(key);
}

for (const demo of demos) {
  for (const producer of producers) {
    test(`tui host: ${demo} (${producer})`, async ({ page }) => {
      await page.goto(`/tui/index.html?demo=${demo}&producer=${producer}`);
      // The page's badge is final once the host is running; a failed load
      // leaves a different message instead.
      await expect(page.locator('#badge')).toHaveText(`waui (tui host) — ${demo} (${producer})`, {
        timeout: 30_000,
      });
      // The first frame: the hint bar, always.
      await expect.poll(() => terminalText(page)).toContain('up/down or j/k: focus');

      if (demo === 'counter') {
        expect(await terminalText(page)).toContain('│+│');
        expect(await terminalText(page)).toContain('│-│');
        // The readout line; `\s*$` for the cursor cell, which may trail it.
        expect(hasLine(await terminalText(page), /0\s*$/)).toBe(true);
        // Enter activates the focused button (the ring seats on "+").
        await press(page, 'Enter');
        await expect.poll(async () => hasLine(await terminalText(page), /1\s*$/)).toBe(true);
      } else if (demo === 'stopwatch') {
        const t = await terminalText(page);
        expect(t).toContain('0 ms');
        expect(t).toContain('│start│');
        // Enter starts it: the button flips to pause.
        await press(page, 'Enter');
        await expect
          .poll(async () => {
            const t = await terminalText(page);
            return t.includes('│pause│') && !t.includes('│start│');
          })
          .toBe(true);
      } else {
        const t = await terminalText(page);
        expect(t).toContain('todos');
        expect(t).toContain('walk the dog');
        expect(t).toContain('buy milk');
        expect(t).toContain('2 left');
        // Move the ring to the input field (target 6 of 9) and type.
        await page.locator('.xterm-helper-textarea').click();
        for (let i = 0; i < 6; i++) {
          await page.keyboard.press('ArrowDown');
        }
        await page.keyboard.type('buy coffee');
        // The space is the activation key: with the input focused it is a
        // no-op (the native host's same rule), so the field shows the typed
        // text without it.
        await expect.poll(() => terminalText(page)).toContain('buycoffee');
      }
    });

    test(`iced host: ${demo} (${producer})`, async ({ page }) => {
      await page.goto(`/iced/index.html?demo=${demo}&producer=${producer}`);
      // iced's Web runtime appends its canvas to the body; a failed load
      // leaves an error message instead of one.
      const canvas = page.locator('canvas');
      await canvas.waitFor({ timeout: 30_000 });
      // The window is sized (360x340 at 1x; a display scale can only make
      // the buffer larger), a frame has been presented (the presented frame
      // is opaque; a fresh canvas is transparent), and the frame has
      // content (a resize clears the canvas before the redraw lands, so the
      // count, not the clear, is the ready signal).
      await page.waitForFunction(
        () => {
          const c = document.querySelector('canvas');
          if (!c || c.width < 360 || c.height < 340) return false;
          const tmp = document.createElement('canvas');
          tmp.width = c.width;
          tmp.height = c.height;
          const ctx = tmp.getContext('2d');
          ctx.drawImage(c, 0, 0);
          const d = ctx.getImageData(0, 0, c.width, c.height).data;
          let nonBackground = 0;
          for (let i = 0; i < d.length; i += 4) {
            if (d[i + 3] === 0) continue;
            if (
              Math.abs(d[i] - 26) > 8 ||
              Math.abs(d[i + 1] - 27) > 8 ||
              Math.abs(d[i + 2] - 38) > 8
            ) {
              nonBackground++;
            }
          }
          return nonBackground > 0;
        },
        null,
        { timeout: 30_000 },
      );
      // Sample a grid of the canvas: the corner is the theme's background,
      // and at least one sample differs from it (content is drawn).
      const px = await page.evaluate(() => {
        const c = document.querySelector('canvas');
        const tmp = document.createElement('canvas');
        tmp.width = c.width;
        tmp.height = c.height;
        const ctx = tmp.getContext('2d');
        ctx.drawImage(c, 0, 0);
        const sample = (x, y) => {
          const d = ctx.getImageData(x, y, 1, 1).data;
          return [d[0], d[1], d[2], d[3]];
        };
        const grid = [];
        for (let i = 0; i <= 10; i++) {
          for (let j = 0; j <= 10; j++) {
            grid.push(sample((i * c.width) / 10, (j * c.height) / 10));
          }
        }
        return { size: [c.width, c.height], corner: sample(1, 1), grid };
      });
      expect(px.size[0]).toBeGreaterThanOrEqual(360);
      expect(px.size[1]).toBeGreaterThanOrEqual(340);
      // The theme's background, #1a1b26 (a render backend may shift the
      // presented values a little: WebGPU's color management, tiny-skia's
      // rounding).
      expect(px.corner[3]).toBe(255);
      expect(Math.abs(px.corner[0] - 26)).toBeLessThan(16);
      expect(Math.abs(px.corner[1] - 27)).toBeLessThan(16);
      expect(Math.abs(px.corner[2] - 38)).toBeLessThan(16);
      // Content on top of the background: at least one sample far from it.
      const differs = px.grid.filter(
        (p) =>
          Math.abs(p[0] - px.corner[0]) > 15 ||
          Math.abs(p[1] - px.corner[1]) > 15 ||
          Math.abs(p[2] - px.corner[2]) > 15,
      ).length;
      expect(differs, 'no content was drawn on the canvas').toBeGreaterThan(0);
    });
  }
}
