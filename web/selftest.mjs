// Headless verification of the web host's ABI layer (Node; the same code
// the browser page uses). Exercises the counter's round-trip and diff
// channel, the stopwatch's import side (now + tick), the todo's text
// input (on-text) and structural diff (add, remove, re-parent), and the
// desktop's windowing imports (an empty registry) and its link node,
// against the raw wasm modules.
//
//   node web/selftest.mjs [counter.wasm] [stopwatch.wasm] [todo.wasm]
//                          [desktop.wasm]
//
// The paths default to the Rust components; passing other core modules
// (e.g. the AssemblyScript builds, `make web-test-as`) runs the same checks
// against them — same glue.js, same assertions, a different producer.
//
// The expected views come from ./expect.mjs, shared with the browser suite
// (e2e/): the same strings, the rendered DOM as a second producer of them.

import { readFileSync } from 'node:fs';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { createApp } from './glue.js';
import { counter, stopwatch, todo, desktop } from './expect.mjs';

// Resolve paths relative to the repository root (this file lives in web/);
// absolute paths (a non-default target) pass through.
const rel = (p) => (p.startsWith('/') ? p : fileURLToPath(new URL(`../${p}`, import.meta.url)));
const [counterPath, stopwatchPath, todoPath, desktopPath] = [
  process.argv[2] ?? 'target/wasm32-unknown-unknown/release/demo_counter.wasm',
  process.argv[3] ?? 'target/wasm32-unknown-unknown/release/demo_stopwatch.wasm',
  process.argv[4] ?? 'target/wasm32-unknown-unknown/release/demo_todo.wasm',
  process.argv[5] ?? 'target/wasm32-unknown-unknown/release/demo_desktop.wasm',
];

// A container into the selftest's node string: `dir` and `align` always
// (the contract's core layout), the other tokens only when non-default —
// the format the e2e DOM serializer (e2e/helpers.mjs) produces, so the two
// suites assert on one shared set of expectations (web/expect.mjs). The
// demos set only `dir` and `align`, so their strings are unchanged by the
// contract's geometry and chrome tokens.
function node(n) {
  if (n.kind === 'container') {
    const parts = [`id:${n.id}`, `parent:${n.parent}`, `dir:${n.direction}`, `align:${n.align}`];
    if (n.positioned) parts.push('pos:1');
    if (n.x > 0) parts.push(`x:${n.x}`);
    if (n.y > 0) parts.push(`y:${n.y}`);
    if (n.w > 0) parts.push(`w:${n.w}`);
    if (n.h > 0) parts.push(`h:${n.h}`);
    if (n.z > 0) parts.push(`z:${n.z}`);
    if (n.gap > 0) parts.push(`gap:${n.gap}`);
    if (n.padding > 0) parts.push(`pad:${n.padding}`);
    if (n.stretch !== 'none') parts.push(`stretch:${n.stretch}`);
    if (n.overflow !== 'clip') parts.push(`overflow:${n.overflow}`);
    if (n.background !== 'none') parts.push(`bg:${n.background}`);
    if (n.border !== 'none') parts.push(`border:${n.border}`);
    if (n.radius !== 'none') parts.push(`radius:${n.radius}`);
    if (n.shadow !== 'none') parts.push(`shadow:${n.shadow}`);
    if (n.cursor !== 'default') parts.push(`cursor:${n.cursor}`);
    return `container{${parts.join(',')}}`;
  }
  // An image has no style: the `src` and the size (a host default when 0).
  if (n.kind === 'image')
    return `image{id:${n.id},parent:${n.parent},src:"${n.src}",width:${n.width},height:${n.height}}`;
  const style = `style:${n.style.color}:${n.style.size}`;
  if (n.kind === 'button')
    return `button{id:${n.id},parent:${n.parent},label:"${n.label}",${style}}`;
  if (n.kind === 'input')
    return `input{id:${n.id},parent:${n.parent},content:"${n.content}",${style},width:${n.width}}`;
  // A link's target rides with it (the DOM serializer reads the `href`
  // attribute back, e2e/helpers.mjs).
  if (n.kind === 'link')
    return `link{id:${n.id},parent:${n.parent},label:"${n.label}",href:"${n.href}",${style}}`;
  return `text{id:${n.id},parent:${n.parent},content:"${n.content}",${style}}`;
}

function mutation(m) {
  return m.kind === 'removed' ? `removed{id:${m.id}}` : `changed ${node(m.node)} index:${m.index}`;
}

// ---------------------------------------------------------------- counter

