// Builds a waui web page for one component.
//
//   node web/build.mjs <component.wasm> <out.html>
//   node web/build.mjs <component.wasm> <out.html> --url <wasm-url>
//
// The default inlines the core wasm module (base64) plus glue.js and app.js
// into a copy of index.html, producing a single self-contained file that
// runs from disk with no server, no npm, and no network access. With --url,
// the page fetches the module from <wasm-url> as a blob instead, for sites
// that serve the .wasm file (make site).

import { readFileSync, writeFileSync } from 'node:fs';
import { basename, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const args = process.argv.slice(2);
const urlFlag = args.indexOf('--url');
let wasmUrl = null;
if (urlFlag >= 0) {
  wasmUrl = args[urlFlag + 1];
  if (!wasmUrl) {
    console.error('--url takes a value: the page will fetch the module from it');
    process.exit(2);
  }
  args.splice(urlFlag, 2);
}
const [wasmPath, outPath] = args;
if (!wasmPath || !outPath) {
  console.error('usage: node web/build.mjs <component.wasm> <out.html> [--url <wasm-url>]');
  process.exit(2);
}

const here = dirname(fileURLToPath(import.meta.url));
const template = readFileSync(`${here}/index.html`, 'utf8');
const glue = readFileSync(`${here}/glue.js`, 'utf8');
const app = readFileSync(`${here}/app.js`, 'utf8');

// The page's scripts are concatenated inside one <script type="module">,
// so glue.js's `export` becomes a plain function declaration there.
const glueInlined = glue.replace(/^export /m, '');

const name = basename(wasmPath)
  .replace(/\.wasm$/, '')
  .replace(/_/g, '-');

// The boot script defines WAUI_WASM_BYTES for the app module. Inlined mode
// is a classic script: it runs before the deferred module, and file:// pages
// cannot load module *files*. Fetch mode is a promise of the bytes —
// WebAssembly.instantiate accepts one, so glue.js and app.js are unchanged.
// A failed fetch (a file:// page, a missing module) leaves a message in the
// page instead of a blank one.
let boot;
if (wasmUrl) {
  boot = `<script>
  const WAUI_WASM_BYTES = fetch(${JSON.stringify(wasmUrl)})
    .then((r) => {
      if (!r.ok) throw new Error('HTTP ' + r.status);
      return r.arrayBuffer();
    })
    .then((b) => new Uint8Array(b))
    .catch((err) => {
      document.getElementById('badge').textContent = 'waui (web) — module load failed';
      document.getElementById('app').textContent =
        'Could not load the wasm module (' + err.message + '). Serve the site over http(s).';
      throw err;
    });
</script>`;
} else {
  const b64 = readFileSync(wasmPath).toString('base64');
  boot = `<script>
  // The component's core wasm module (the pre-lift artifact the wasmtime
  // hosts lift from), base64 so the page needs no fetch.
  const WAUI_WASM_B64 = '${b64}';
  const WAUI_WASM_BYTES = Uint8Array.from(atob(WAUI_WASM_B64), (c) => c.charCodeAt(0));
</script>`;
}

const page = template
  .replaceAll('__WAUI_NAME__', name)
  .replaceAll('__WAUI_WASM_BOOT__', boot)
  .replaceAll('__WAUI_JS__', glueInlined + '\n' + app);

writeFileSync(outPath, page);
const size = wasmUrl
  ? `module at ${wasmUrl}`
  : `component ${(readFileSync(wasmPath).length / 1024).toFixed(0)} kB`;
console.log(`wrote ${outPath} (${(page.length / 1024).toFixed(0)} kB, ${size})`);
