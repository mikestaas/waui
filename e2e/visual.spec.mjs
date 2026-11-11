// Screenshot goldens: the pixel layer.
//
// Font rendering is platform-specific, so these compare only in CI (the
// committed goldens were generated on the CI runner's image); locally the
// file skips. Regenerate after a runner image change with the workflow's
// update_snapshots input. A local `--update-snapshots` run writes this
// machine's rendering — do not commit it.
//
// The demo pages render in a 360x340 viewport — the iced host's window
// size (crates/host-iced's WINDOW_WIDTH/HEIGHT) — so the goldens show the
// demos the way the windowed host does. The desktop keeps the default
// 1280x720.
//
// Each demo page walks its states, as the `--dump` frame goldens do: the
// counter at 0, 1, and -3 (a fresh load per state), the stopwatch paused
// and running (the running readout is a fixed 1000 ms on the mocked
// clock), and the todo seeded and after an add.

import { expect, test } from '@playwright/test';
import { loadDemo } from './helpers.mjs';

test.skip(!process.env.CI, 'screenshot goldens are CI-only (font rendering is platform-specific)');

// The iced host's window size (crates/host-iced's WINDOW_WIDTH/HEIGHT).
const DEMO_VIEWPORT = { width: 360, height: 340 };

// Settle the frame before a screenshot: move the mouse off the last-clicked
// widget (its :hover style) and drop focus (the browser's focus ring).
async function settle(page) {
  await page.mouse.move(0, 0);
  await page.evaluate(() => document.activeElement?.blur());
}

test.describe('demo pages', () => {
  test.use({ viewport: DEMO_VIEWPORT });

  for (const [name, path] of [
    ['counter', '/demo-counter.html'],
    ['counter-as', '/demo-counter-as.html'],
    ['counter-wat', '/demo-counter-wat.html'],
    ['stopwatch', '/demo-stopwatch.html'],
    ['stopwatch-as', '/demo-stopwatch-as.html'],
    ['stopwatch-wat', '/demo-stopwatch-wat.html'],
    ['todo', '/demo-todo.html'],
    ['todo-as', '/demo-todo-as.html'],
    ['todo-wat', '/demo-todo-wat.html'],
  ]) {
    if (name.startsWith('counter')) {
      // The readout's color follows the value's sign: 0 and 1 are neutral,
      // -3 takes the negative tone. A fresh load per state, as the --dump
      // tests (a page's component keeps its state across clicks).
      test(`${name} page`, async ({ page }) => {
        await loadDemo(page, path);
        await expect(page).toHaveScreenshot(`${name}-0.png`);

        // + + - = 1.
        await loadDemo(page, path);
        await page.locator('button[data-id="2"]').click();
        await page.locator('button[data-id="2"]').click();
        await page.locator('button[data-id="4"]').click();
        await settle(page);
        await expect(page).toHaveScreenshot(`${name}-1.png`);

        // - - - = -3.
        await loadDemo(page, path);
        await page.locator('button[data-id="4"]').click();
        await page.locator('button[data-id="4"]').click();
        await page.locator('button[data-id="4"]').click();
        await settle(page);
        await expect(page).toHaveScreenshot(`${name}--3.png`);
      });
    } else if (name.startsWith('stopwatch')) {
      // The host clock is mocked: installed before the load, then paused, so
      // the now() samples are exact (the demos spec uses the same).
      test(`${name} page`, async ({ page }) => {
        await page.clock.install({ time: new Date(1_700_000_000_000) });
        await loadDemo(page, path);
        await page.clock.pauseAt(new Date(1_700_000_000_000 + 60_000));
        await expect(page).toHaveScreenshot(`${name}-paused.png`);

        // Start, then let a second pass: the readout is a fixed 1000 ms.
        await page.locator('button[data-id="3"]').click();
        await page.clock.fastForward(1000);
        await expect(page.locator('[data-id="2"]')).toHaveText('1000 ms');
        await settle(page);
        await expect(page).toHaveScreenshot(`${name}-running.png`);
      });
    } else {
      // The todo's states: the seeded list, then a typed add — the new row
      // at the top of the active column, the cleared field, the count.
      test(`${name} page`, async ({ page }) => {
        await loadDemo(page, path);
        await expect(page).toHaveScreenshot(`${name}-seeded.png`);

        await page.locator('input[data-id="8"]').type('buy coffee');
        await page.locator('button[data-id="9"]').click();
        await settle(page);
        await expect(page).toHaveScreenshot(`${name}-added.png`);
      });
    }
  }
});

test('desktop with two windows', async ({ page }) => {
  await page.goto('/');
  const tiles = page.locator('#grid .tile');
  await tiles.nth(0).dblclick(); // Counter (Rust)
  await tiles.nth(7).dblclick(); // Todo (AssemblyScript)
  // Both windows' demos are static at rest; wait for the top one to boot.
  await expect(
    page.locator('.window').last().frameLocator('iframe').locator('[data-id="2"]'),
  ).toHaveText('todos');
  await expect(page).toHaveScreenshot('desktop.png');
});
