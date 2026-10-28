// Replace the incremental runtime's `env.abort` import with an in-module
// `unreachable` function, so the lifted component is import-free again.
//
// `ui/tree` needs the real AS runtime (the nested object tree lives on its
// heap), and asc's incremental runtime pulls in an `env.abort` import on its
// OOM path. The `waui:ui` world declares no imports from `env`, so the
// hand-rolled module must not have one either.
//
// The patch is a WAT round-trip — `wasm2wat`, a text rewrite, `wat2wasm` —
// because the rewrite is a *slot swap*: the abort import's function index is
// redefined in place. Imports and defined functions share one index space
// (imports first, in section order), so converting one import into a
// definition reindexes what follows it; the patch computes the full
// old→new index map and rewrites every function reference (`call`,
// `export`/`start`/`elem` targets, `ref.func`) accordingly. For the counter
// (abort is its only import) the map is the identity; for the stopwatch
// (abort + the world's `$root.now`) `now` shifts one slot and the map
// renumbers it. `WebAssembly.validate` plus the selftest then prove the
// result.
//
// `unreachable` is behaviorally identical to `abort` for this purpose —
// both trap, so the OOM path remains a trap.
// The output is checked (valid, no `env` imports, same export names) before
// it is written, and a module without the import (e.g. a `--runtime stub`
// build) is copied through untouched.
//
// Requires wabt (`wasm2wat`, `wat2wasm`) on the PATH — the same class of
// system wasm tool the pipeline already needs for `wasm-tools`.
//
// Usage: node assembly/patch-env-abort.mjs in.wasm out.wasm

