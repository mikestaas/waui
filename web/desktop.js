// The desktop host: the windowing layer for the `demo-desktop` component.
//
// It runs the desktop component on the `desktop` world (createDesktop from
// glue.js): the component owns the icon grid and the menu bar's window count,
// and this file implements the two imports it calls. `list-demos` is the
// registry below; `open-window` opens a window (openWindow) that embeds a demo
// component (the `app` world) rendered in place, not in an iframe.
//
// The demos are pre-compiled at startup (WebAssembly.compile, async) so that
// `open-window` — a sync wasm call — can instantiate one synchronously (new
// WebAssembly.Instance on the pre-compiled module).
//
// This file is concatenated into the generated page by build-desktop.mjs, so
// it relies on `createDesktop`/`instantiateApp` from glue.js and
// `WAUI_WASM_BYTES` from the boot script.

// The demo registry, as the host provides it to the component's `list-demos`.
// `src` is the demo module's URL: the host pre-compiles it (keyed by URL) and
// the component passes it back to `open-window` when a tile is activated.
const DEMOS = [
  {
    name: 'Counter',
    lang: 'Rust',
    icon: 'icons/counter-rust.svg',
    src: 'demo-counter.wasm',
    w: 300,
    h: 380,
  },
  {
    name: 'Counter',
    lang: 'AssemblyScript',
    icon: 'icons/counter-as.svg',
    src: 'demo-counter-as.wasm',
    w: 300,
    h: 380,
  },
  {
    name: 'Counter',
    lang: 'WAT',
    icon: 'icons/counter-wat.svg',
    src: 'demo-counter-wat.wasm',
    w: 300,
    h: 380,
  },
  {
    name: 'Stopwatch',
    lang: 'Rust',
    icon: 'icons/stopwatch-rust.svg',
    src: 'demo-stopwatch.wasm',
    w: 300,
    h: 380,
  },
  {
    name: 'Stopwatch',
    lang: 'AssemblyScript',
    icon: 'icons/stopwatch-as.svg',
    src: 'demo-stopwatch-as.wasm',
    w: 300,
    h: 380,
  },
  {
    name: 'Stopwatch',
    lang: 'WAT',
    icon: 'icons/stopwatch-wat.svg',
    src: 'demo-stopwatch-wat.wasm',
    w: 300,
    h: 380,
  },
  {
    name: 'Todo',
    lang: 'Rust',
    icon: 'icons/todo-rust.svg',
    src: 'demo-todo.wasm',
    w: 460,
    h: 500,
  },
  {
    name: 'Todo',
    lang: 'AssemblyScript',
    icon: 'icons/todo-as.svg',
    src: 'demo-todo-as.wasm',
    w: 460,
    h: 500,
  },
  {
    name: 'Todo',
    lang: 'WAT',
    icon: 'icons/todo-wat.svg',
    src: 'demo-todo-wat.wasm',
    w: 460,
    h: 500,
  },
];

// The demo modules, pre-compiled at startup and keyed by URL (the registry's
// `src`). `open-window` looks one up to instantiate it synchronously.
const modules = new Map();
async function preloadDemos() {
  await Promise.all(
    DEMOS.map(async (d) => {
      const res = await fetch(d.src);
      if (!res.ok) throw new Error(`HTTP ${res.status} for ${d.src}`);
      const bytes = new Uint8Array(await res.arrayBuffer());
      modules.set(d.src, await WebAssembly.compile(bytes));
    }),
  );
}

// --------------------------------------------------------------------------
// The windowing layer: the host-drawn chrome around each embedded demo.

const MENUBAR_H = 34;
const windowsEl = document.getElementById('windows');
const countEl = document.getElementById('window-count');

let z = 10; // the next z-index to assign (a raised window gets the top)
let cascade = 0; // the new-window placement counter (wraps after 9)
let nextHandle = 1; // the next window handle (the host's flat id)

// A window: its element, its handle, and the embedded demo's renderer.
const windows = new Map();

function raise(win) {
  win.el.style.zIndex = ++z;
  win.el.dataset.z = z;
}

function updateCount() {
  const n = windows.size;
  countEl.textContent = n + (n === 1 ? ' window' : ' windows');
}

// Close a window: drop it from the set, remove its element, and tell the
// component (so it drops the handle from its open set and updates the count).
function closeWindow(win) {
  if (win.timer) clearInterval(win.timer);
  windows.delete(win.handle);
  win.el.remove();
  desktop.windowClose(win.handle);
  refreshDesktop();
  updateCount();
}

