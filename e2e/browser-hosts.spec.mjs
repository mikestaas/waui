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

      if (demo === 'todo') {
        // The delete symbol (✕) is the danger color. The browser loads no
        // system fonts, so the ✕ only renders if the bundled symbol font
        // loaded — the live-window case the byte-parity selftest does not
        // cover. With no symbol font the glyph falls back to Fira Sans's
        // notdef, a tall narrow box; the real ✕ is square. Aspect ratio
        // separates the two (both have a filled center). The three delete
        // buttons sit in the rows, above the footer's red "clear done", so a
        // square, filled-center component above 0.58·H isolates them.
        const countX = () =>
          page.evaluate(() => {
            const c = document.querySelector('canvas');
            const W = c.width,
              H = c.height;
            const t = document.createElement('canvas');
            t.width = W;
            t.height = H;
            const ctx = t.getContext('2d');
            ctx.drawImage(c, 0, 0);
            const d = ctx.getImageData(0, 0, W, H).data;
            const isRed = (x, y) => {
              const i = (y * W + x) * 4;
              return d[i] > 100 && d[i] - d[i + 1] > 55 && d[i] - d[i + 2] > 40;
            };
            const hasInk = (x, y) => {
              for (let dy = -1; dy <= 1; dy++)
                for (let dx = -1; dx <= 1; dx++) {
                  const nx = x + dx,
                    ny = y + dy;
                  if (nx < 0 || ny < 0 || nx >= W || ny >= H) continue;
                  if (isRed(nx, ny)) return true;
                }
              return false;
            };
            const seen = new Uint8Array(W * H);
            const dxs = [1, -1, 0, 0, 1, -1, 1, -1];
            const dys = [0, 0, 1, -1, 1, 1, -1, -1];
            let n = 0;
            for (let y = 0; y < H; y++)
              for (let x = 0; x < W; x++) {
                if (seen[y * W + x] || !isRed(x, y)) continue;
                const st = [x, y];
                seen[y * W + x] = 1;
                let minX = x,
                  maxX = x,
                  minY = y,
                  maxY = y;
                while (st.length) {
                  const cy = st.pop(),
                    cx = st.pop();
                  if (cx < minX) minX = cx;
                  if (cx > maxX) maxX = cx;
                  if (cy < minY) minY = cy;
                  if (cy > maxY) maxY = cy;
                  for (let k = 0; k < 8; k++) {
                    const nx = cx + dxs[k],
                      ny = cy + dys[k];
                    if (nx < 0 || ny < 0 || nx >= W || ny >= H) continue;
                    if (seen[ny * W + nx] || !isRed(nx, ny)) continue;
                    seen[ny * W + nx] = 1;
                    st.push(nx, ny);
                  }
                }
                const w = maxX - minX + 1,
                  h = maxY - minY + 1;
                if (w < 6 || h < 6) continue;
                const cx = (minX + maxX) / 2,
                  cy = (minY + maxY) / 2;
                const aspect = w / h;
                if (
                  cy < 0.58 * H &&
                  aspect >= 0.5 &&
                  aspect <= 2 &&
                  hasInk(Math.round(cx), Math.round(cy))
                )
                  n++;
              }
            return n;
          });
        await expect.poll(countX, { timeout: 15_000 }).toBeGreaterThanOrEqual(2);
      }
    });
  }
}

// The desktop: the registry's grid, and a window opened from a tile. The
// page fetches the nine entry artifacts in the background (the host has no
// filesystem); wait for them, then activate the first tile and expect the
// window. One producer: the open path is the host's, the producer only
// changes the desktop's own rendering.
test('tui host: desktop (rust) opens a window', async ({ page }) => {
  await page.goto('/tui/index.html?demo=desktop&producer=rust');
  await expect(page.locator('#badge')).toHaveText('waui (tui host) — desktop (rust)', {
    timeout: 30_000,
  });
  await page.waitForFunction(() => window.entriesReady === true, null, { timeout: 30_000 });
  // The ring starts on the first tile; Enter on a tile synthesizes the
  // double-click that opens its entry.
  await press(page, 'Enter');
  // The menu bar counts the open window.
  await expect.poll(() => terminalText(page)).toContain('1 window');
});

