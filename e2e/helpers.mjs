// Shared helpers for the browser suite.
//
// The rendered DOM carries the same facts as the ABI node the Node selftest
// decodes (web/glue.js): the widget id in data-id, the kind and the style
// tokens as waui-* classes, the content as text or value, the input's width
// in data-width, the image's src and size, and the link's label and href.
// The serializer below turns the rendered tree back into the selftest's node
// strings, so the suite and the selftest assert on one shared set of
// expectations (web/expect.mjs).

import { expect } from '@playwright/test';

// The page's palette (web/index.html), as computed rgb() values.
export const palette = {
  neutral: 'rgb(192, 202, 245)', // #c0caf5, inherited from the body
  accent: 'rgb(122, 162, 247)', // #7aa2f7
  positive: 'rgb(158, 206, 106)', // #9ece6a
  negative: 'rgb(247, 118, 142)', // #f7768e
  warning: 'rgb(224, 175, 104)', // #e0af68
};

// The page's font sizes (web/index.html: a 15px base, 0.85em/1em/1.8em).
export const sizes = {
  small: '12.75px',
  medium: '15px',
  large: '27px',
};

// The token tables, in the WIT case order of waui:ui/widget (the same
// tables as web/glue.js). Unknown values degrade to the middle entry, as
// the host does.
const COLORS = ['neutral', 'accent', 'positive', 'negative', 'warning', 'dim'];
const SIZES = ['small', 'medium', 'large'];
const ALIGNS = ['start', 'center', 'end'];
const STRETCHES = ['none', 'main', 'cross', 'both'];
const OVERFLOWS = ['clip', 'scroll'];
const BACKGROUNDS = ['none', 'panel', 'bar'];
const BORDERS = ['none', 'line', 'ring'];
const RADII = ['none', 'small', 'large'];
const SHADOWS = ['none', 'raised'];
const CURSORS = ['default', 'pointer', 'grab', 'grabbing', 'resize'];

