// The Playwright config for the browser suite (e2e/).
//
// The site is served from target/site/ (make site) by a plain python http
// server: the demo pages fetch their .wasm modules, so the suite needs a
// server, exactly like the deployed site (GitHub Pages, or
// `python3 -m http.server` on the built directory).

import { defineConfig } from '@playwright/test';
import { existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('.', import.meta.url));
const port = 8321;

if (!existsSync(`${root}target/site/index.html`)) {
  throw new Error('target/site/index.html is missing — run `make site` first (or `make e2e`)');
}

export default defineConfig({
  testDir: './e2e',
  timeout: 30_000,
  expect: {
    timeout: 5_000,
    toHaveScreenshot: {
      pathTemplate: '{testDir}/screenshots/{testFileName}/{arg}-{projectName}{ext}',
    },
  },
  reporter: process.env.CI ? 'github' : 'list',
  // Four browsers at most: the suite is small, and each chromium is a
  // hundred-plus MB.
  workers: 4,
  use: {
    baseURL: `http://127.0.0.1:${port}`,
    viewport: { width: 1280, height: 720 },
    trace: 'retain-on-failure',
  },
  projects: [{ name: 'chromium' }],
  webServer: {
    command: `python3 -m http.server ${port} --directory ${root}target/site`,
    url: `http://127.0.0.1:${port}`,
    timeout: 30_000,
    reuseExistingServer: !process.env.CI,
  },
});