{
  const app = await createApp(readFileSync(rel(counterPath)));
  const expect = (want) => {
    assert.deepEqual(
      app.view().map(node),
      want,
      `counter view mismatch (memory pages: ${app.memoryPages()})`,
    );
  };

  expect(counter.zero);

  // The diff channel: a click dirties exactly the readout — never the
  // whole tree.
  app.click(2);
  assert.deepEqual(
    app.changes().map(mutation),
    ['changed text{id:3,parent:1,content:"1",style:neutral:large} index:1'],
    'one click should report exactly the changed readout',
  );

  // Re-querying with nothing new reports an empty diff.
  assert.deepEqual(app.changes(), []);

  // An unknown widget id is the component's own no-op branch: no diff.
  app.click(99);
  assert.deepEqual(app.changes(), []);

  // The pointer and key channels (the windowing side of the contract): a
  // plain demo has no pointer-driven or key-driven UI, so the events are the
  // component's no-op branches: no diff.
  app.pointerDown(2, 10, 10);
  app.pointerMove(12, 12);
  app.pointerUp(13, 13);
  app.key('Escape');
  app.key('Tab');
  assert.deepEqual(app.changes(), [], 'pointer and key events must be no-ops for a plain demo');

  // Two decrements: the number's content *and* color change (one
  // mutation), and view() is still the full truth.
  app.click(4);
  app.click(4);
  assert.deepEqual(app.changes().map(mutation), [
    'changed text{id:3,parent:1,content:"-1",style:negative:large} index:1',
  ]);
  expect(counter.minusOne);

  // The WIT bootstrap rule.
  const app2 = await createApp(readFileSync(rel(counterPath)));
  assert.equal(app2.changes().length, 4, 'first-ever changes() should export the whole tree');

  // No unbounded growth: 60 more view() and 60 changes() calls must not
  // grow memory (both return guest-allocated buffers the host posts back).
  const before = app.memoryPages();
  for (let i = 0; i < 60; i++) {
    app.view();
    app.changes();
  }
  assert.ok(
    app.memoryPages() - before < 4,
    `view()/changes() leaked memory: ${before} -> ${app.memoryPages()} pages`,
  );
  console.log('counter: ok (round-trip + diff + leak check)');
}

// -------------------------------------------------------------- stopwatch

{
  const app = await createApp(readFileSync(rel(stopwatchPath)));
  const view = () => app.view().map(node);

  assert.deepEqual(view(), stopwatch.idle);

  // Start: both the readout (its color) and the button (label + color)
  // change — two mutations.
  app.click(3);
  let muts = app.changes().map(mutation);
  assert.equal(muts.length, 2);
  assert.match(muts[0], /^changed text\{.*content:"\d+ ms",style:accent:large\} index:0$/);
  assert.equal(muts[1], 'changed button{id:3,parent:1,label:"pause",style:warning:medium} index:1');

  // Let real time pass, tick: while running, only the readout changes.
  await new Promise((r) => setTimeout(r, 400));
  app.tick();
  muts = app.changes().map(mutation);
  assert.equal(muts.length, 1);
  const ms = Number(muts[0].match(/content:"(-?\d+) ms"/)[1]);
  assert.ok(ms >= 200 && ms < 2000, `unexpected elapsed time: ${ms} ms`);
  assert.match(muts[0], /style:accent:large/);
  assert.match(muts[0], /index:0$/);

  // Pause: the readout and the button change again...
  app.click(3);
  muts = app.changes().map(mutation);
  assert.equal(muts.length, 2);
  assert.match(muts[0], /^changed text\{.*content:"\d+ ms",style:neutral:large\} index:0$/);
  assert.equal(
    muts[1],
    'changed button{id:3,parent:1,label:"start",style:positive:medium} index:1',
  );

  // ...and a further tick while paused reports an *empty* diff: the
  // paused stopwatch generates zero diff traffic.
  await new Promise((r) => setTimeout(r, 300));
  app.tick();
  assert.deepEqual(app.changes(), [], 'paused stopwatch must report no changes');

  // view() is still the full truth, and the reading is frozen.
  const [, text] = view();
  assert.match(text, /style:neutral:large/);
  const frozenMs = Number(text.match(/content:"(-?\d+) ms"/)[1]);
  await new Promise((r) => setTimeout(r, 300));
  app.tick();
  assert.deepEqual(app.changes(), []);
  assert.equal(
    Number(view()[1].match(/content:"(-?\d+) ms"/)[1]),
    frozenMs,
    'paused stopwatch kept advancing',
  );

  console.log(`stopwatch: ok (ran ${ms} ms of real time, ${frozenMs} ms frozen)`);
}

// -------------------------------------------------------------------- todo

