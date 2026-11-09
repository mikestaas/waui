// The site's desktop (site/desktop.js): the icon grid and the windows.
//
// Each window embeds a demo page in an iframe (site/desktop.js), so the
// demo's document is asserted through frameLocator. The tiles are located
// by position: the DEMOS order in site/desktop.js is fixed.

import { expect, test } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  await page.goto('/');
});

// The z-order counter (site/desktop.js's raise()).
const z = (win) => win.evaluate((el) => Number(el.dataset.z));

test('renders the six demo tiles', async ({ page }) => {
  const tiles = page.locator('#grid .tile');
  await expect(tiles).toHaveCount(6);
  await expect(tiles.nth(0).locator('.tile-name')).toHaveText('Counter');
  await expect(tiles.nth(0).locator('.tile-lang')).toHaveText('Rust');
  await expect(tiles.nth(1).locator('.tile-lang')).toHaveText('AssemblyScript');
  await expect(tiles.nth(2).locator('.tile-name')).toHaveText('Stopwatch');
  await expect(tiles.nth(4).locator('.tile-name')).toHaveText('Todo');
  await expect(tiles.nth(5).locator('.tile-lang')).toHaveText('AssemblyScript');
  await expect(page.locator('#window-count')).toHaveText('0 windows');
});

test('double-click opens a window', async ({ page }) => {
  await page.locator('#grid .tile').nth(0).dblclick();
  const win = page.locator('.window');
  await expect(win).toHaveCount(1);
  await expect(win.locator('.titlebar-title')).toHaveText('Counter — Rust');
  // The demo runs in the frame, exactly as standalone.
  await expect(win.frameLocator('iframe').locator('[data-id="3"]')).toHaveText('0');
  await expect(page.locator('#window-count')).toHaveText('1 window');
});

test('Enter opens a focused tile', async ({ page }) => {
  await page.locator('#grid .tile').nth(0).focus();
  await page.keyboard.press('Enter');
  await expect(page.locator('.window')).toHaveCount(1);
});

test('windows raise on click', async ({ page }) => {
  const tiles = page.locator('#grid .tile');
  await tiles.nth(0).dblclick(); // counter
  await tiles.nth(2).dblclick(); // stopwatch
  await expect(page.locator('#window-count')).toHaveText('2 windows');
  const counterWin = page.locator('.window').first();
  const stopWin = page.locator('.window').last();

  // The last opened is on top.
  expect(await z(stopWin)).toBeGreaterThan(await z(counterWin));

  // A click on the counter's title bar raises it.
  await counterWin.locator('.titlebar').click();
  expect(await z(counterWin)).toBeGreaterThan(await z(stopWin));

  // A click inside the stopwatch's frame raises that window (the frame's
  // document carries the raise listener). The point sits in the frame's
  // bottom strip, clear of the counter window, which now overlaps the
  // frame's top-left.
  await stopWin
    .frameLocator('iframe')
    .locator('body')
    .click({ position: { x: 40, y: 330 } });
  expect(await z(stopWin)).toBeGreaterThan(await z(counterWin));
});

test('close button and Escape close windows', async ({ page }) => {
  const tiles = page.locator('#grid .tile');
  await tiles.nth(0).dblclick();
  await tiles.nth(2).dblclick();
  await expect(page.locator('#window-count')).toHaveText('2 windows');

  // The frontmost (last opened) closes by its button.
  await page.locator('.window').last().locator('.close').click();
  await expect(page.locator('#window-count')).toHaveText('1 window');

  // Escape closes the frontmost. The desktop's wallpaper takes the key
  // focus (a click inside a window would send it to the frame's document).
  // #desktop itself is zero-height (its grid is absolute), so the click
  // lands on the body.
  await page.locator('body').click({ position: { x: 1150, y: 650 } });
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

test('the Rust todo window shows its heap note', async ({ page }) => {
  await page.locator('#grid .tile').nth(4).dblclick();
  await expect(page.locator('.window .note')).toHaveText(
    'note: the Rust web todo can run out of heap after a few adds',
  );
});
