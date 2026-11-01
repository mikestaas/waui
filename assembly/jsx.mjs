// jsx.mjs — transform a `.tsx` source into the `.gen.ts` that `asc` compiles.
//
// Why: asc's parser has no JSX support (no `--jsxFactory` either), so a
// source that uses JSX cannot be compiled directly. The standard
// `@babel/plugin-transform-react-jsx` (classic runtime, pragma `h`) rewrites
// each element to a call on the tree layer's factory —
// `<column id={1}>a b</column>` becomes `h("column", { id: 1 }, a, b)` —
// and the `typescript` parser plugin keeps the `i32`/`i64` annotations, so
// the output is plain AssemblyScript again.
//
// The object-literal props argument compiles because AssemblyScript coerces
// an object literal to a class type: absent attributes take the `Props`
// field defaults, and a typo'd attribute is a compile error.
//
// Usage: node assembly/jsx.mjs <in.tsx> <out.ts>

import { writeFileSync } from 'node:fs';
import { transformFileSync } from '@babel/core';

const [inFile, outFile] = process.argv.slice(2);
if (!inFile || !outFile) {
  console.error('usage: node assembly/jsx.mjs <in.tsx> <out.ts>');
  process.exit(2);
}

let code;
try {
  const result = transformFileSync(inFile, {
    sourceType: 'module',
    // The `.tsx` extension only pulls in the `jsx` parser plugin; the
    // `typescript` one must be asked for explicitly or the type
    // annotations fail to parse.
    parserOpts: { plugins: ['typescript', 'jsx'] },
    plugins: [
      // The default `throwIfNamespace` stays on: a `<a:b>` tag is a
      // transform error, not a string tag the factory would reject later.
      ['@babel/plugin-transform-react-jsx', { runtime: 'classic', pragma: 'h' }],
    ],
  });
  code = result.code;
} catch (e) {
  console.error(`jsx: ${inFile}: ${e.message}`);
  process.exit(1);
}

writeFileSync(outFile, code);
console.log(`jsx: ${inFile} -> ${outFile}`);