{
  const app = await createApp(readFileSync(rel(todoPath)));
  const view = () => app.view().map(node);

  // Seeded: two active (newest first), one done, "2 left".
  assert.deepEqual(view(), todo.seeded);

  // Type one char at a time: the host reports the whole buffer on each
  // keystroke, so each is a fresh buffer the guest owns, and the previous
  // one is freed as the new is stored. The field's content re-derives — one
  // mutation for the whole edit.
  let buf = '';
  for (const ch of 'buy coffee') {
    buf += ch;
    app.text(8, buf);
  }
  assert.deepEqual(app.changes().map(mutation), [
    'changed input{id:8,parent:6,content:"buy coffee",style:neutral:medium,width:12} index:1',
  ]);

  // Add with the typed text: the new row and its three children, the count,
  // and the cleared field. The row lands at the top of the active column.
  app.click(9);
  assert.deepEqual(app.changes().map(mutation), [
    'changed container{id:112,parent:4,dir:row,align:center} index:0',
    'changed button{id:113,parent:112,label:"○",style:neutral:medium} index:0',
    'changed text{id:114,parent:112,content:"buy coffee",style:neutral:medium} index:1',
    'changed button{id:115,parent:112,label:"✕",style:negative:medium} index:2',
    'changed text{id:7,parent:6,content:"3 left",style:neutral:small} index:0',
    'changed input{id:8,parent:6,content:"",style:neutral:medium,width:12} index:1',
  ]);

  // Toggle: the row is re-added under a new id (504), its children are
  // re-parented (same id, new parent), the count changes, and the old row is
  // removed. Six mutations.
  app.click(105);
  assert.deepEqual(app.changes().map(mutation), [
    'changed container{id:504,parent:5,dir:row,align:center} index:0',
    'changed button{id:105,parent:504,label:"✓",style:positive:medium} index:0',
    'changed text{id:106,parent:504,content:"walk the dog",style:warning:medium} index:1',
    'changed button{id:107,parent:504,label:"✕",style:negative:medium} index:2',
    'changed text{id:7,parent:6,content:"2 left",style:neutral:small} index:0',
    'removed{id:104}',
  ]);

  // view() is the full truth, and it is what the diff application yields:
  // the moved row at the top of the done column, newest first.
  assert.deepEqual(view(), todo.afterToggle);

  // Delete: the count, then the row and its children. Five mutations.
  app.click(103);
  assert.deepEqual(app.changes().map(mutation), [
    'changed text{id:7,parent:6,content:"1 left",style:neutral:small} index:0',
    'removed{id:100}',
    'removed{id:101}',
    'removed{id:102}',
    'removed{id:103}',
  ]);

  // Clear done: only the removed rows — the count (the active list) is
  // unchanged, so it is not reported.
  app.click(10);
  assert.deepEqual(app.changes().map(mutation), [
    'removed{id:504}',
    'removed{id:105}',
    'removed{id:106}',
    'removed{id:107}',
    'removed{id:508}',
    'removed{id:109}',
    'removed{id:110}',
    'removed{id:111}',
  ]);

  // No-op events: an unknown click, a tick.
  app.click(99);
  assert.deepEqual(app.changes(), []);
  app.tick();
  assert.deepEqual(app.changes(), []);

  // The WIT bootstrap rule.
  const app2 = await createApp(readFileSync(rel(todoPath)));
  assert.equal(app2.changes().length, 22, 'first-ever changes() should export the whole tree');

  // No unbounded growth: 60 more view() and 60 changes() calls must not
  // grow memory (both return guest-allocated buffers the host posts back).
  const before = app.memoryPages();
  for (let i = 0; i < 60; i++) {
    app.view();
    app.changes();
  }
  assert.ok(
    app.memoryPages() - before < 4,
    `view()/changes() leaked memory: ${before} -> ${app.memoryPages()} pages`,
  );
  console.log('todo: ok (text input + structural add/remove/re-parent + leak check)');
}

// ------------------------------------------------------------------ desktop

{
  const app = await createApp(readFileSync(rel(desktopPath)), {
    // An empty registry: the grid stays empty, so the view is the menu bar
    // and the empty grid area. The windowing hooks are wired as the browser
    // page wires them; the empty registry never opens a window.
    registry: [],
    onOpenWindow: () => 10000,
  });
  const view = () => app.view().map(node);

  // The menu bar: the name, the tagline, a spacer, the count, a hint, and
  // the repo link (a `link` node: a label, an href, a style — id-less).
  assert.deepEqual(view(), desktop.initial);

  // The diff channel never carries the id-less nodes (the WIT's identity
  // rule): a first-ever changes() exports the six id'd ones.
  const app2 = await createApp(readFileSync(rel(desktopPath)), { registry: [] });
  assert.equal(app2.changes().length, 6, "first-ever changes() should export the id'd nodes");

  // view() resets the diff baseline: nothing new since.
  assert.deepEqual(app.changes(), []);

  // The link is the host's, not the component's: the host opens the href,
  // and the component is never told (the WIT). An unknown id is the
  // component's no-op branch: no diff.
  app.click(99);
  assert.deepEqual(app.changes(), []);

  // No unbounded growth: 60 more view() and 60 changes() calls must not
  // grow memory (both return guest-allocated buffers the host posts back).
  const before = app.memoryPages();
  for (let i = 0; i < 60; i++) {
    app.view();
    app.changes();
  }
  assert.ok(
    app.memoryPages() - before < 4,
    `view()/changes() leaked memory: ${before} -> ${app.memoryPages()} pages`,
  );
  console.log('desktop: ok (menu bar + link node + empty grid)');
}

console.log('web selftest: all checks passed');
