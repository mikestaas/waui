// Headless verification of the web host's windowing ABI layer (Node; the
// same code the browser page uses). Runs the `demo-desktop` component
// against a two-entry registry and exercises the windowing import side
// (`list-entries`, `open-entry`) and the window count's round-trip: a
// double-click opens a window (the count goes up), a close drops it (the
// count goes down), and a window the host cannot open is dropped.
//
//   node web/desktop-selftest.mjs [demo-desktop.wasm]
//
// The path defaults to the Rust build. The expected views are local to this
// file, not in expect.mjs: the desktop's tree is built from the registry
// passed here, and no other suite serializes the same strings.

import { readFileSync } from 'node:fs';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { createApp } from './glue.js';

// Resolve paths relative to the repository root (this file lives in web/);
// absolute paths (a non-default target) pass through.
const rel = (p) => (p.startsWith('/') ? p : fileURLToPath(new URL(`../${p}`, import.meta.url)));
const [desktopPath] = [
  process.argv[2] ?? 'target/wasm32-unknown-unknown/release/demo_desktop.wasm',
];

// The two-entry registry: the grid's source of truth for this test. The
// `data` blob carries the tile's presentation (the label, the caption, the
// icon) and the window's default size.
const REGISTRY = [
  {
    name: 'counter-rust',
    src: 'demo-counter.wasm',
    data: JSON.stringify({
      label: 'Counter',
      lang: 'Rust',
      icon: 'icons/counter-rust.svg',
      width: 300,
      height: 380,
    }),
  },
  {
    name: 'todo-wat',
    src: 'demo-todo-wat.wasm',
    data: JSON.stringify({
      label: 'Todo',
      lang: 'WAT',
      icon: 'icons/todo-wat.svg',
      width: 460,
      height: 500,
    }),
  },
];

function node(n) {
  if (n.kind === 'container')
    return `container{id:${n.id},parent:${n.parent},dir:${n.direction},align:${n.align}}`;
  // An image has no style: the `src` and the size (a host default when 0).
  if (n.kind === 'image')
    return `image{id:${n.id},parent:${n.parent},src:"${n.src}",width:${n.width},height:${n.height}}`;
  const style = `style:${n.style.color}:${n.style.size}`;
  if (n.kind === 'button')
    return `button{id:${n.id},parent:${n.parent},label:"${n.label}",${style}}`;
  if (n.kind === 'input')
    return `input{id:${n.id},parent:${n.parent},content:"${n.content}",${style},width:${n.width}}`;
  return `text{id:${n.id},parent:${n.parent},content:"${n.content}",${style}}`;
}

function mutation(m) {
  return m.kind === 'removed' ? `removed{id:${m.id}}` : `changed ${node(m.node)} index:${m.index}`;
}

// The desktop's whole view for a given window count: the menu bar, one row
// (two tiles fit in the three-column grid), and the two tiles (an icon, a
// label, and a caption each).
function grid(count) {
  return [
    'container{id:null,parent:null,dir:column,align:center}',
    'container{id:1,parent:null,dir:row,align:center}',
    'text{id:null,parent:1,content:"waui",style:accent:large}',
    `text{id:2,parent:1,content:"${count}",style:neutral:small}`,
    'container{id:3,parent:null,dir:column,align:center}',
    'container{id:10,parent:3,dir:row,align:center}',
    'container{id:100,parent:10,dir:column,align:center}',
    'image{id:null,parent:100,src:"icons/counter-rust.svg",width:112,height:112}',
    'text{id:null,parent:100,content:"Counter",style:neutral:medium}',
    'text{id:null,parent:100,content:"Rust",style:neutral:small}',
    'container{id:101,parent:10,dir:column,align:center}',
    'image{id:null,parent:101,src:"icons/todo-wat.svg",width:112,height:112}',
    'text{id:null,parent:101,content:"Todo",style:neutral:medium}',
    'text{id:null,parent:101,content:"WAT",style:neutral:small}',
  ];
}

// The host's windowing layer: records each `open-entry` call and hands out
// handles from a queue; once the queue is empty it reports a not-found, so
// the error path is exercised by a third open of the same tile.
const openCalls = [];
const handles = [7, 8];
const desktop = await createApp(readFileSync(rel(desktopPath)), {
  registry: REGISTRY,
  onOpenWindow: (name, x, y, w, h) => {
    openCalls.push({ name, x, y, w, h });
    return handles.length > 0 ? handles.shift() : null;
  },
});

