// The component desktop (web/desktop.html): the `demo-desktop` component on
// the windowing side of the `app` world, with the windowing host. Unlike the
// static launcher (desktop.spec.mjs, `/`), each window embeds a demo
// *component* rendered in place (`.window-content`), not in an iframe.
//
// The grid is the component's tree: nine tiles (`.tile`), each an icon, a
// label (`.tile-name`), and a caption (`.tile-lang`). The window count is the
// component's menu-bar node, restyled into `#window-count`.

import { expect, test } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  await page.goto('/desktop.html');
  // The boot is async (the demos are pre-compiled, then the component runs);
  // wait for the grid to populate.
  await expect(page.locator('#grid .tile')).toHaveCount(9);
});

// The z-order counter (the host's raise()).
const z = (win) => win.evaluate((el) => Number(el.dataset.z));

test('renders the nine demo tiles', async ({ page }) => {
  const tiles = page.locator('#grid .tile');
  await expect(tiles).toHaveCount(9);
  await expect(tiles.nth(0).locator('.tile-name')).toHaveText('Counter');
  await expect(tiles.nth(0).locator('.tile-lang')).toHaveText('Rust');
  await expect(tiles.nth(1).locator('.tile-lang')).toHaveText('AssemblyScript');
  await expect(tiles.nth(2).locator('.tile-lang')).toHaveText('WAT');
  await expect(tiles.nth(3).locator('.tile-name')).toHaveText('Stopwatch');
  await expect(tiles.nth(5).locator('.tile-lang')).toHaveText('WAT');
  await expect(tiles.nth(6).locator('.tile-name')).toHaveText('Todo');
  await expect(tiles.nth(8).locator('.tile-lang')).toHaveText('WAT');
  await expect(page.locator('#window-count')).toHaveText('0 windows');
});

test('double-click opens a window with the embedded demo', async ({ page }) => {
  await page.locator('#grid .tile').nth(0).dblclick(); // Counter, Rust
  const win = page.locator('.window');
  await expect(win).toHaveCount(1);
  await expect(win.locator('.titlebar-title')).toHaveText('Counter');
  // The demo runs in place: the counter's display reads "0".
  await expect(win.locator('.window-content')).toHaveText(/\b0\b/);
  await expect(page.locator('#window-count')).toHaveText('1 window');
});

test('the embedded demo is interactive', async ({ page }) => {
  await page.locator('#grid .tile').nth(0).dblclick(); // Counter, Rust
  const win = page.locator('.window');
  const display = win
    .locator('.window-content')
    .locator('div')
    .filter({ hasText: /^\d+$/ })
    .first();
  await expect(display).toHaveText('0');
  // The "+" button is the first button in the content.
  await win.locator('.window-content button').first().click();
  await expect(display).toHaveText('1');
  await win.locator('.window-content button').first().click();
  await expect(display).toHaveText('2');
});

test('typing in the embedded todo keeps the input focused', async ({ page }) => {
  await page.locator('#grid .tile').nth(6).dblclick(); // Todo, Rust
  const win = page.locator('.window');
  const input = win.locator('.window-content input');
  await input.click();
  await input.type('buy coffee');
  // A content-only change restyles in place: the element is not replaced, so
  // focus and caret survive every keystroke.
  await expect(input).toBeFocused();
  await expect(input).toHaveValue('buy coffee');
});

test('windows raise on click', async ({ page }) => {
  const tiles = page.locator('#grid .tile');
  await tiles.nth(0).dblclick(); // counter
  await tiles.nth(3).dblclick(); // stopwatch
  await expect(page.locator('#window-count')).toHaveText('2 windows');
  const counterWin = page.locator('.window').first();
  const stopWin = page.locator('.window').last();
  expect(await z(stopWin)).toBeGreaterThan(await z(counterWin));
  // A click on the counter's title bar raises it above the stopwatch.
  await counterWin.locator('.titlebar').click();
  expect(await z(counterWin)).toBeGreaterThan(await z(stopWin));
});

test('close button and Escape close windows', async ({ page }) => {
  const tiles = page.locator('#grid .tile');
  await tiles.nth(0).dblclick();
  await tiles.nth(3).dblclick();
  await expect(page.locator('#window-count')).toHaveText('2 windows');

  // The frontmost (last opened) closes by its button.
  await page.locator('.window').last().locator('.close').click();
  await expect(page.locator('#window-count')).toHaveText('1 window');

  // Escape closes the frontmost. Focus the wallpaper first (a key pressed
  // while a window's content has focus would go to the demo).
  await page.locator('body').click({ position: { x: 1200, y: 650 } });
  await page.keyboard.press('Escape');
  await expect(page.locator('#window-count')).toHaveText('0 windows');
});

test('drag moves the window', async ({ page }) => {
  await page.locator('#grid .tile').nth(0).dblclick();
  const win = page.locator('.window');
  const before = await win.boundingBox();
  const bar = await win.locator('.titlebar').boundingBox();
  await page.mouse.move(bar.x + bar.width / 2, bar.y + bar.height / 2);
  await page.mouse.down();
  await page.mouse.move(bar.x + bar.width / 2 + 80, bar.y + bar.height / 2 + 50, { steps: 8 });
  await page.mouse.up();
  const after = await win.boundingBox();
  expect(Math.abs(after.x - before.x - 80)).toBeLessThan(1);
  expect(Math.abs(after.y - before.y - 50)).toBeLessThan(1);
});

test('the grip resizes, clamped to a minimum', async ({ page }) => {
  await page.locator('#grid .tile').nth(0).dblclick(); // 300x380
  const win = page.locator('.window');
  const size = () => win.evaluate((el) => [el.offsetWidth, el.offsetHeight]);

  // Grow.
  let grip = await win.locator('.grip').boundingBox();
  await page.mouse.move(grip.x + grip.width / 2, grip.y + grip.height / 2);
  await page.mouse.down();
  await page.mouse.move(grip.x + grip.width / 2 + 100, grip.y + grip.height / 2 + 80, { steps: 8 });
  await page.mouse.up();
  expect(await size()).toEqual([400, 460]);

  // Shrink past the minimum: clamps to 220x260.
  grip = await win.locator('.grip').boundingBox();
  await page.mouse.move(grip.x + grip.width / 2, grip.y + grip.height / 2);
  await page.mouse.down();
  await page.mouse.move(grip.x + grip.width / 2 - 1000, grip.y + grip.height / 2 - 1000, {
    steps: 8,
  });
  await page.mouse.up();
  expect(await size()).toEqual([220, 260]);
});
