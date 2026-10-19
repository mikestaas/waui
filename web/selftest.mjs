// Headless verification of the web host's ABI layer (Node, same code the
// browser page uses). Exercises the counter's round-trip and the
// stopwatch's import side (now + tick) against the raw wasm modules.
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

  app.click(2);
  expect([
    'container{id:1,parent:null,dir:column}',
    'button{id:2,parent:1,label:"+",style:positive:medium}',
    'text{id:3,parent:1,content:"1",style:neutral:large}',
    'button{id:4,parent:1,label:"-",style:negative:medium}',
  ]);

  // Two decrements: exercises the negative-number branch (a second
  // character in the string buffer) — and the number's color flips
  // with its sign.
  app.click(4);
  app.click(4);
  expect([
    'container{id:1,parent:null,dir:column}',
    'button{id:2,parent:1,label:"+",style:positive:medium}',
    'text{id:3,parent:1,content:"-1",style:negative:large}',
    'button{id:4,parent:1,label:"-",style:negative:medium}',
  ]);

  // Ignore an unknown widget id (the component's own no-op branch).
  app.click(99);
  app.view();

  // No unbounded growth: 60 more view() calls must not grow memory.
  const before = app.memoryPages();
  for (let i = 0; i < 60; i++) app.view();
  assert.ok(
    app.memoryPages() - before < 4,
    `view() leaked memory: ${before} -> ${app.memoryPages()} pages`,
  );
  console.log('counter: ok (round-trip + leak check)');
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

  // Start, let real time pass, tick, and confirm the elapsed time is
  // honest (the component's now() import, driven by the host clock).
  app.click(3);
  assert.equal(
    view()[2],
    'button{id:3,parent:1,label:"pause",style:warning:medium}',
  );
  // While running, the readout asks for the host's accent color.
  assert.match(view()[1], /style:accent:large/);
  await new Promise((r) => setTimeout(r, 400));
  app.tick();
  const [, text] = view(); // node order: container, text, button
  const ms = Number(text.match(/content:"(-?\d+) ms"/)[1]);
  assert.ok(ms >= 200 && ms < 2000, `unexpected elapsed time: ${ms} ms`);

  // Pause: a further tick must not change the reading.
  app.click(3);
  const frozen = view()[1];
  await new Promise((r) => setTimeout(r, 300));
  app.tick();
  assert.equal(view()[1], frozen, 'paused stopwatch kept advancing');

  console.log(`stopwatch: ok (ran ${ms} ms of real time)`);
}

console.log('web selftest: all checks passed');
