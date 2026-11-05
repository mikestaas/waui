// Headless verification of the web host's ABI layer (Node; the same code
// the browser page uses). Exercises the counter's round-trip and diff
// channel, the stopwatch's import side (now + tick), and the todo's text
// input (on-text) and structural diff (add, remove, re-parent), against the
// raw wasm modules.
//
//   node web/selftest.mjs [counter.wasm] [stopwatch.wasm] [todo.wasm]
//
// The paths default to the Rust components; passing other core modules
// (e.g. the AssemblyScript builds, `make web-test-as`) runs the same checks
// against them — same glue.js, same assertions, a different producer.

import { readFileSync } from 'node:fs';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { createApp } from './glue.js';

// Resolve paths relative to the repository root (this file lives in web/);
// absolute paths (a non-default target) pass through.
const rel = (p) => (p.startsWith('/') ? p : fileURLToPath(new URL(`../${p}`, import.meta.url)));
const [counterPath, stopwatchPath, todoPath] = [
  process.argv[2] ?? 'target/wasm32-unknown-unknown/release/demo_counter.wasm',
  process.argv[3] ?? 'target/wasm32-unknown-unknown/release/demo_stopwatch.wasm',
  process.argv[4] ?? 'target/wasm32-unknown-unknown/release/demo_todo.wasm',
];

function node(n) {
  if (n.kind === 'container')
    return `container{id:${n.id},parent:${n.parent},dir:${n.direction},align:${n.align}}`;
  const style = `style:${n.style.color}:${n.style.size}`;
  if (n.kind === 'button')
    return `button{id:${n.id},parent:${n.parent},label:"${n.label}",${style}}`;
  if (n.kind === 'input')
    return `input{id:${n.id},parent:${n.parent},content:"${n.content}",${style}}`;
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

  expect([
    'container{id:1,parent:null,dir:column,align:center}',
    'button{id:2,parent:1,label:"+",style:positive:medium}',
    'text{id:3,parent:1,content:"0",style:neutral:large}',
    'button{id:4,parent:1,label:"-",style:negative:medium}',
  ]);

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

  // Two decrements: the number's content *and* color change (one
  // mutation), and view() is still the full truth.
  app.click(4);
  app.click(4);
  assert.deepEqual(app.changes().map(mutation), [
    'changed text{id:3,parent:1,content:"-1",style:negative:large} index:1',
  ]);
  expect([
    'container{id:1,parent:null,dir:column,align:center}',
    'button{id:2,parent:1,label:"+",style:positive:medium}',
    'text{id:3,parent:1,content:"-1",style:negative:large}',
    'button{id:4,parent:1,label:"-",style:negative:medium}',
  ]);

  // Bootstrap rule: a host that calls changes() first, without ever
  // calling view(), still gets the whole tree (as changed nodes).
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

  assert.deepEqual(view(), [
    'container{id:1,parent:null,dir:column,align:center}',
    'text{id:2,parent:1,content:"0 ms",style:neutral:large}',
    'button{id:3,parent:1,label:"start",style:positive:medium}',
  ]);

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
  assert.deepEqual(view(), [
    'container{id:1,parent:null,dir:column,align:center}',
    'text{id:2,parent:1,content:"todos",style:accent:large}',
    'container{id:3,parent:1,dir:column,align:start}',
    'container{id:4,parent:3,dir:column,align:start}',
    'container{id:104,parent:4,dir:row,align:center}',
    'button{id:105,parent:104,label:"○",style:neutral:medium}',
    'text{id:106,parent:104,content:"walk the dog",style:neutral:medium}',
    'button{id:107,parent:104,label:"✕",style:negative:medium}',
    'container{id:100,parent:4,dir:row,align:center}',
    'button{id:101,parent:100,label:"○",style:neutral:medium}',
    'text{id:102,parent:100,content:"buy milk",style:neutral:medium}',
    'button{id:103,parent:100,label:"✕",style:negative:medium}',
    'container{id:5,parent:3,dir:column,align:start}',
    'container{id:508,parent:5,dir:row,align:center}',
    'button{id:109,parent:508,label:"✓",style:positive:medium}',
    'text{id:110,parent:508,content:"water plants",style:warning:medium}',
    'button{id:111,parent:508,label:"✕",style:negative:medium}',
    'container{id:6,parent:1,dir:row,align:center}',
    'text{id:7,parent:6,content:"2 left",style:neutral:small}',
    'input{id:8,parent:6,content:"",style:neutral:medium}',
    'button{id:9,parent:6,label:"add",style:positive:medium}',
    'button{id:10,parent:6,label:"clear done",style:negative:medium}',
  ]);

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
    'changed input{id:8,parent:6,content:"buy coffee",style:neutral:medium} index:1',
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
    'changed input{id:8,parent:6,content:"",style:neutral:medium} index:1',
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
  assert.deepEqual(view(), [
    'container{id:1,parent:null,dir:column,align:center}',
    'text{id:2,parent:1,content:"todos",style:accent:large}',
    'container{id:3,parent:1,dir:column,align:start}',
    'container{id:4,parent:3,dir:column,align:start}',
    'container{id:112,parent:4,dir:row,align:center}',
    'button{id:113,parent:112,label:"○",style:neutral:medium}',
    'text{id:114,parent:112,content:"buy coffee",style:neutral:medium}',
    'button{id:115,parent:112,label:"✕",style:negative:medium}',
    'container{id:100,parent:4,dir:row,align:center}',
    'button{id:101,parent:100,label:"○",style:neutral:medium}',
    'text{id:102,parent:100,content:"buy milk",style:neutral:medium}',
    'button{id:103,parent:100,label:"✕",style:negative:medium}',
    'container{id:5,parent:3,dir:column,align:start}',
    'container{id:504,parent:5,dir:row,align:center}',
    'button{id:105,parent:504,label:"✓",style:positive:medium}',
    'text{id:106,parent:504,content:"walk the dog",style:warning:medium}',
    'button{id:107,parent:504,label:"✕",style:negative:medium}',
    'container{id:508,parent:5,dir:row,align:center}',
    'button{id:109,parent:508,label:"✓",style:positive:medium}',
    'text{id:110,parent:508,content:"water plants",style:warning:medium}',
    'button{id:111,parent:508,label:"✕",style:negative:medium}',
    'container{id:6,parent:1,dir:row,align:center}',
    'text{id:7,parent:6,content:"2 left",style:neutral:small}',
    'input{id:8,parent:6,content:"",style:neutral:medium}',
    'button{id:9,parent:6,label:"add",style:positive:medium}',
    'button{id:10,parent:6,label:"clear done",style:negative:medium}',
  ]);

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

  // Bootstrap rule: a host that calls changes() first, without ever
  // calling view(), still gets the whole tree (as changed nodes).
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

console.log('web selftest: all checks passed');
