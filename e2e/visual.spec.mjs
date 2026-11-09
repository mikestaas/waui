// Screenshot goldens: the pixel layer.
//
// Font rendering is platform-specific, so these compare only in CI (the
// committed goldens were generated on the CI runner's image); locally the
// file skips. Regenerate after a runner image change with the workflow's
// update_snapshots input. A local `--update-snapshots` run writes this
// machine's rendering — do not commit it.

import { expect, test } from '@playwright/test';
import { loadDemo } from './helpers.mjs';

test.skip(!process.env.CI, 'screenshot goldens are CI-only (font rendering is platform-specific)');

for (const [name, path] of [
  ['counter', '/demo-counter.html'],
  ['counter-as', '/demo-counter-as.html'],
  ['stopwatch', '/demo-stopwatch.html'],
  ['stopwatch-as', '/demo-stopwatch-as.html'],
  ['todo', '/demo-todo.html'],
  ['todo-as', '/demo-todo-as.html'],
]) {
  // All six pages are static at rest: the counter and todo request no
  // refresh, and the stopwatch only runs while started.
  test(`${name} page`, async ({ page }) => {
    await loadDemo(page, path);
    await expect(page).toHaveScreenshot(`${name}.png`);
  });
}

test('desktop with two windows', async ({ page }) => {
  await page.goto('/');
  const tiles = page.locator('#grid .tile');
  await tiles.nth(0).dblclick(); // Counter (Rust)
  await tiles.nth(5).dblclick(); // Todo (AssemblyScript)
  // Both windows' demos are static at rest; wait for the top one to boot.
  await expect(
    page.locator('.window').last().frameLocator('iframe').locator('[data-id="2"]'),
  ).toHaveText('todos');
  await expect(page).toHaveScreenshot('desktop.png');
});