test('iced host: desktop (rust) opens a window', async ({ page }) => {
  await page.goto('/iced/index.html?demo=desktop&producer=rust');
  const canvas = page.locator('canvas');
  await canvas.waitFor({ timeout: 30_000 });
  // The app canvas (960x600 at 1x), with the grid drawn.
  await page.waitForFunction(
    () => {
      const c = document.querySelector('canvas');
      if (!c || c.width < 960 || c.height < 600) return false;
      const tmp = document.createElement('canvas');
      tmp.width = c.width;
      tmp.height = c.height;
      const ctx = tmp.getContext('2d');
      ctx.drawImage(c, 0, 0);
      const d = ctx.getImageData(0, 0, c.width, c.height).data;
      let nonBackground = 0;
      for (let i = 0; i < d.length; i += 4) {
        if (d[i + 3] === 0) continue;
        if (Math.abs(d[i] - 26) > 8 || Math.abs(d[i + 1] - 27) > 8 || Math.abs(d[i + 2] - 38) > 8) {
          nonBackground++;
        }
      }
      return nonBackground > 0;
    },
    null,
    { timeout: 30_000 },
  );
  // The entry artifacts land in the background; wait for them.
  await page.waitForFunction(() => window.entriesReady === true, null, { timeout: 30_000 });
  // The window's cascade slot (top-left, 24x48): background before the
  // double-click, the window's body fill after. The canvas is the viewport,
  // so layout coordinates scale by the buffer's device ratio.
  const countContent = () =>
    page.evaluate(() => {
      const c = document.querySelector('canvas');
      const s = c.width / window.innerWidth;
      const tmp = document.createElement('canvas');
      tmp.width = c.width;
      tmp.height = c.height;
      const ctx = tmp.getContext('2d');
      ctx.drawImage(c, 0, 0);
      const d = ctx.getImageData(
        Math.floor(30 * s),
        Math.floor(200 * s),
        Math.floor(40 * s),
        Math.floor(120 * s),
      ).data;
      let n = 0;
      for (let i = 0; i < d.length; i += 4) {
        if (d[i + 3] === 0) continue;
        if (
          Math.abs(d[i] - 26) > 15 ||
          Math.abs(d[i + 1] - 27) > 15 ||
          Math.abs(d[i + 2] - 38) > 15
        ) {
          n++;
        }
      }
      return n;
    });
  expect(await countContent(), 'the probe box should be background before the open').toBe(0);
  // The first tile (the counter): the grid is top-packed and centered, so
  // its icon center sits at (464, 108) in the 1280-wide viewport. The whole
  // tile is the double-click target; the point is its icon's center.
  const box = await canvas.boundingBox();
  await page.mouse.dblclick(box.x + 464, box.y + 108);
  await expect.poll(countContent).toBeGreaterThan(0);
});

test('iced host: desktop (rust) drop shadow stays stable while mousing inside a window', async ({
  page,
}) => {
  await page.goto('/iced/index.html?demo=desktop&producer=rust');
  const canvas = page.locator('canvas');
  await canvas.waitFor({ timeout: 30_000 });
  await page.waitForFunction(() => window.entriesReady === true, null, { timeout: 30_000 });

  // A new window opens at (24, 48) sized 300x380. The shadow extends
  // outside it: offset y=18, blur 50. This region is shadow only.
  const shadowRegion = { x: 350, y: 300, width: 15, height: 15 };
  const readShadow = () =>
    page.evaluate((region) => {
      const c = document.querySelector('canvas');
      const s = c.width / window.innerWidth;
      const tmp = document.createElement('canvas');
      tmp.width = c.width;
      tmp.height = c.height;
      const ctx = tmp.getContext('2d');
      ctx.drawImage(c, 0, 0);
      const d = ctx.getImageData(
        Math.floor(region.x * s),
        Math.floor(region.y * s),
        Math.floor(region.width * s),
        Math.floor(region.height * s),
      ).data;
      return Array.from(d);
    }, shadowRegion);

  // Window content at (30,200)..(70,320), as in the opens-a-window test.
  const countContent = () =>
    page.evaluate(() => {
      const c = document.querySelector('canvas');
      const s = c.width / window.innerWidth;
      const tmp = document.createElement('canvas');
      tmp.width = c.width;
      tmp.height = c.height;
      const ctx = tmp.getContext('2d');
      ctx.drawImage(c, 0, 0);
      const d = ctx.getImageData(
        Math.floor(30 * s),
        Math.floor(200 * s),
        Math.floor(40 * s),
        Math.floor(120 * s),
      ).data;
      let n = 0;
      for (let i = 0; i < d.length; i += 4) {
        if (d[i + 3] === 0) continue;
        if (
          Math.abs(d[i] - 26) > 15 ||
          Math.abs(d[i + 1] - 27) > 15 ||
          Math.abs(d[i + 2] - 38) > 15
        )
          n++;
      }
      return n;
    });

  const box = await canvas.boundingBox();
  await page.mouse.dblclick(box.x + 464, box.y + 108);
  await expect.poll(countContent).toBeGreaterThan(0);

  // Let the window's first frame settle.
  await page.waitForTimeout(100);
  const before = await readShadow();

  // Mouse around inside the open window without dragging it.
  const moves = [
    [100, 200],
    [110, 200],
    [100, 210],
    [120, 240],
    [100, 300],
    [50, 100],
    [200, 100],
    [150, 250],
    [100, 200],
  ];
  for (const [x, y] of moves) {
    await page.mouse.move(box.x + x, box.y + y);
  }
  await page.waitForTimeout(100);

  expect(await readShadow()).toEqual(before);
});
