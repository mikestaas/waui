// The component desktop (the site's index page, ?demo=desktop): the
// `demo-desktop` component on the windowing side of the `app` world, with
// the windowing host. Each window's content is a demo *component* running in
// its own sandbox, its tree seated in the window's content slot (the
// container whose id is the host handle).
//
// The grid is the component's tree: nine tiles (the cursor-pointer nodes,
// ids 100 and up), each an icon, a label, and a caption. The window count is
// the component's menu-bar node (id 4, under the menu bar, id 2).
//
// The window's chrome is the component's own tree, addressed by the handle
// scheme (crates/demo-desktop): window h+100000, title bar h+200000, close
// h+400000, grip h+500000, content slot h. Handles start at 10000 and grow
// monotonically per page, so a fresh page's first window is 10000.
//
// The suite runs against the component in all three source languages (the
// Rust page, the AssemblyScript one, the WAT one): the host and the demo
// windows are the same on all three, so the whole contract must hold on each.

import { expect, test } from '@playwright/test';

// The desktop page per producer (helpers' desktopPages).
const PAGES = [
  ['/index.html?demo=desktop&producer=rust', 'Rust'],
  ['/index.html?demo=desktop&producer=as', 'AssemblyScript'],
  ['/index.html?demo=desktop&producer=wat', 'WAT'],
];

