// Builds a self-contained waui web page for one component.
//
//   node web/build.mjs <component.wasm> <out.html>
//
// Inlines the core wasm module (base64), glue.js, and app.js into a copy of
// index.html, producing a single file that runs from disk with no server,
// no npm, and no network access.

import { readFileSync, writeFileSync } from 'node:fs';
import { basename, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const [wasmPath, outPath] = process.argv.slice(2);
if (!wasmPath || !outPath) {
  console.error('usage: node web/build.mjs <component.wasm> <out.html>');
  process.exit(2);
}

const here = dirname(fileURLToPath(import.meta.url));
const template = readFileSync(`${here}/index.html`, 'utf8');
const glue = readFileSync(`${here}/glue.js`, 'utf8');
const app = readFileSync(`${here}/app.js`, 'utf8');

// The page's scripts are concatenated inside one <script type="module">,
// so glue.js's `export` becomes a plain function declaration there.
const glueInlined = glue.replace(/^export /m, '');

const wasm = readFileSync(wasmPath);
const b64 = wasm.toString('base64');

const name = basename(wasmPath)
  .replace(/\.wasm$/, '')
  .replace(/_/g, '-');

const page = template
  .replaceAll('__WAUI_NAME__', name)
  .replaceAll('__WAUI_WASM_B64__', b64)
  .replaceAll('__WAUI_JS__', glueInlined + '\n' + app);

writeFileSync(outPath, page);
console.log(
  `wrote ${outPath} (${(page.length / 1024).toFixed(0)} kB, component ${(wasm.length / 1024).toFixed(0)} kB)`,
);
