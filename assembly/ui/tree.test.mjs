// Node harness for `assembly/ui/tree.test.ts`: instantiates the core module
// (import-free after the `env.abort` patch) and runs its structural-diff
// scenarios. `run()` returns 0 on success, or the first failing scenario's
// index.
//
//   node assembly/ui/tree.test.mjs [tree-test.core.wasm]

import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const rel = (p) => (p.startsWith('/') ? p : fileURLToPath(new URL(`../../${p}`, import.meta.url)));
const path = process.argv[2] ?? 'target/assembly/tree-test.core.wasm';

const bytes = readFileSync(rel(path));
// Given a compiled module, `instantiate` resolves to the instance directly.
const instance = await WebAssembly.instantiate(await WebAssembly.compile(bytes));

let code;
try {
  code = instance.exports.run();
} catch (e) {
  console.error(`tree.test: trapped: ${e}`);
  process.exit(1);
}

if (code !== 0) {
  const names = [
    'bootstrap',
    'unchanged',
    'value-change',
    'removal',
    'addition',
    'reparent',
    'idless',
  ];
  console.error(`tree.test: scenario ${code} (${names[code - 1]}) failed`);
  process.exit(1);
}

console.log(
  'tree.test: all scenarios passed (bootstrap, unchanged, value, add, remove, reparent, idless)',
);