// The embedded demo's DOM renderer: the app host's retained-tree + diff +
// event delegation, scoped to the window's content element. The demo runs on
// the `app` world, so it is the same contract the standalone demo pages use.
function renderDemo(api, contentEl) {
  let retained = api.view();
  let elements = new Map();

  const elFor = (n, childrenOf) => {
    let el;
    if (n.kind === 'container') {
      el = document.createElement('div');
      el.className = `waui waui-${n.direction} waui-align-${n.align}`;
    } else if (n.kind === 'button') {
      el = document.createElement('button');
      el.className = `waui waui-button waui-${n.style.color} waui-${n.style.size}`;
      el.textContent = n.label;
    } else if (n.kind === 'input') {
      el = document.createElement('input');
      el.className = `waui waui-input waui-${n.style.color} waui-${n.style.size}`;
      el.value = n.content;
      el.style.width = n.width > 0 ? `${n.width}ch` : '';
    } else if (n.kind === 'image') {
      el = document.createElement('img');
      el.className = 'waui waui-image';
      el.src = n.src;
      el.alt = '';
    } else {
      el = document.createElement('div');
      el.className = `waui waui-text waui-${n.style.color} waui-${n.style.size}`;
      el.textContent = n.content;
    }
    if (n.id !== null) {
      el.dataset.id = n.id;
      elements.set(n.id, el);
    }
    for (const child of childrenOf.get(n.id) ?? []) el.appendChild(elFor(child, childrenOf));
    return el;
  };

  function rebuild() {
    elements = new Map();
    const childrenOf = new Map();
    for (const n of retained) {
      if (n.parent !== null) {
        const list = childrenOf.get(n.parent) ?? [];
        list.push(n);
        childrenOf.set(n.parent, list);
      }
    }
    const roots = retained.filter((n) => n.parent === null).map((n) => elFor(n, childrenOf));
    let top;
    if (roots.length === 0) {
      top = document.createElement('div');
    } else if (roots.length === 1) {
      top = roots[0];
    } else {
      top = document.createElement('div');
      top.className = 'waui waui-column waui-align-center';
      for (const r of roots) top.appendChild(r);
    }
    contentEl.replaceChildren(top);
  }

  function refresh() {
    const mutations = api.changes();
    if (mutations.length === 0) return;
    // The demos are small; a full rebuild on any change is cheap and correct.
    for (const m of mutations) {
      if (m.kind === 'removed') {
        const p = retained.findIndex((x) => x.id === m.id);
        if (p >= 0) retained.splice(p, 1);
      } else if (m.node.id !== null) {
        const i = retained.findIndex((x) => x.id === m.node.id);
        if (i >= 0) retained[i] = m.node;
        else retained.push(m.node);
      }
    }
    rebuild();
  }

  // Event delegation: one listener per kind survives the rebuilds.
  contentEl.addEventListener('click', (e) => {
    const btn = e.target.closest('button[data-id]');
    if (!btn) return;
    api.click(Number(btn.dataset.id));
    refresh();
  });
  contentEl.addEventListener('input', (e) => {
    const field = e.target.closest('input[data-id]');
    if (!field) return;
    api.text(Number(field.dataset.id), field.value);
    refresh();
  });

  rebuild();
  return { refresh };
}