// The window management and grid contract, for one producer's page.
function defineDesktopTests(url) {
  test.beforeEach(async ({ page }) => {
    await page.goto(url);
    // The boot is async (the demos are pre-compiled, then the component
    // runs); wait for the grid to populate.
    await expect(page.locator('#app .waui-cursor-pointer')).toHaveCount(9);
  });

  // The window nodes of handle `h` (the host assigns 10000 to the first
  // window a fresh page opens, then 10001, ...).
  const win = (page, h) => page.locator(`[data-id="${h + 100000}"]`);
  const z = (el) => el.evaluate((el) => Number(el.dataset.z));

  test('renders the nine demo tiles', async ({ page }) => {
    const tiles = page.locator('#app .waui-cursor-pointer');
    await expect(tiles).toHaveCount(9);
    // The tile's label and caption are its text children (the icon, when
    // the registry has one, is an img, not a div).
    await expect(tiles.nth(0).locator('div').nth(0)).toHaveText('Counter');
    await expect(tiles.nth(0).locator('div').nth(1)).toHaveText('Rust');
    await expect(tiles.nth(1).locator('div').nth(1)).toHaveText('AssemblyScript');
    await expect(tiles.nth(2).locator('div').nth(1)).toHaveText('WAT');
    await expect(tiles.nth(3).locator('div').nth(0)).toHaveText('Stopwatch');
    await expect(tiles.nth(5).locator('div').nth(1)).toHaveText('WAT');
    await expect(tiles.nth(6).locator('div').nth(0)).toHaveText('Todo');
    await expect(tiles.nth(8).locator('div').nth(1)).toHaveText('WAT');
    await expect(page.locator('[data-id="2"] [data-id="4"]')).toHaveText('0 windows');
  });

  // The menu bar's repo link: id-less, so the host's marker class is its
  // address. The host owns the opening (a new tab); the component is never
  // told (the WIT), so the link is out of the component's pointer model.
  test('the menu bar carries the repo link', async ({ page }) => {
    const link = page.locator('#app a.waui-link');
    await expect(link).toHaveCount(1);
    await expect(link).toHaveText('GitHub');
    await expect(link).toHaveAttribute('href', 'https://github.com/mikestaas/waui');
    await expect(link).toHaveAttribute('target', '_blank');
    await expect(link).toHaveAttribute('rel', 'noopener');
  });

  test('double-click opens a window with the embedded demo', async ({ page }) => {
    await page.locator('#app .waui-cursor-pointer').nth(0).dblclick(); // Counter, Rust
    const w = win(page, 10000);
    await expect(w).toHaveCount(1);
    // The title bar's label (the entry's label, an id-less text node).
    await expect(w.locator('[data-id="210000"] div').first()).toHaveText('Counter');
    // The demo runs in place: the counter's readout (id 3) reads "0".
    await expect(w.locator('[data-id="10000"] [data-id="3"]')).toHaveText('0');
    await expect(page.locator('[data-id="2"] [data-id="4"]')).toHaveText('1 window');
  });

  test('the embedded demo is interactive', async ({ page }) => {
    await page.locator('#app .waui-cursor-pointer').nth(0).dblclick(); // Counter, Rust
    const w = win(page, 10000);
    const display = w.locator('[data-id="10000"] [data-id="3"]');
    await expect(display).toHaveText('0');
    // The "+" button (id 2) in the content slot (id 10000).
    const plus = w.locator('[data-id="10000"] button[data-id="2"]');
    await plus.click();
    await expect(display).toHaveText('1');
    await plus.click();
    await expect(display).toHaveText('2');
  });

  test('typing in the embedded todo keeps the input focused', async ({ page }) => {
    await page.locator('#app .waui-cursor-pointer').nth(6).dblclick(); // Todo, Rust
    const w = win(page, 10000);
    const input = w.locator('[data-id="10000"] input[data-id="8"]');
    await input.click();
    await input.type('buy coffee');
    // A content-only change restyles in place: the element is not replaced, so
    // focus and caret survive every keystroke.
    await expect(input).toBeFocused();
    await expect(input).toHaveValue('buy coffee');
  });

  test('windows raise on click', async ({ page }) => {
    const tiles = page.locator('#app .waui-cursor-pointer');
    await tiles.nth(0).dblclick(); // counter
    await tiles.nth(3).dblclick(); // stopwatch
    await expect(page.locator('[data-id="2"] [data-id="4"]')).toHaveText('2 windows');
    const counter = win(page, 10000);
    const stop = win(page, 10001);
    expect(await z(stop)).toBeGreaterThan(await z(counter));
    // A click on the counter's title bar raises it above the stopwatch.
    await counter.locator('[data-id="210000"]').click();
    expect(await z(counter)).toBeGreaterThan(await z(stop));
  });

  test('close marker and Escape close windows', async ({ page }) => {
    const tiles = page.locator('#app .waui-cursor-pointer');
    await tiles.nth(0).dblclick();
    await tiles.nth(3).dblclick();
    await expect(page.locator('[data-id="2"] [data-id="4"]')).toHaveText('2 windows');

    // The frontmost (last opened) closes by its marker.
    await win(page, 10001).locator('[data-id="410001"]').click();
    await expect(page.locator('[data-id="2"] [data-id="4"]')).toHaveText('1 window');

    // Escape closes the frontmost. Focus the wallpaper first (a key pressed
    // while a window's content has focus would go to the demo).
    await page.locator('body').click({ position: { x: 1200, y: 650 } });
    await page.keyboard.press('Escape');
    await expect(page.locator('[data-id="2"] [data-id="4"]')).toHaveText('0 windows');
  });

  test('drag moves the window', async ({ page }) => {
    await page.locator('#app .waui-cursor-pointer').nth(0).dblclick();
    const w = win(page, 10000);
    const before = await w.boundingBox();
    const bar = await w.locator('[data-id="210000"]').boundingBox();
    await page.mouse.move(bar.x + bar.width / 2, bar.y + bar.height / 2);
    await page.mouse.down();
    await page.mouse.move(bar.x + bar.width / 2 + 80, bar.y + bar.height / 2 + 50, {
      steps: 8,
    });
    await page.mouse.up();
    const after = await w.boundingBox();
    expect(Math.abs(after.x - before.x - 80)).toBeLessThan(1);
    expect(Math.abs(after.y - before.y - 50)).toBeLessThan(1);
  });

  test('the grip resizes, clamped to a minimum', async ({ page }) => {
    await page.locator('#app .waui-cursor-pointer').nth(0).dblclick(); // 300x380
    const w = win(page, 10000);
    const size = () => w.evaluate((el) => [el.offsetWidth, el.offsetHeight]);

    // Grow.
    let grip = await w.locator('[data-id="510000"]').boundingBox();
    await page.mouse.move(grip.x + grip.width / 2, grip.y + grip.height / 2);
    await page.mouse.down();
    await page.mouse.move(grip.x + grip.width / 2 + 100, grip.y + grip.height / 2 + 80, {
      steps: 8,
    });
    await page.mouse.up();
    expect(await size()).toEqual([400, 460]);

    // Shrink past the minimum: clamps to 220x260.
    grip = await w.locator('[data-id="510000"]').boundingBox();
    await page.mouse.move(grip.x + grip.width / 2, grip.y + grip.height / 2);
    await page.mouse.down();
    await page.mouse.move(grip.x + grip.width / 2 - 1000, grip.y + grip.height / 2 - 1000, {
      steps: 8,
    });
    await page.mouse.up();
    expect(await size()).toEqual([220, 260]);
  });
}

for (const [url, producer] of PAGES) {
  test.describe(`component desktop (${producer})`, () => defineDesktopTests(url));
}