import { readFileSync, writeFileSync, mkdtempSync, rmSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const [input, output] = process.argv.slice(2);
if (input.length < 2) {
  console.error('usage: node assembly/patch-env-abort.mjs in.wasm out.wasm');
  process.exit(2);
}
const inBytes = readFileSync(input);

// Modules without the import need no patch.
let inModule;
try {
  inModule = new WebAssembly.Module(inBytes);
} catch (e) {
  console.error(`patch-env-abort: input is not a valid module: ${e.message}`);
  process.exit(1);
}
const inImports = WebAssembly.Module.imports(inModule);
if (!inImports.some((i) => i.module === 'env' && i.name === 'abort')) {
  console.log('patch-env-abort: no env.abort import; copied through');
  writeFileSync(output, inBytes);
  process.exit(0);
}

function run(cmd, args) {
  const r = spawnSync(cmd, args, { stdio: ['ignore', 'pipe', 'pipe'] });
  if (r.error) {
    console.error(`patch-env-abort: '${cmd}' not found on PATH (install wabt)`);
    process.exit(1);
  }
  if (r.status !== 0) {
    console.error(`patch-env-abort: ${cmd} failed: ${r.stderr.toString().trim()}`);
    process.exit(1);
  }
}

const dir = mkdtempSync(join(tmpdir(), 'patch-env-abort-'));
const inWat = join(dir, 'module.wat');
const outWasm = join(dir, 'module.wasm');
try {
  run('wasm2wat', [input, '--output', inWat]);
  const wat = readFileSync(inWat, 'utf8');

  // The abort entry, in wasm2wat's shape (the func may or may not carry a
  // name): (import "env" "abort" (func $n (;K;) (type T)))
  const reEntry =
    /\(import "env" "abort" \(func ((?:\$[a-zA-Z_][a-zA-Z0-9_]* )?)\(;\s*(\d+)\s*;\) \(type (\d+)\)\)\)/g;
  const entries = [...wat.matchAll(reEntry)];
  if (entries.length !== 1) {
    console.error(
      `patch-env-abort: expected 1 env.abort import in the WAT, found ${entries.length}`,
    );
    process.exit(1);
  }
  const name = entries[0][1] ?? '';
  const abortNum = parseInt(entries[0][2], 10);
  const abortType = entries[0][3];

  // ------------------------------------------------ the index map
  // The flat function index space: imports first (in section order), then
  // defined functions (in section order). The binary is the source of truth
  // for the import side; the WAT (same order) provides the defined count.
  const importIdx = inImports.map((i, k) => ({ mod: i.module, field: i.name, kind: i.kind, k }));
  const funcImports = importIdx.filter((i) => i.kind === 'function');
  const N = funcImports.length;
  if (funcImports.length !== importIdx.length) {
    console.error('patch-env-abort: non-function imports present; not supported');
    process.exit(1);
  }
  const kAbort = funcImports.findIndex((i) => i.mod === 'env' && i.field === 'abort');
  if (kAbort !== abortNum) {
    console.error(
      `patch-env-abort: import order mismatch (binary says ${kAbort}, WAT says ${abortNum})`,
    );
    process.exit(1);
  }
  // Defined functions: top-level `(func` lines in the WAT (the import's
  // inner `(func ...)` sits inside an `(import ...)` line, so it does not
  // match a line-anchored pattern).
  const M = [...wat.matchAll(/^ {2}\(func /gm)].length;
  const total = N + M;

  // New space: abort becomes the *first* defined function; the other
  // imports keep their relative order (those after abort shift left one);
  // the other defined functions are untouched.
  const map = new Map();
  funcImports.forEach((i) => {
    if (i.k === kAbort)
      map.set(i.k, N - 1); // the slot the new definition takes
    else map.set(i.k, i.k < kAbort ? i.k : i.k - 1);
  });
  for (let j = 0; j < M; j++) map.set(N + j, N + j); // identity
  for (let i = 0; i < total; i++) {
    if (!map.has(i) || !Number.isInteger(map.get(i)) || map.get(i) < 0 || map.get(i) >= total) {
      console.error(`patch-env-abort: index map hole at ${i}`);
      process.exit(1);
    }
  }
  // (The map is a bijection by construction; this asserts totality.)
  if (new Set(map.values()).size !== total) {
    console.error('patch-env-abort: index map is not injective');
    process.exit(1);
  }

  // ------------------------------------------------------- the rewrite
  // 1. The slot swap: the abort import becomes a definition, renumbered to
  //    its new index in the comment. WAT requires all imports to precede
  //    all definitions, so the definition is re-inserted as the *first*
  //    definition (after the last remaining import, or at the top of the
  //    function block) — which is exactly the slot the index map assigns it.
  const lines = wat.split('\n');
  const reEntryLine =
    /^ {2}\(import "env" "abort" \(func ((?:\$[a-zA-Z_][a-zA-Z0-9_]* )?)\(;\s*(\d+)\s*;\) \(type (\d+)\)\)\)\s*$/;
  const entryIdx = lines.findIndex((l) => reEntryLine.test(l));
  if (entryIdx < 0) {
    console.error('patch-env-abort: the env.abort import line has an unexpected shape');
    process.exit(1);
  }
  const [, entryName, entryNum, entryType] = lines[entryIdx].match(reEntryLine);
  if (parseInt(entryNum, 10) !== abortNum || parseInt(entryType, 10) !== parseInt(abortType, 10)) {
    console.error('patch-env-abort: the WAT and the binary disagree about the abort entry');
    process.exit(1);
  }
  lines.splice(entryIdx, 1);
  const funcLine = `  (func${entryName ? ` ${entryName.trim()}` : ''} (;${map.get(kAbort)};) (type ${entryType}) unreachable)`;
  const isImportLine = (l) => /^\s*\(import /.test(l);
  const lastImport = lines
    .map((l, i) => (isImportLine(l) ? i : -1))
    .filter((i) => i >= 0)
    .pop();
  let insertAt;
  if (lastImport === undefined) {
    insertAt = lines.findIndex((l) => /^\s*\(func /.test(l)); // first definition
    if (insertAt < 0) {
      console.error('patch-env-abort: no function block found in the WAT');
      process.exit(1);
    }
  } else {
    insertAt = lastImport + 1;
  }
  lines.splice(insertAt, 0, funcLine);
  let out = lines.join('\n');
  if (out === wat) {
    console.error('patch-env-abort: the slot swap did not apply (WAT shape unexpected)');
    process.exit(1);
  }

  // 2. Renumber every function reference. `String.replace` with a callback
  //    applies to the original text, occurrence by occurrence, so a
  //    bijective map cannot cascade.
  const renum = (re) => {
    out = out.replace(re, (whole, num) => {
      const n = parseInt(num, 10);
      if (!map.has(n)) {
        throw new Error(`function index ${n} has no image in the patch (in '${whole}')`);
      }
      return whole.replace(num, String(map.get(n)));
    });
  };
  renum(/\bcall (\d+)/g); // code
  renum(/\bref\.func (\d+)/g); // funcref constants
  renum(/\(func (\d+)\)/g); // export / start / singleton elem items
  renum(/\(start (\d+)\)/g);

  const outWat = inWat.replace('.wat', '.patched.wat');
  writeFileSync(outWat, out);
  run('wat2wasm', [outWat, '--output', outWasm]);

  // ----------------------------------------------------------- checks
  const outBytes = readFileSync(outWasm);
  if (!WebAssembly.validate(outBytes)) {
    console.error('patch-env-abort: patched module failed WebAssembly.validate');
    process.exit(1);
  }
  const outModule = new WebAssembly.Module(outBytes);
  const outImports = WebAssembly.Module.imports(outModule);
  if (outImports.some((i) => i.module === 'env')) {
    console.error(
      `patch-env-abort: module still imports from env: ${outImports.filter((i) => i.module === 'env').map((i) => i.name)}`,
    );
    process.exit(1);
  }
  const inExports = WebAssembly.Module.exports(inModule)
    .map((e) => e.name)
    .sort();
  const outExports = WebAssembly.Module.exports(outModule)
    .map((e) => e.name)
    .sort();
  if (JSON.stringify(inExports) !== JSON.stringify(outExports)) {
    console.error(`patch-env-abort: export set changed: ${inExports} -> ${outExports}`);
    process.exit(1);
  }

  writeFileSync(output, outBytes);
  const kept = outImports.map((i) => `${i.module}.${i.name}`).join(', ') || 'none';
  console.log(
    `patch-env-abort: env.abort (func ${kAbort}) replaced with unreachable — import-free (remaining imports: ${kept})`,
  );
} finally {
  rmSync(dir, { recursive: true, force: true });
}
