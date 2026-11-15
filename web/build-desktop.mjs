// Builds the waui desktop page (the `demo-desktop` component, the windowing
// side of the `app` world).
//
//   node web/build-desktop.mjs <demo-desktop.wasm> <out.html>
//   node web/build-desktop.mjs <demo-desktop.wasm> <out.html> --url <wasm-url>
//
// Like build.mjs, it inlines the core wasm module (base64) plus glue.js and
// desktop.js into a copy of desktop.html. The demo modules and the icons are
// fetched at runtime, so the page needs a server (make site). With --url, the
// `demo-desktop` module is fetched from <wasm-url> instead of inlined.

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
  console.error(
    'usage: node web/build-desktop.mjs <demo-desktop.wasm> <out.html> [--url <wasm-url>]',
  );
  process.exit(2);
}

const here = dirname(fileURLToPath(import.meta.url));
const template = readFileSync(`${here}/desktop.html`, 'utf8');
const glue = readFileSync(`${here}/glue.js`, 'utf8');
const desktop = readFileSync(`${here}/desktop.js`, 'utf8');

// The page's scripts are concatenated inside one <script type="module">, so
// glue.js's `export` becomes a plain function declaration there. desktop.js
// runs as a module (its top-level await needs one) and references the
// createApp/instantiateApp declarations from glue.js.
const glueInlined = glue.replace(/^export /m, '');

const name = basename(wasmPath)
  .replace(/\.wasm$/, '')
  .replace(/_/g, '-');

// The boot script defines WAUI_WASM_BYTES for the `demo-desktop` module.
// Inlined mode is a classic script: it runs before the deferred module, and
// file:// pages cannot load module *files*. Fetch mode is a promise of the
// bytes — WebAssembly.instantiate accepts one. A failed fetch (a file:// page,
// a missing module) leaves a message in the page instead of a blank one.
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
      document.getElementById('window-count').textContent = 'module load failed';
      document.getElementById('grid').textContent =
        'Could not load the desktop module (' + err.message + '). Serve the site over http(s).';
      throw err;
    });
</script>`;
} else {
  const b64 = readFileSync(wasmPath).toString('base64');
  boot = `<script>
  // The desktop component's core wasm module (the pre-lift artifact the
  // wasmtime hosts lift from), base64 so the page needs no fetch.
  const WAUI_WASM_B64 = '${b64}';
  const WAUI_WASM_BYTES = Uint8Array.from(atob(WAUI_WASM_B64), (c) => c.charCodeAt(0));
</script>`;
}

const page = template
  .replaceAll('__WAUI_NAME__', name)
  .replaceAll('__WAUI_WASM_BOOT__', boot)
  .replaceAll('__WAUI_JS__', glueInlined + '\n' + desktop);

writeFileSync(outPath, page);
const size = wasmUrl
  ? `module at ${wasmUrl}`
  : `component ${(readFileSync(wasmPath).length / 1024).toFixed(0)} kB`;
console.log(`wrote ${outPath} (${(page.length / 1024).toFixed(0)} kB, ${size})`);
