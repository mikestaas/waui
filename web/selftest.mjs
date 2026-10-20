// Headless verification of the web host's ABI layer (Node, same code the
// browser page uses). Exercises the counter's round-trip, the diff channel
// (changes), and the stopwatch's import side (now + tick) against the raw
// wasm modules.
//
//   node web/selftest.mjs

import { readFileSync } from 'node:fs';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { createApp } from './glue.js';

// Resolve paths relative to the repository root (this file lives in web/).
const rel = (p) => fileURLToPath(new URL(`../${p}`, import.meta.url));

function node(n) {
  if (n.kind === 'container') return `container{id:${n.id},parent:${n.parent},dir:${n.direction}}`;
  const style = `style:${n.style.color}:${n.style.size}`;
  if (n.kind === 'button') return `button{id:${n.id},parent:${n.parent},label:"${n.label}",${style}}`;
  return `text{id:${n.id},parent:${n.parent},content:"${n.content}",${style}}`;
}

function mutation(m) {
  return m.kind === 'removed'
    ? `removed{id:${m.id}}`
    : `changed ${node(m.node)}`;
}

// ---------------------------------------------------------------- counter

{
  const app = await createApp(readFileSync(rel('target/wasm32-unknown-unknown/release/demo_counter.wasm')));
  const expect = (want) => {
    assert.deepEqual(
      app.view().map(node),
      want,
      `counter view mismatch (memory pages: ${app.memoryPages()})`,
    );
  };

  expect([
    'container{id:1,parent:null,dir:column}',
    'button{id:2,parent:1,label:"+",style:positive:medium}',
    'text{id:3,parent:1,content:"0",style:neutral:large}',
    'button{id:4,parent:1,label:"-",style:negative:medium}',
  ]);

  // The diff channel: a click dirties exactly the readout — never the
  // whole tree.
  app.click(2);
  assert.deepEqual(
    app.changes().map(mutation),
    ['changed text{id:3,parent:1,content:"1",style:neutral:large}'],
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
  assert.deepEqual(
    app.changes().map(mutation),
    ['changed text{id:3,parent:1,content:"-1",style:negative:large}'],
  );
  expect([
    'container{id:1,parent:null,dir:column}',
    'button{id:2,parent:1,label:"+",style:positive:medium}',
    'text{id:3,parent:1,content:"-1",style:negative:large}',
    'button{id:4,parent:1,label:"-",style:negative:medium}',
  ]);

  // Bootstrap rule: a host that calls changes() first, without ever
  // calling view(), still gets the whole tree (as changed nodes).
  const app2 = await createApp(readFileSync(rel('target/wasm32-unknown-unknown/release/demo_counter.wasm')));
  assert.equal(
    app2.changes().length,
    4,
    'first-ever changes() should export the whole tree',
  );

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
  const app = await createApp(readFileSync(rel('target/wasm32-unknown-unknown/release/demo_stopwatch.wasm')));
  const view = () => app.view().map(node);

  assert.deepEqual(view(), [
    'container{id:1,parent:null,dir:column}',
    'text{id:2,parent:1,content:"0 ms",style:neutral:large}',
    'button{id:3,parent:1,label:"start",style:positive:medium}',
  ]);

  // Start: both the readout (its color) and the button (label + color)
  // change — two mutations.
  app.click(3);
  let muts = app.changes().map(mutation);
  assert.equal(muts.length, 2);
  assert.match(muts[0], /^changed text\{.*content:"\d+ ms",style:accent:large\}$/);
  assert.equal(
    muts[1],
    'changed button{id:3,parent:1,label:"pause",style:warning:medium}',
  );

  // Let real time pass, tick: while running, only the readout changes.
  await new Promise((r) => setTimeout(r, 400));
  app.tick();
  muts = app.changes().map(mutation);
  assert.equal(muts.length, 1);
  const ms = Number(muts[0].match(/content:"(-?\d+) ms"/)[1]);
  assert.ok(ms >= 200 && ms < 2000, `unexpected elapsed time: ${ms} ms`);
  assert.match(muts[0], /style:accent:large/);

  // Pause: the readout and the button change again...
  app.click(3);
  muts = app.changes().map(mutation);
  assert.equal(muts.length, 2);
  assert.match(muts[0], /^changed text\{.*content:"\d+ ms",style:neutral:large\}$/);
  assert.equal(
    muts[1],
    'changed button{id:3,parent:1,label:"start",style:positive:medium}',
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

console.log('web selftest: all checks passed');
