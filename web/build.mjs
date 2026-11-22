// Builds a waui web page for one component.
//
//   node web/build.mjs <component.wasm> <out.html>
//   node web/build.mjs <component.wasm> <out.html> --url <wasm-url>
//   node web/build.mjs <component.wasm> <out.html> --entry <name=path>...
//   node web/build.mjs <component.wasm> <out.html> --registry <manifest.json>
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
//
// With --registry, the registry comes from the manifest file the native
// hosts read (registry.json): one source of truth for the desktop's entries.
// The manifest's `data` is carried as-is, except the icon, which the page
// remaps from the repo's `site/icons/` to its own `icons/`. Each entry's
// `src` is derived from the name's language suffix: --url mode is the
// module's URL, inlined mode the core module the page reads (its rust one
// from `target/<wasm-target>/release/`, the flag's default).

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
// The registry manifest, one: `--registry <manifest.json>`, the file the
// native hosts read (registry.json at the repo root) — one source of truth
// for the desktop's entries.
const registryFlag = args.indexOf('--registry');
let manifest = null;
if (registryFlag >= 0) {
  const file = args[registryFlag + 1];
  if (!file) {
    console.error('--registry takes a manifest path');
    process.exit(2);
  }
  manifest = JSON.parse(readFileSync(file, 'utf8'));
  args.splice(registryFlag, 2);
}
// The rust core module's directory for inlined mode (the Makefile's
// `WASM_TARGET`); the other producers' directories are fixed.
const targetFlag = args.indexOf('--wasm-target');
let wasmTarget = 'wasm32-unknown-unknown';
if (targetFlag >= 0) {
  wasmTarget = args[targetFlag + 1];
  if (!wasmTarget) {
    console.error('--wasm-target takes a value');
    process.exit(2);
  }
  args.splice(targetFlag, 2);
}
const [wasmPath, outPath] = args;
if (!wasmPath || !outPath) {
  console.error(
    'usage: node web/build.mjs <component.wasm> <out.html> [--url <wasm-url>] [--entry <name=path>]... [--registry <manifest.json>] [--wasm-target <triple>]',
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

// The manifest's `data`, with the icon remapped to the page's icon
// directory: the manifest carries the canonical (native) path, and the page
// serves the same set from its own `icons/`.
function manifestData(d) {
  const data = { ...d };
  if (typeof data.icon === 'string' && data.icon.startsWith('site/')) {
    data.icon = data.icon.slice('site/'.length);
  }
  return data;
}

// The manifest entry's module `src`, from the name's language suffix (no
// suffix is rust): --url mode is the module's URL; inlined mode is the core
// module's path, read for the data: URL. The manifest's own `path` is the
// native component's, which the page does not run.
function entrySrc(name) {
  const m = name.match(/^(.*)-(rust|as|wat)$/);
  const base = m ? m[1] : name;
  const suffix = m ? `-${m[2]}` : '';
  if (wasmUrl) return `demo-${base}${suffix}.wasm`;
  if (!m || m[2] === 'rust') return `target/${wasmTarget}/release/demo_${base}.wasm`;
  if (m[2] === 'as') return `target/assembly/demo-${base}-as.wasm`;
  return `target/wat/demo-${base}.wasm`;
}

// The registry, as the page carries it. `src` is host-internal (the
// component never sees it): inlined mode is a data: URL the page fetches
// like any other src; --url mode is the URL as given.
const registry = [
  // The manifest's entries: the `data` as carried (the icon remapped), the
  // `src` derived from the name.
  ...(manifest
    ? manifest.entries.map((e) => ({
        name: e.name,
        path: entrySrc(e.name),
        data: JSON.stringify(manifestData(e.data)),
      }))
    : []),
  // The ad-hoc `--entry` ones: the `data` derived from the name.
  ...entries.map((e) => ({ ...e, data: deriveData(e.name) })),
].map((e) => {
  const entry = { name: e.name, data: e.data };
  entry.src = wasmUrl
    ? e.path
    : `data:application/wasm;base64,${readFileSync(e.path).toString('base64')}`;
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
  `wrote ${outPath} (${(page.length / 1024).toFixed(0)} kB, ${size}${registry.length > 0 ? `, ${registry.length} entries` : ''})`,
);