// The rendered widget tree as the selftest's node strings (pre-order). The
// serializer runs in the page — the token tables are passed in, because an
// evaluate callback is serialized without its module scope.
//
// `panelHandles` names the content slots of open panels (the windowing host
// seats a panel's tree in the slot whose id is the handle): such a slot
// serializes childless, because the panel's nodes belong to the panel, not
// to this tree.
export async function domTree(page, { panelHandles = [] } = {}) {
  return page.evaluate(
    ({
      colors,
      sizes,
      aligns,
      stretches,
      overflows,
      backgrounds,
      borders,
      radii,
      shadows,
      cursors,
      panelHandles,
    }) => {
      // The token class for one table: the first class present, or the
      // fallback (the host degrades unknown values the same way).
      const token = (classes, prefix, table, fallback) => {
        for (const t of table) {
          if (classes.contains(`${prefix}${t}`)) return t;
        }
        return fallback;
      };
      // One rendered widget element into the selftest's node string
      // (web/selftest.mjs's node()), followed by its descendants in
      // pre-order — the flat list the contract exports.
      const node = (el, parentId) => {
        const id = el.dataset.id !== undefined ? Number(el.dataset.id) : null;
        const c = el.classList;
        let s;
        if (c.contains('waui-image')) {
          // An image has no style token: the raw `src` attribute (not the
          // resolved `el.src` URL) and the size, from the dataset app.js
          // writes.
          const src = el.getAttribute('src') ?? '';
          const width = Number(el.dataset.width ?? 0);
          const height = Number(el.dataset.height ?? 0);
          s = `image{id:${id},parent:${parentId},src:"${src}",width:${width},height:${height}}`;
        } else if (
          c.contains('waui-button') ||
          c.contains('waui-input') ||
          c.contains('waui-text')
        ) {
          const kind = c.contains('waui-button')
            ? 'button'
            : c.contains('waui-input')
              ? 'input'
              : 'text';
          const field = kind === 'button' ? 'label' : 'content';
          const value = kind === 'input' ? el.value : el.textContent;
          const style = `style:${token(c, 'waui-', colors, 'neutral')}:${token(c, 'waui-', sizes, 'medium')}`;
          // The input's width (data-width, set by web/app.js); the other
          // leaves carry none in the selftest's string.
          const width = kind === 'input' ? `,width:${Number(el.dataset.width ?? 0)}` : '';
          s = `${kind}{id:${id},parent:${parentId},${field}:"${value}",${style}${width}}`;
        } else if (c.contains('waui-link')) {
          // A link: the label (its text) and the target (the raw `href`
          // attribute, as app.js sets it), plus the style tokens.
          const style = `style:${token(c, 'waui-', colors, 'neutral')}:${token(c, 'waui-', sizes, 'medium')}`;
          const href = el.getAttribute('href') ?? '';
          s = `link{id:${id},parent:${parentId},label:"${el.textContent}",href:"${href}",${style}}`;
        } else {
          // dir and align are always present; the other fields only when
          // non-default (web/selftest.mjs's node(), the same split).
          const dir = c.contains('waui-column') ? 'column' : 'row';
          const align = token(c, 'waui-align-', aligns, 'center');
          const d = el.dataset;
          const parts = [`id:${id}`, `parent:${parentId}`, `dir:${dir}`, `align:${align}`];
          if (c.contains('waui-positioned')) parts.push('pos:1');
          if (Number(d.x ?? 0) > 0) parts.push(`x:${d.x}`);
          if (Number(d.y ?? 0) > 0) parts.push(`y:${d.y}`);
          if (Number(d.w ?? 0) > 0) parts.push(`w:${d.w}`);
          if (Number(d.h ?? 0) > 0) parts.push(`h:${d.h}`);
          if (Number(d.z ?? 0) > 0) parts.push(`z:${d.z}`);
          if (Number(d.gap ?? 0) > 0) parts.push(`gap:${d.gap}`);
          if (Number(d.padding ?? 0) > 0) parts.push(`pad:${d.padding}`);
          const stretch = token(c, 'waui-stretch-', stretches, 'none');
          if (stretch !== 'none') parts.push(`stretch:${stretch}`);
          const overflow = token(c, 'waui-overflow-', overflows, 'clip');
          if (overflow !== 'clip') parts.push(`overflow:${overflow}`);
          const bg = token(c, 'waui-bg-', backgrounds, 'none');
          if (bg !== 'none') parts.push(`bg:${bg}`);
          const border = token(c, 'waui-border-', borders, 'none');
          if (border !== 'none') parts.push(`border:${border}`);
          const radius = token(c, 'waui-radius-', radii, 'none');
          if (radius !== 'none') parts.push(`radius:${radius}`);
          const shadow = token(c, 'waui-shadow-', shadows, 'none');
          if (shadow !== 'none') parts.push(`shadow:${shadow}`);
          const cursor = token(c, 'waui-cursor-', cursors, 'default');
          if (cursor !== 'default') parts.push(`cursor:${cursor}`);
          s = `container{${parts.join(',')}}`;
        }
        // A content slot with a seated panel serializes childless (the
        // panel's nodes are the panel's tree, not this one's).
        const kids = panelHandles.includes(id) ? [] : [...el.children];
        return [s, ...kids.flatMap((child) => node(child, id))];
      };
      const out = [];
      for (const root of document.getElementById('app').children) {
        out.push(...node(root, null));
      }
      return out;
    },
    {
      colors: COLORS,
      sizes: SIZES,
      aligns: ALIGNS,
      stretches: STRETCHES,
      overflows: OVERFLOWS,
      backgrounds: BACKGROUNDS,
      borders: BORDERS,
      radii: RADII,
      shadows: SHADOWS,
      cursors: CURSORS,
      panelHandles,
    },
  );
}

// Assert the rendered tree equals the expected node strings
// (web/expect.mjs).
export async function expectView(page, expected, message) {
  const actual = await domTree(page);
  expect(actual, message).toEqual(expected);
}

// The page's badge is set after the component instantiated (web/app.js);
// a failed module load leaves a different message instead.
export async function loadDemo(page, path) {
  await page.goto(path);
  await page.waitForFunction(
    () => document.getElementById('badge').textContent.includes('renderer: static html'),
    { timeout: 15_000 },
  );
}

