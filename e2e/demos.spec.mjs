// The nine demo pages (three demos, three producers) in a real browser.
//
// The rendered DOM is asserted against the same node strings the Node
// selftest uses (web/expect.mjs) — the byte-parity gate, extended from the
// ABI to the DOM. The computed styles assert the page's palette mapping
// (web/index.html). The element-identity checks assert app.js's
// in-place-vs-rebuild rule.

import { expect, test } from '@playwright/test';
import { counter, stopwatch, todo } from '../web/expect.mjs';
import {
  demoPages,
  expectView,
  loadDemo,
  palette,
  producers,
  sizes,
  widgetStyle,
} from './helpers.mjs';

for (let i = 0; i < 3; i++) {
  test(`counter (${producers[i]})`, async ({ page }) => {
    await loadDemo(page, demoPages.counter[i]);
    await expectView(page, counter.zero, 'initial view');

    // The palette: the readout inherits the body color, the buttons take
    // their semantic tokens.
    const readout = await widgetStyle(page, 3);
    expect(readout.color).toBe(palette.neutral);
    expect(readout.fontSize).toBe(sizes.large);
    expect((await widgetStyle(page, 2)).color).toBe(palette.positive);
    expect((await widgetStyle(page, 4)).color).toBe(palette.negative);

    // A content-only change restyles in place: the readout element survives
    // the click.
    await page.evaluate(() => {
      document.querySelector('[data-id="3"]').__e2e = true;
    });
    await page.locator('button[data-id="2"]').click();
    await expect(page.locator('[data-id="3"]')).toHaveText('1');
    expect(
      await page.evaluate(() => document.querySelector('[data-id="3"]').__e2e === true),
      'in-place restyle replaced the readout element',
    ).toBe(true);

    // Two decrements: the readout's content and color change, still in
    // place.
    await page.locator('button[data-id="4"]').click();
    await page.locator('button[data-id="4"]').click();
    await expect(page.locator('[data-id="3"]')).toHaveText('-1');
    expect(
      await page.evaluate(() => document.querySelector('[data-id="3"]').__e2e === true),
      'in-place restyle replaced the readout element',
    ).toBe(true);
    expect((await widgetStyle(page, 3)).color).toBe(palette.negative);
    await expectView(page, counter.minusOne, 'after two decrements');
  });

  test(`stopwatch (${producers[i]})`, async ({ page }) => {
    // The host clock is mocked: installed before the load, then paused, so
    // the 250ms wake loop and the now() samples are exact (the selftest
    // uses real time). The jump to a fixed time parks the start sample on
    // a known value; only the deltas matter.
    await page.clock.install({ time: new Date(1_700_000_000_000) });
    await loadDemo(page, demoPages.stopwatch[i]);
    await page.clock.pauseAt(new Date(1_700_000_000_000 + 60_000));
    await expectView(page, stopwatch.idle, 'initial view');

    // Start: the button flips to pause/warning, the readout takes the
    // accent, and the clock advances exactly.
    await page.locator('button[data-id="3"]').click();
    await page.clock.fastForward(1000);
    await expect(page.locator('[data-id="2"]')).toHaveText('1000 ms');
    expect((await widgetStyle(page, 2)).color).toBe(palette.accent);
    expect((await widgetStyle(page, 3)).color).toBe(palette.warning);
    await expect(page.locator('button[data-id="3"]')).toHaveText('pause');

    // Pause: the reading freezes.
    await page.locator('button[data-id="3"]').click();
    await page.clock.fastForward(5000);
    await expect(page.locator('[data-id="2"]')).toHaveText('1000 ms');
    expect((await widgetStyle(page, 2)).color).toBe(palette.neutral);
    await expect(page.locator('button[data-id="3"]')).toHaveText('start');
  });

  test(`todo (${producers[i]})`, async ({ page }) => {
    await loadDemo(page, demoPages.todo[i]);
    await expectView(page, todo.seeded, 'initial view');

    // Type one char at a time, like the selftest: each keystroke is a fresh
    // buffer through on-text, and the field's content re-derives.
    await page.locator('input[data-id="8"]').type('buy coffee');
    await expect(page.locator('input[data-id="8"]')).toHaveValue('buy coffee');
    await expectView(page, todo.typed, 'after typing');

    // Add: the new row at the top of the active column, the count, the
    // cleared field.
    await page.locator('button[data-id="9"]').click();
    await expectView(page, todo.added, 'after add');

    // A structural diff rebuilds the whole tree: even an unchanged row's
    // element is replaced.
    await page.evaluate(() => {
      document.querySelector('[data-id="114"]').__e2e = true;
    });
    await page.locator('button[data-id="105"]').click();
    expect(
      await page.evaluate(() => document.querySelector('[data-id="114"]')?.__e2e === true),
      'the structural diff should have rebuilt the tree',
    ).toBe(false);
    // The old row is gone, its label re-parented under the new row.
    expect(await page.evaluate(() => document.querySelector('[data-id="104"]'))).toBeNull();
    await expectView(page, todo.afterToggle, 'after toggle');

    // Remove.
    await page.locator('button[data-id="103"]').click();
    await expectView(page, todo.afterRemove, 'after remove');

    // Clear done: the count is unchanged, only the done rows go.
    await page.locator('button[data-id="10"]').click();
    await expectView(page, todo.afterClear, 'after clear done');
  });
}
