// Builds a waui web page for one component.
//
//   node web/build.mjs <component.wasm> <out.html>
//   node web/build.mjs <component.wasm> <out.html> --url <wasm-url>
//   node web/build.mjs <component.wasm> <out.html> --url-params
//   node web/build.mjs <component.wasm> <out.html> --entry <name=path>...
//   node web/build.mjs <component.wasm> <out.html> --registry <manifest.json>
//   node web/build.mjs <component.wasm> <out.html> --name <name>
//
// The default inlines the core wasm module (base64) plus glue.js and app.js
// into a copy of index.html, producing a single self-contained file that
// runs from disk with no server, no npm, and no network access. With --url,
// the page fetches the module from <wasm-url> as a blob instead, for sites
// that serve the .wasm file. With --url-params, the page is the site's
// index: ?demo= and ?producer= select the module at load (the iced and tui
// host pages take the same pair; the default demo is the desktop), so one
// page serves the nine demos and the desktop.
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
// remaps from the demo's source tree to its own flat `icons/`. Each entry's
// `src` is derived from the name's language suffix: --url mode is the
// module's URL, inlined mode the core module the page reads (its rust one
// from `target/<wasm-target>/release/`, the flag's default). --url-params
// mode applies the registry only when ?demo= is the desktop: a plain demo
// runs standalone, with no windowing and no entry preload.

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
// The site's index: the module URL from ?demo=/?producer= (a boolean).
const urlParams = args.includes('--url-params');
if (urlParams) {
  if (wasmUrl) {
    console.error('--url-params and --url are mutually exclusive');
    process.exit(2);
  }
  args.splice(args.indexOf('--url-params'), 1);
}
// The page's name (the title): the input module's by default.
const nameFlag = args.indexOf('--name');
let nameOverride = null;
if (nameFlag >= 0) {
  nameOverride = args[nameFlag + 1];
  if (!nameOverride) {
    console.error('--name takes a value');
    process.exit(2);
  }
  args.splice(nameFlag, 2);
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
    'usage: node web/build.mjs <component.wasm> <out.html> [--url <wasm-url> | --url-params] [--entry <name=path>]... [--registry <manifest.json>] [--wasm-target <triple>] [--name <name>]',
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

const name =
  nameOverride ??
  basename(wasmPath)
    .replace(/\.wasm$/, '')
    .replace(/_/g, '-');

// The entry's `data` blob, derived from its name (the host-defined metadata
// the opening component parses): the display label (the name's base,
// capitalized), the language's caption (the name's suffix), the icon (the
// demo's own source tree, when it has one for the name), and the window's
// default size (per demo). A name without a recognized suffix (a local
// registry) yields the label and the size only.
function deriveData(n) {
  const m = n.match(/^(.*)-(rust|as|wat)$/);
  const base = m ? m[1] : n;
  const data = { label: base.charAt(0).toUpperCase() + base.slice(1) };
  if (m) data.lang = { rust: 'Rust', as: 'AssemblyScript', wat: 'WAT' }[m[2]];
  // The icon, in the demo's source tree, per producer (the WAT demos are
  // flat files, so their icons are flat too).
  const iconPath = m
    ? {
        rust: `../crates/demo-${base}/${n}.svg`,
        as: `../assembly/demo-${base}/${n}.svg`,
        wat: `../wat/${n}.svg`,
      }[m[2]]
    : null;
  if (iconPath && existsSync(new URL(iconPath, import.meta.url))) {
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
// directory: the manifest carries the canonical path (the demo's source
// tree), and the page serves the same set from its own flat `icons/`. A
// page-relative icon (one without a directory) is carried as-is.
function manifestData(d) {
  const data = { ...d };
  if (typeof data.icon === 'string' && data.icon.includes('/')) {
    data.icon = `icons/${data.icon.split('/').pop()}`;
  }
  return data;
}

// The manifest entry's module `src`, from the name's language suffix (no
// suffix is rust): --url and --url-params modes are the module's URL;
// inlined mode is the core module's path, read for the data: URL. The
// manifest's own `path` is the native component's, which the page does not
// run.
function entrySrc(name) {
  const m = name.match(/^(.*)-(rust|as|wat)$/);
  const base = m ? m[1] : name;
  // rust carries no suffix (the site's file naming, as the host pages'
  // artifacts): demo-counter.wasm, demo-counter-as.wasm, demo-counter-wat.wasm.
  const suffix = m && m[2] !== 'rust' ? `-${m[2]}` : '';
  if (wasmUrl || urlParams) return `demo-${base}${suffix}.wasm`;
  if (!m || m[2] === 'rust') return `target/${wasmTarget}/release/demo_${base}.wasm`;
  if (m[2] === 'as') return `target/assembly/demo-${base}-as.wasm`;
  return `target/wat/demo-${base}.wasm`;
}

// The registry, as the page carries it. `src` is host-internal (the
// component never sees it): inlined mode is a data: URL the page fetches
// like any other src; --url and --url-params modes are the URL as given.
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
  entry.src =
    wasmUrl || urlParams
      ? e.path
      : `data:application/wasm;base64,${readFileSync(e.path).toString('base64')}`;
  return entry;
});

// The boot script defines WAUI_WASM_BYTES for the app module and
// WAUI_REGISTRY for the windowing layer. Inlined mode is a classic script:
// it runs before the deferred module, and file:// pages cannot load module
// *files*. The fetch modes (--url, --url-params) are a promise of the bytes
// — WebAssembly.instantiate accepts one, so glue.js and app.js are
// unchanged. A failed fetch (a file:// page, a missing module) leaves a
// message in the page instead of a blank one.
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
} else if (urlParams) {
  boot = `<script>
  // The site's index: ?demo= and ?producer= select the module (the iced and
  // tui host pages take the same pair; the default demo is the desktop).
  // The rust producer's module has no suffix (demo-counter.wasm); the others
  // do (demo-counter-as.wasm, demo-counter-wat.wasm).
  const params = new URLSearchParams(location.search);
  const demo = params.get('demo') ?? 'desktop';
  const producer = params.get('producer') ?? 'rust';
  const suffix = producer === 'rust' ? '' : \`-\${producer}\`;
  const moduleUrl = \`demo-\${demo}\${suffix}.wasm\`;
  const WAUI_WASM_BYTES = fetch(moduleUrl)
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
  // The entry registry: the desktop's windows seat the nine demos, so the
  // page applies it only for the desktop; a plain demo runs standalone
  // (no windowing, no entry preload).
  const WAUI_REGISTRY = demo === 'desktop' ? ${JSON.stringify(registry)} : [];
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
  : urlParams
    ? 'module from ?demo=/?producer='
    : `component ${(readFileSync(wasmPath).length / 1024).toFixed(0)} kB`;
console.log(
  `wrote ${outPath} (${(page.length / 1024).toFixed(0)} kB, ${size}${registry.length > 0 ? `, ${registry.length} entries` : ''})`,
);