// The computed style of the widget with `id`, for the palette and layout
// assertions.
export async function widgetStyle(page, id) {
  return page.evaluate((id) => {
    const el = document.querySelector(`[data-id="${id}"]`);
    const cs = getComputedStyle(el);
    return {
      color: cs.color,
      fontSize: cs.fontSize,
      backgroundColor: cs.backgroundColor,
      borderColor: cs.borderColor,
      flexDirection: cs.flexDirection,
      alignItems: cs.alignItems,
    };
  }, id);
}

// The site's single index page, per demo: the three producers, in order
// (the Rust page, the AssemblyScript one, the WAT one). ?demo= and
// ?producer= select the module, as on the iced and tui host pages.
export const demoPages = {
  counter: [
    '/index.html?demo=counter&producer=rust',
    '/index.html?demo=counter&producer=as',
    '/index.html?demo=counter&producer=wat',
  ],
  stopwatch: [
    '/index.html?demo=stopwatch&producer=rust',
    '/index.html?demo=stopwatch&producer=as',
    '/index.html?demo=stopwatch&producer=wat',
  ],
  todo: [
    '/index.html?demo=todo&producer=rust',
    '/index.html?demo=todo&producer=as',
    '/index.html?demo=todo&producer=wat',
  ],
};

// The producer labels, in demoPages order.
export const producers = ['Rust', 'AssemblyScript', 'WAT'];

// The desktop, per producer: the `demo-desktop` component (the index page's
// default demo), in its three source languages, on the windowing host.
export const desktopPages = [
  '/index.html?demo=desktop&producer=rust',
  '/index.html?demo=desktop&producer=as',
  '/index.html?demo=desktop&producer=wat',
];

// Wait for the desktop page's grid to populate (the boot is async: the
// demos are pre-compiled, then the component runs). The tiles are the
// component's cursor-pointer nodes (the demos' own nodes use no pointer).
export async function loadDesktop(page, path) {
  await page.goto(path);
  await page.locator('#app .waui-cursor-pointer').first().waitFor();
}

// The window count (the component's menu-bar node, id 4, under the menu
// bar, id 2: a panel's tree can carry the same id, so scope to the bar).
const windowCount = (page) => page.locator('[data-id="2"] [data-id="4"]');

// The desktop states, as a setup function per state: drive a page into the
// state from a fresh load (a page's component keeps its state across
// clicks, so each state reloads). The windows open deterministic demos
// (the counters, at rest), so the states are screenshot-stable.
export const desktopStates = {
  '0 windows': async (page, path) => {
    await loadDesktop(page, path);
  },
  // The first tile (Counter, Rust).
  '1 window': async (page, path) => {
    await loadDesktop(page, path);
    await page.locator('#app .waui-cursor-pointer').nth(0).dblclick();
    await expect(windowCount(page)).toHaveText('1 window');
  },
  // The first two tiles (both counters, at rest).
  '2 windows': async (page, path) => {
    await loadDesktop(page, path);
    await page.locator('#app .waui-cursor-pointer').nth(0).dblclick();
    await page.locator('#app .waui-cursor-pointer').nth(1).dblclick();
    await expect(windowCount(page)).toHaveText('2 windows');
  },
};

