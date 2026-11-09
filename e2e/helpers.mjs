// Shared helpers for the browser suite.
//
// The rendered DOM carries the same facts as the ABI node the Node selftest
// decodes (web/glue.js): the widget id in data-id, the kind and the style
// tokens as waui-* classes, the content as text or value. The serializer
// below turns the rendered tree back into the selftest's node strings, so
// the suite and the selftest assert on one shared set of expectations
// (web/expect.mjs).

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
const COLORS = ['neutral', 'accent', 'positive', 'negative', 'warning'];
const SIZES = ['small', 'medium', 'large'];
const ALIGNS = ['start', 'center', 'end'];

// The rendered widget tree as the selftest's node strings (pre-order). The
// serializer runs in the page — the token tables are passed in, because an
// evaluate callback is serialized without its module scope.
export async function domTree(page) {
  return page.evaluate(
    ({ colors, sizes, aligns }) => {
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
        if (c.contains('waui-button') || c.contains('waui-input') || c.contains('waui-text')) {
          const kind = c.contains('waui-button')
            ? 'button'
            : c.contains('waui-input')
              ? 'input'
              : 'text';
          const field = kind === 'button' ? 'label' : 'content';
          const value = kind === 'input' ? el.value : el.textContent;
          const style = `style:${token(c, 'waui-', colors, 'neutral')}:${token(c, 'waui-', sizes, 'medium')}`;
          s = `${kind}{id:${id},parent:${parentId},${field}:"${value}",${style}}`;
        } else {
          const dir = c.contains('waui-column') ? 'column' : 'row';
          const align = token(c, 'waui-align-', aligns, 'center');
          s = `container{id:${id},parent:${parentId},dir:${dir},align:${align}}`;
        }
        return [s, ...[...el.children].flatMap((child) => node(child, id))];
      };
      const out = [];
      for (const root of document.getElementById('app').children) {
        out.push(...node(root, null));
      }
      return out;
    },
    { colors: COLORS, sizes: SIZES, aligns: ALIGNS },
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
