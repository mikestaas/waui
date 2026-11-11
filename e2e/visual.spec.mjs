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
// Each demo page walks its states (helpers.mjs's demoStates): the counter
// at 0, 1, and -3, the stopwatch paused and running, and the todo seeded
// and after an add. One golden per page and state, per producer. The
// producer parity — the three pages must render each state identically —
// is e2e/parity.spec.mjs.

import { expect, test } from '@playwright/test';
import { demoPages, demoStates, settle } from './helpers.mjs';

test.skip(!process.env.CI, 'screenshot goldens are CI-only (font rendering is platform-specific)');

// The iced host's window size (crates/host-iced's WINDOW_WIDTH/HEIGHT).
const DEMO_VIEWPORT = { width: 360, height: 340 };

test.describe('demo pages', () => {
  test.use({ viewport: DEMO_VIEWPORT });

  for (const [demo, paths] of Object.entries(demoPages)) {
    const suffixes = ['', '-as', '-wat'];
    paths.forEach((path, i) => {
      const name = i === 0 ? demo : `${demo}${suffixes[i]}`;
      test(`${name} page`, async ({ page }) => {
        for (const [state, drive] of Object.entries(demoStates[demo])) {
          await drive(page, path);
          await settle(page);
          await expect(page).toHaveScreenshot(`${name}-${state}.png`);
        }
      });
    });
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