// The demo states, as a setup function per state: drive a page into the
// state from a fresh load (a page's component keeps its state across
// clicks, so each state reloads). The visual and parity specs share these.
// The stopwatch's clock is mocked inside (installed before the load, then
// paused), so its readout is exact.
export const demoStates = {
  counter: {
    0: async (page, path) => {
      await loadDemo(page, path);
    },
    // + + - = 1.
    1: async (page, path) => {
      await loadDemo(page, path);
      await page.locator('button[data-id="2"]').click();
      await page.locator('button[data-id="2"]').click();
      await page.locator('button[data-id="4"]').click();
    },
    // - - - = -3.
    '-3': async (page, path) => {
      await loadDemo(page, path);
      await page.locator('button[data-id="4"]').click();
      await page.locator('button[data-id="4"]').click();
      await page.locator('button[data-id="4"]').click();
    },
  },
  stopwatch: {
    paused: async (page, path) => {
      await page.clock.install({ time: new Date(1_700_000_000_000) });
      await loadDemo(page, path);
      await page.clock.pauseAt(new Date(1_700_000_000_000 + 60_000));
    },
    // Started, then a second passed: the readout is a fixed 1000 ms.
    running: async (page, path) => {
      await page.clock.install({ time: new Date(1_700_000_000_000) });
      await loadDemo(page, path);
      await page.clock.pauseAt(new Date(1_700_000_000_000 + 60_000));
      await page.locator('button[data-id="3"]').click();
      await page.clock.fastForward(1000);
    },
  },
  todo: {
    seeded: async (page, path) => {
      await loadDemo(page, path);
    },
    // A typed add: the new row at the top, the cleared field, the count.
    added: async (page, path) => {
      await loadDemo(page, path);
      await page.locator('input[data-id="8"]').type('buy coffee');
      await page.locator('button[data-id="9"]').click();
    },
  },
};

// Settle the frame before a screenshot: move the mouse off the last-clicked
// widget (its :hover style) and drop focus (the browser's focus ring). The
// desktop's icons are <img> fetches (the site's registry); wait for them,
// or the screenshot catches a tile mid-icon.
export async function settle(page) {
  await page.mouse.move(0, 0);
  await page.evaluate(() => document.activeElement?.blur());
  await page.waitForFunction(() => [...document.images].every((i) => i.complete));
}

// The per-channel tolerance of the pixel comparison. One: chromium's
// anti-aliasing of an edge is not bit-stable across contexts, so identical
// content can differ by a step at an edge. A real divergence (a color, a
// glyph, a layout shift) differs by more.
const PIXEL_TOL = 1;

// Compare two screenshots pixel by pixel: a canvas in the browser decodes
// the PNGs (the suite has no PNG dependency). Returns the differing-pixel
// count and their bounding box, or a size mismatch.
export async function pixelDiff(page, a, b) {
  return page.evaluate(
    ([a, b, tol]) => {
      const load = (dataUrl) =>
        new Promise((resolve, reject) => {
          const img = new Image();
          img.onload = () => resolve(img);
          img.onerror = () => reject(new Error('failed to decode the screenshot'));
          img.src = dataUrl;
        });
      return Promise.all([load(a), load(b)]).then(([ia, ib]) => {
        if (ia.width !== ib.width || ia.height !== ib.height) {
          return {
            sizeA: [ia.width, ia.height],
            sizeB: [ib.width, ib.height],
            n: 0,
            box: null,
          };
        }
        const c = document.createElement('canvas');
        c.width = ia.width;
        c.height = ia.height;
        const ctx = c.getContext('2d');
        ctx.drawImage(ia, 0, 0);
        const pa = ctx.getImageData(0, 0, ia.width, ia.height).data;
        ctx.clearRect(0, 0, ia.width, ia.height);
        ctx.drawImage(ib, 0, 0);
        const pb = ctx.getImageData(0, 0, ia.width, ib.height).data;
        let n = 0;
        let minX = ia.width,
          minY = ia.height,
          maxX = -1,
          maxY = -1;
        for (let i = 0; i < pa.length; i += 4) {
          if (
            Math.abs(pa[i] - pb[i]) > tol ||
            Math.abs(pa[i + 1] - pb[i + 1]) > tol ||
            Math.abs(pa[i + 2] - pb[i + 2]) > tol
          ) {
            n += 1;
            const x = (i / 4) % ia.width;
            const y = (i / 4 / ia.width) | 0;
            if (x < minX) minX = x;
            if (y < minY) minY = y;
            if (x > maxX) maxX = x;
            if (y > maxY) maxY = y;
          }
        }
        return { sizeA: null, sizeB: null, n, box: n > 0 ? [minX, minY, maxX, maxY] : null };
      });
    },
    [
      `data:image/png;base64,${Buffer.from(a).toString('base64')}`,
      `data:image/png;base64,${Buffer.from(b).toString('base64')}`,
      PIXEL_TOL,
    ],
  );
}