// Open a window running the demo at `src`, titled `title`. The geometry
// (`x`, `y`, `w`, `h`) is the component's request: a 0 dimension is the
// host's default (the demo's registry size, or a cascade placement). Returns
// the host's window handle, or null if the demo cannot be run.
function openWindow(src, title, x, y, w, h) {
  const demo = DEMOS.find((d) => d.src === src);
  const compiled = modules.get(src);
  if (!demo || !compiled) return null; // not a demo the host can run

  const width = w > 0 ? w : demo.w;
  const height = h > 0 ? h : demo.h;

  const el = document.createElement('div');
  el.className = 'window';
  el.style.width = width + 'px';
  el.style.height = height + 'px';

  // New windows cascade down-right, then wrap back to the top-left.
  const step = 28;
  const n = cascade++ % 9;
  const left = Math.max(8, Math.min(24 + n * step, innerWidth - width - 8));
  const top = Math.max(MENUBAR_H + 4, Math.min(48 + n * step, innerHeight - height - 8));
  el.style.left = left + 'px';
  el.style.top = top + 'px';

  // The title bar: the icon, the title, and the close button.
  const bar = document.createElement('div');
  bar.className = 'titlebar';
  const icon = document.createElement('img');
  icon.src = demo.icon;
  icon.alt = '';
  const titleLabel = document.createElement('span');
  titleLabel.className = 'titlebar-title';
  titleLabel.textContent = title;
  const close = document.createElement('button');
  close.className = 'close';
  close.textContent = '×';
  close.setAttribute('aria-label', 'Close ' + title);
  bar.append(icon, titleLabel, close);
  el.append(bar);

  // The content: the embedded demo's tree, rendered in place.
  const content = document.createElement('div');
  content.className = 'window-content';
  el.append(content);

  // The corner grip, which resizes the window.
  const grip = document.createElement('div');
  grip.className = 'grip';
  grip.setAttribute('aria-hidden', 'true');
  el.append(grip);

  windowsEl.append(el);

  const handle = nextHandle++;
  // Instantiate the demo (sync, from the pre-compiled module) and render it.
  // `onSchedule` arms a per-window wake loop for the demo's time-based refresh.
  let timer = null;
  const api = instantiateApp(compiled, {
    onSchedule: (ms) => {
      if (timer) clearInterval(timer);
      timer = ms > 0 ? setInterval(wake, ms) : null;
    },
  });
  const renderer = renderDemo(api, content);
  function wake() {
    api.tick();
    renderer.refresh();
  }

  const win = { handle, el, timer };
  windows.set(handle, win);

  // The close button drops the window (and tells the component).
  close.addEventListener('click', (e) => {
    e.stopPropagation();
    closeWindow(win);
  });

  // A pointerdown on the chrome raises the window and, on the title bar,
  // starts a drag. The demo's elements live inside the content (in place, not
  // in an iframe), so a press on them must not start a drag: the capture
  // would swallow the click and the demo would never see it.
  el.addEventListener('pointerdown', (e) => {
    if (content.contains(e.target) || e.target === grip || e.target === close) return;
    raise(win);
    startDrag(e, win, bar);
  });

  // A pointerdown inside the content raises the window (the demo's own events
  // are handled by the content's delegated listeners).
  content.addEventListener('pointerdown', () => raise(win));

  // The grip resizes the window, clamped to a usable minimum and the viewport.
  grip.addEventListener('pointerdown', (e) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    grip.setPointerCapture(e.pointerId);
    raise(win);
    const startW = el.offsetWidth;
    const startH = el.offsetHeight;
    const startX = e.clientX;
    const startY = e.clientY;
    grip.classList.add('dragging');
    const move = (ev) => {
      const nw = Math.max(
        220,
        Math.min(startW + ev.clientX - startX, innerWidth - el.offsetLeft - 8),
      );
      const nh = Math.max(
        260,
        Math.min(startH + ev.clientY - startY, innerHeight - el.offsetTop - 8),
      );
      el.style.width = nw + 'px';
      el.style.height = nh + 'px';
    };
    const up = () => {
      removeEventListener('pointermove', move);
      removeEventListener('pointerup', up);
      grip.classList.remove('dragging');
    };
    addEventListener('pointermove', move);
    addEventListener('pointerup', up);
  });

  raise(win);
  return handle;
}

// Drag by the title bar, clamped so the bar stays on screen and clear of the
// menu bar. The capture keeps the moves flowing to this document: without it,
// moves over the content would not reach the window-level listeners.
function startDrag(e, win, bar) {
  if (e.button !== 0) return;
  e.preventDefault();
  bar.setPointerCapture(e.pointerId);
  const rect = win.el.getBoundingClientRect();
  const dx = e.clientX - rect.left;
  const dy = e.clientY - rect.top;
  bar.classList.add('dragging');
  const move = (ev) => {
    const x = Math.max(8 - rect.width + 64, Math.min(ev.clientX - dx, innerWidth - 64));
    const y = Math.max(MENUBAR_H, Math.min(ev.clientY - dy, innerHeight - bar.offsetHeight));
    win.el.style.left = x + 'px';
    win.el.style.top = y + 'px';
  };
  const up = () => {
    removeEventListener('pointermove', move);
    removeEventListener('pointerup', up);
    bar.classList.remove('dragging');
  };
  addEventListener('pointermove', move);
  addEventListener('pointerup', up);
}

// Escape closes the frontmost window (when the desktop has focus; keys typed
// inside a window go to its content).
addEventListener('keydown', (e) => {
  if (e.key !== 'Escape') return;
  let top = null;
  for (const w of windows.values()) {
    if (!top || Number(w.el.dataset.z) > Number(top.el.dataset.z)) top = w;
  }
  if (top) closeWindow(top);
});

// --------------------------------------------------------------------------
// The desktop component: the grid and the menu bar's count.

// Boot: pre-compile the demos, then run the desktop component. The pre-compile
// is awaited so a double-click always finds a ready module.
await preloadDemos();

const desktop = await createDesktop(WAUI_WASM_BYTES, DEMOS, {
  onOpenWindow: (src, title, x, y, w, h) => openWindow(src, title, x, y, w, h),
});

const gridEl = document.getElementById('grid');

// The retained node list (the component's grid + menu bar), as the diff
// protocol maintains it: a full snapshot from `view()`, then `changes()`.
let retained = desktop.view();
let elements = new Map();