// -------------------------------------------------------------------- the grid

{
  // The registry is fetched on the first tree build, and the grid is built
  // from it: the menu bar (the count at zero), one row, two tiles.
  const view = () => desktop.view().map(node);
  assert.deepEqual(
    view(),
    grid('0 windows'),
    `grid view mismatch (memory pages: ${desktop.memoryPages()})`,
  );

  // Nothing has changed since the view: an empty diff.
  assert.deepEqual(desktop.changes(), []);

  // The WIT bootstrap rule, on a fresh instance that never called view():
  // every id'd node is exported (the six containers and the count; the
  // id-less images and labels are not addressable, so they are not).
  const fresh = await createApp(readFileSync(rel(desktopPath)), {
    registry: REGISTRY,
    onOpenWindow: () => null,
  });
  assert.deepEqual(fresh.changes().map(mutation), [
    'changed container{id:1,parent:null,dir:row,align:center} index:0',
    'changed text{id:2,parent:1,content:"0 windows",style:neutral:small} index:1',
    'changed container{id:3,parent:null,dir:column,align:center} index:1',
    'changed container{id:10,parent:3,dir:row,align:center} index:0',
    'changed container{id:100,parent:10,dir:column,align:center} index:0',
    'changed container{id:101,parent:10,dir:column,align:center} index:1',
  ]);
  console.log('grid: ok (registry -> view + bootstrap diff)');
}

// --------------------------------------------------------- the window round-trip

{
  // A double-click on a tile opens its entry: the import carries the entry's
  // name, the blob's default size, and the host's placement (0, 0 = the
  // default cascade).
  desktop.doubleClick(100);
  assert.deepEqual(openCalls, [{ name: 'counter-rust', x: 0, y: 0, w: 300, h: 380 }]);
  // The count is the only node that changed — the grid is untouched.
  assert.deepEqual(desktop.changes().map(mutation), [
    'changed text{id:2,parent:1,content:"1 window",style:neutral:small} index:1',
  ]);
  assert.deepEqual(desktop.view().map(node), grid('1 window'));

  // A second window: the plural returns, and the count is the only change.
  desktop.doubleClick(101);
  assert.deepEqual(openCalls, [
    { name: 'counter-rust', x: 0, y: 0, w: 300, h: 380 },
    { name: 'todo-wat', x: 0, y: 0, w: 460, h: 500 },
  ]);
  assert.deepEqual(desktop.changes().map(mutation), [
    'changed text{id:2,parent:1,content:"2 windows",style:neutral:small} index:1',
  ]);

  // A window the host cannot open is dropped (the handle queue is exhausted):
  // the count does not move, and no diff is reported.
  desktop.doubleClick(100);
  assert.equal(openCalls.length, 3, 'the third open must reach the host');
  assert.deepEqual(desktop.changes(), []);

  // Closes drop windows from the open set, one at a time.
  desktop.windowClose(8);
  assert.deepEqual(desktop.changes().map(mutation), [
    'changed text{id:2,parent:1,content:"1 window",style:neutral:small} index:1',
  ]);
  desktop.windowClose(7);
  assert.deepEqual(desktop.changes().map(mutation), [
    'changed text{id:2,parent:1,content:"0 windows",style:neutral:small} index:1',
  ]);
  assert.deepEqual(desktop.view().map(node), grid('0 windows'));

  // A double-click on a non-tile id is the component's own no-op branch: no
  // open, no diff.
  desktop.doubleClick(2);
  assert.equal(openCalls.length, 3, 'a non-tile id must not open a window');
  assert.deepEqual(desktop.changes(), []);

  // No unbounded growth: 60 more view() and 60 changes() calls must not grow
  // memory (both return guest-allocated buffers the host posts back). The
  // registry's strings are adopted by the component and kept in a static —
  // they are allocated once, at the first tree build, not per call.
  const before = desktop.memoryPages();
  for (let i = 0; i < 60; i++) {
    desktop.view();
    desktop.changes();
  }
  assert.ok(
    desktop.memoryPages() - before < 4,
    `view()/changes() leaked memory: ${before} -> ${desktop.memoryPages()} pages`,
  );
  console.log('windows: ok (open/close round-trip + error path + leak check)');
}

console.log('desktop selftest: all checks passed');
