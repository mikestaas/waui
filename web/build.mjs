// Builds a waui web page for one component.
//
//   node web/build.mjs <component.wasm> <out.html>
//   node web/build.mjs <component.wasm> <out.html> --url <wasm-url>
//   node web/build.mjs <component.wasm> <out.html> --entry <name=path>...
//
// The default inlines the core wasm module (base64) plus glue.js and app.js
// into a copy of index.html, producing a single self-contained file that
// runs from disk with no server, no npm, and no network access. With --url,
// the page fetches the module from <wasm-url> as a blob instead, for sites
// that serve the .wasm file (make site).
//
// With --entry, the page also carries the windowing side of the `app` world:
// a registry of launchable entries. Each entry's module is inlined (a data:
// URL) or fetched (--url mode), and its `data` blob is derived from the name
// (the display label, the language's caption, the icon, the window's default
// size) — the host-defined metadata the opening component parses.

import { existsSync, readFileSync, writeFileSync } from 'node:fs';
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
// The entry registry, repeatable: `--entry <name=path>`.
const entries = [];
for (;;) {
  const entryFlag = args.indexOf('--entry');
  if (entryFlag < 0) break;
  const spec = args[entryFlag + 1];
  if (!spec || !spec.includes('=')) {
    console.error('--entry takes name=path');
    process.exit(2);
  }
  const [name, ...rest] = spec.split('=');
  entries.push({ name, path: rest.join('=') });
  args.splice(entryFlag, 2);
}
const [wasmPath, outPath] = args;
if (!wasmPath || !outPath) {
  console.error(
    'usage: node web/build.mjs <component.wasm> <out.html> [--url <wasm-url>] [--entry <name=path>]...',
  );
  process.exit(2);
}

const here = dirname(fileURLToPath(import.meta.url));
const template = readFileSync(`${here}/index.html`, 'utf8');
const glue = readFileSync(`${here}/glue.js`, 'utf8');
const app = readFileSync(`${here}/app.js`, 'utf8');

// The page's scripts are concatenated inside one <script type="module">, so
// glue.js's `export` becomes a plain function declaration there.
const glueInlined = glue.replace(/^export /m, '');

const name = basename(wasmPath)
  .replace(/\.wasm$/, '')
  .replace(/_/g, '-');

// The entry's `data` blob, derived from its name (the host-defined metadata
// the opening component parses): the display label (the name's base,
// capitalized), the language's caption (the name's suffix), the icon (the
// site's icon set, when it has one for the name), and the window's default
// size (per demo). A name without a recognized suffix (a local registry)
// yields the label and the size only.
function deriveData(n) {
  const m = n.match(/^(.*)-(rust|as|wat)$/);
  const base = m ? m[1] : n;
  const data = { label: base.charAt(0).toUpperCase() + base.slice(1) };
  if (m) data.lang = { rust: 'Rust', as: 'AssemblyScript', wat: 'WAT' }[m[2]];
  if (existsSync(new URL(`../site/icons/${n}.svg`, import.meta.url))) {
    data.icon = `icons/${n}.svg`;
  }
  const [width, height] = { counter: [300, 380], stopwatch: [300, 380], todo: [460, 500] }[
    base
  ] ?? [300, 380];
  data.width = width;
  data.height = height;
  return JSON.stringify(data);
}

// The registry, as the page carries it. `src` is host-internal (the
// component never sees it): inlined mode is a data: URL the page fetches
// like any other src; --url mode is the URL as given.
const registry = entries.map((e) => {
  const entry = { name: e.name, data: deriveData(e.name) };
  if (wasmUrl) {
    entry.src = e.path;
  } else {
    entry.src = `data:application/wasm;base64,${readFileSync(e.path).toString('base64')}`;
  }
  return entry;
});

// The boot script defines WAUI_WASM_BYTES for the app module and
// WAUI_REGISTRY for the windowing layer. Inlined mode is a classic script:
// it runs before the deferred module, and file:// pages cannot load module
// *files*. Fetch mode is a promise of the bytes — WebAssembly.instantiate
// accepts one, so glue.js and app.js are unchanged. A failed fetch (a file://
// page, a missing module) leaves a message in the page instead of a blank
// one.
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
  const WAUI_REGISTRY = ${JSON.stringify(registry)};
</script>`;
} else {
  const b64 = readFileSync(wasmPath).toString('base64');
  boot = `<script>
  // The component's core wasm module (the pre-lift artifact the wasmtime
  // hosts lift from), base64 so the page needs no fetch.
  const WAUI_WASM_B64 = '${b64}';
  const WAUI_WASM_BYTES = Uint8Array.from(atob(WAUI_WASM_B64), (c) => c.charCodeAt(0));
  const WAUI_REGISTRY = ${JSON.stringify(registry)};
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
console.log(
  `wrote ${outPath} (${(page.length / 1024).toFixed(0)} kB, ${size}${entries.length > 0 ? `, ${entries.length} entries` : ''})`,
);