// The node with `id` and its descendants, in pre-order (a subtree is a
// contiguous run of the flat list).
function subtree(id) {
  const out = [];
  const i = retained.findIndex((n) => n.id === id);
  if (i < 0) return out;
  out.push(retained[i]);
  let j = i + 1;
  while (j < retained.length && descendantOf(retained[j], id)) {
    out.push(retained[j]);
    j++;
  }
  return out;
}

function descendantOf(node, ancestor) {
  let cur = node.parent;
  while (cur !== null) {
    if (cur === ancestor) return true;
    const parent = retained.find((n) => n.id === cur);
    cur = parent ? parent.parent : null;
  }
  return false;
}

// Render a subtree (a flat pre-order list) into a container. The host tags
// the grid's structure: the grid, its rows, and the tiles (a column with an
// image) get their desktop classes, so the stylesheet can lay them out.
function renderSubtree(rootId, container) {
  const nodes = subtree(rootId);
  if (nodes.length === 0) {
    container.replaceChildren();
    return;
  }
  const childrenOf = new Map();
  for (const n of nodes) {
    if (n.parent !== null) {
      const list = childrenOf.get(n.parent) ?? [];
      list.push(n);
      childrenOf.set(n.parent, list);
    }
  }
  const elFor = (n, childIndex) => {
    let el;
    if (n.kind === 'container') {
      el = document.createElement('div');
      el.className = `waui waui-${n.direction} waui-align-${n.align}`;
      // The grid's structure, tagged for the stylesheet.
      if (n.id === 3) el.classList.add('grid');
      else if (n.id !== null && n.id >= 10 && n.id < 100) el.classList.add('grid-row');
      else if (n.id !== null && n.id >= 100) el.classList.add('tile');
    } else if (n.kind === 'image') {
      el = document.createElement('img');
      el.className = 'waui waui-image';
      el.src = n.src;
      el.alt = '';
      el.style.width = n.width > 0 ? `${n.width}px` : '';
      el.style.height = n.height > 0 ? `${n.height}px` : '';
    } else {
      el = document.createElement('div');
      el.className = `waui waui-text waui-${n.style.color} waui-${n.style.size}`;
      // A tile's name (its 2nd child) and language (3rd), restyled by the
      // desktop's .tile-name/.tile-lang rules (the reference's tile labels).
      if (n.parent !== null && n.parent >= 100) {
        if (childIndex === 1) el.classList.add('tile-name');
        else if (childIndex === 2) el.classList.add('tile-lang');
      }
      el.textContent = n.content;
    }
    if (n.id !== null) {
      el.dataset.id = n.id;
      elements.set(n.id, el);
    }
    let ci = 0;
    for (const child of childrenOf.get(n.id) ?? []) el.appendChild(elFor(child, ci++));
    return el;
  };
  container.replaceChildren(elFor(nodes[0]));
}

function refreshDesktop() {
  const mutations = desktop.changes();
  if (mutations.length === 0) return;
  for (const m of mutations) {
    if (m.kind === 'removed') {
      const p = retained.findIndex((x) => x.id === m.id);
      if (p >= 0) retained.splice(p, 1);
    } else if (m.node.id !== null) {
      const i = retained.findIndex((x) => x.id === m.node.id);
      if (i >= 0) retained[i] = m.node;
      else retained.push(m.node);
    }
  }
  // The grid is static (the component never adds or removes tiles); only the
  // count changes. Re-render the grid (cheap) and update the count in place.
  renderSubtree(3, gridEl);
  const count = retained.find((n) => n.id === 2);
  if (count) countEl.textContent = count.content;
}

// The initial render: the grid, and the count (the component's "0 windows").
renderSubtree(3, gridEl);
{
  const count = retained.find((n) => n.id === 2);
  if (count) countEl.textContent = count.content;
}

// A double-clicked tile opens its demo. The host maps Enter/Space on a
// focused tile to the same. The count update lands through the diff.
gridEl.addEventListener('dblclick', (e) => {
  const tile = e.target.closest('[data-id].tile');
  if (!tile) return;
  desktop.doubleClick(Number(tile.dataset.id));
  refreshDesktop();
});
gridEl.addEventListener('keydown', (e) => {
  if (e.key !== 'Enter' && e.key !== ' ') return;
  const tile = e.target.closest('[data-id].tile');
  if (!tile) return;
  e.preventDefault();
  desktop.doubleClick(Number(tile.dataset.id));
  refreshDesktop();
});
// A single click selects the tile (the reference's behavior).
gridEl.addEventListener('click', (e) => {
  const tile = e.target.closest('[data-id].tile');
  if (!tile) return;
  for (const t of gridEl.querySelectorAll('.tile')) t.classList.toggle('selected', t === tile);
});
