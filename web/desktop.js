// The desktop host: the windowing layer for the `demo-desktop` component.
//
// It runs the desktop component via `createApp` from glue.js: the component
// owns the icon grid and the menu bar's window count, and this file
// implements the windowing imports it calls. `list-entries` is the registry
// below; `open-entry` opens a window (openWindow) that embeds a demo
// component rendered in place, not in an iframe.
//
// The demos are pre-compiled at startup (WebAssembly.compile, async) so that
// `open-entry` — a sync wasm call — can instantiate one synchronously (new
// WebAssembly.Instance on the pre-compiled module).
//
// This file is concatenated into the generated page by build-desktop.mjs, so
// it relies on `createApp`/`instantiateApp` from glue.js and
// `WAUI_WASM_BYTES` from the boot script.

// The entry registry, as the host provides it to the component's
// `list-entries`. Each entry's `name` is its unique identifier (the display
// label is in the `data` blob); its `src` is the demo module's URL —
// host-internal: the host pre-compiles it (keyed by URL) and resolves an
// `open-entry` name to it. `data` is a JSON object of per-entry metadata the
// component parses.
const DEMOS = [
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
    name: 'counter-as',
    src: 'demo-counter-as.wasm',
    data: JSON.stringify({
      label: 'Counter',
      lang: 'AssemblyScript',
      icon: 'icons/counter-as.svg',
      width: 300,
      height: 380,
    }),
  },
  {
    name: 'counter-wat',
    src: 'demo-counter-wat.wasm',
    data: JSON.stringify({
      label: 'Counter',
      lang: 'WAT',
      icon: 'icons/counter-wat.svg',
      width: 300,
      height: 380,
    }),
  },
  {
    name: 'stopwatch-rust',
    src: 'demo-stopwatch.wasm',
    data: JSON.stringify({
      label: 'Stopwatch',
      lang: 'Rust',
      icon: 'icons/stopwatch-rust.svg',
      width: 300,
      height: 380,
    }),
  },
  {
    name: 'stopwatch-as',
    src: 'demo-stopwatch-as.wasm',
    data: JSON.stringify({
      label: 'Stopwatch',
      lang: 'AssemblyScript',
      icon: 'icons/stopwatch-as.svg',
      width: 300,
      height: 380,
    }),
  },
  {
    name: 'stopwatch-wat',
    src: 'demo-stopwatch-wat.wasm',
    data: JSON.stringify({
      label: 'Stopwatch',
      lang: 'WAT',
      icon: 'icons/stopwatch-wat.svg',
      width: 300,
      height: 380,
    }),
  },
  {
    name: 'todo-rust',
    src: 'demo-todo.wasm',
    data: JSON.stringify({
      label: 'Todo',
      lang: 'Rust',
      icon: 'icons/todo-rust.svg',
      width: 460,
      height: 500,
    }),
  },
  {
    name: 'todo-as',
    src: 'demo-todo-as.wasm',
    data: JSON.stringify({
      label: 'Todo',
      lang: 'AssemblyScript',
      icon: 'icons/todo-as.svg',
      width: 460,
      height: 500,
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

// The demo modules, pre-compiled at startup and keyed by URL (the registry's
// `src`). `open-entry` resolves a name to its entry, then to a module.
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
// The host's default window size, for a dimension the component leaves at 0
// and the entry's blob does not set.
const DEFAULT_W = 300;
const DEFAULT_H = 380;
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
//
// Content-only mutations (a text, button, or input whose position in the tree
// is unchanged) are applied in place — the existing element is updated, no DOM
// rebuild — so a field being typed in keeps its focus and caret. Anything
// structural (removals, additions, re-parenting, container changes) falls back
// to a full rebuild from the retained tree; an empty diff does nothing.
function renderDemo(api, contentEl) {
  let retained = api.view();
  // Widget id -> its DOM element, for in-place updates.
  let elements = new Map();

  // The retained list's diff machinery, scoped to this window's tree (the app
  // host's, web/app.js): a `changes` diff applied by node id.

  // Whether `node` is in the subtree rooted at `ancestor`.
  function descendantOf(node, ancestor) {
    let cur = node.parent;
    while (cur !== null) {
      if (cur === ancestor) return true;
      const parent = retained.find((n) => n.id === cur);
      cur = parent ? parent.parent : null;
    }
    return false;
  }

  // Insert a node the host does not have at `index` among its parent's
  // children (0 is the front; an index past the end appends), or at the end
  // of the list if it is a new root. The walk skips the node's own
  // descendants still in the list, so a re-added container lands after them.
  function insertNode(n, index) {
    const p = retained.findIndex((x) => x.id === n.parent);
    if (p < 0) {
      retained.push(n); // a new root
      return;
    }
    let i = p + 1;
    let siblings = 0;
    while (i < retained.length) {
      const x = retained[i];
      if (descendantOf(x, n.id)) {
        i++; // the node's own descendant
        continue;
      }
      if (!descendantOf(x, n.parent)) break; // past the parent's subtree
      if (x.parent === n.parent) {
        if (siblings >= index) break; // before the `index`-th sibling
        siblings++;
        const childId = x.id;
        i++;
        while (i < retained.length && descendantOf(retained[i], childId)) i++;
      } else {
        i++; // a deeper descendant: advance
      }
    }
    retained.splice(i, 0, n);
  }

  // Remove a node and, if it is a container, its subtree. Pre-order makes a
  // subtree a contiguous run of the list.
  function removeSubtree(id) {
    const p = retained.findIndex((n) => n.id === id);
    if (p < 0) return;
    let end = p + 1;
    while (end < retained.length && descendantOf(retained[end], id)) end++;
    retained.splice(p, end - p);
  }

  // Apply a diff to the retained list.
  function apply(mutations) {
    for (const m of mutations) {
      if (m.kind === 'removed') {
        removeSubtree(m.id);
      } else if (m.node.id === null) {
        // An id-less node is unaddressable (see the WIT identity note):
        // the diff cannot name it, so ignore it rather than guess.
      } else {
        const n = m.node;
        const i = retained.findIndex((x) => x.id === n.id);
        if (i >= 0) {
          if (retained[i].parent === n.parent) {
            retained[i] = n; // a changed node keeps its position (index ignored)
          } else {
            // Re-parented: move it to `index` among the new parent's children.
            retained.splice(i, 1);
            insertNode(n, m.index);
          }
        } else {
          insertNode(n, m.index); // a node the host has never seen
        }
      }
    }
  }

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
      // The dataset copy is what the e2e DOM serializer reads back.
      el.dataset.width = n.width;
      el.style.width = n.width > 0 ? `${n.width}ch` : '';
    } else if (n.kind === 'image') {
      el = document.createElement('img');
      el.className = 'waui waui-image';
      el.src = n.src;
      el.alt = '';
      el.dataset.width = n.width;
      el.dataset.height = n.height;
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

  // Restyle an element in place from its node. The element is not replaced, so
  // a field being typed in keeps its focus and caret. Returns whether the
  // element was found.
  function restyle(n) {
    if (n.id === null) return false; // unaddressable
    const el = elements.get(n.id);
    if (!el) return false;
    if (n.kind === 'button') {
      el.textContent = n.label;
      el.className = `waui waui-button waui-${n.style.color} waui-${n.style.size}`;
    } else if (n.kind === 'text') {
      el.textContent = n.content;
      el.className = `waui waui-text waui-${n.style.color} waui-${n.style.size}`;
    } else if (n.kind === 'input') {
      // Apply the component's answer, but skip the field when it just echoes
      // the buffer the user typed (setting .value to the same text moves the
      // caret in some browsers).
      if (el.value !== n.content) el.value = n.content;
      el.className = `waui waui-input waui-${n.style.color} waui-${n.style.size}`;
      el.dataset.width = n.width;
      el.style.width = n.width > 0 ? `${n.width}ch` : '';
    } else if (n.kind === 'image') {
      // A changed image keeps its element; update the `src` and the size.
      el.src = n.src;
      el.dataset.width = n.width;
      el.dataset.height = n.height;
    }
    return true;
  }

  function refresh() {
    const mutations = api.changes();
    if (mutations.length === 0) return; // nothing changed: do nothing

    // Content-only? A changed leaf (text, button, or input) with an id, whose
    // position in the tree is unchanged, can be restyled in place. Anything
    // else (a container — restyle cannot update one — or an id-less node)
    // forces a rebuild.
    const inPlace = mutations.every((m) => {
      if (m.kind !== 'changed' || m.node.id === null) return false;
      if (m.node.kind === 'container') return false;
      const prev = retained.find((n) => n.id === m.node.id);
      return prev !== undefined && prev.parent === m.node.parent;
    });

    apply(mutations);
    if (inPlace) {
      for (const m of mutations) restyle(m.node);
    } else {
      rebuild();
    }
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

// Open a window running the entry with `name`. The geometry (`x`, `y`, `w`,
// `h`) is the component's request: a 0 dimension is the entry's blob default,
// or the host's (a cascade placement). Returns the host's window handle, or
// null if the entry cannot be run.
function openWindow(name, x, y, w, h) {
  const entry = DEMOS.find((e) => e.name === name);
  const compiled = entry ? modules.get(entry.src) : null;
  if (!entry || !compiled) return null; // not an entry the host can run

  // The entry's `data` blob (host-defined JSON): the window's presentation.
  // A blob that will not parse (or lacks a field) falls back to the name and
  // the host's default size, as the component degrades a style it cannot
  // express.
  let meta = {};
  try {
    meta = JSON.parse(entry.data);
  } catch {
    // the blob will not parse: all fallbacks
  }
  const title = typeof meta.label === 'string' && meta.label ? meta.label : name;
  // Only positive numbers count (the component's parser reads the same way).
  const num = (v) => (typeof v === 'number' && v > 0 ? v : 0);
  const width = w > 0 ? w : num(meta.width) || DEFAULT_W;
  const height = h > 0 ? h : num(meta.height) || DEFAULT_H;

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

  // The title bar: the icon (when the blob supplies one), the title, and the
  // close button.
  const bar = document.createElement('div');
  bar.className = 'titlebar';
  const barChildren = [];
  if (typeof meta.icon === 'string' && meta.icon) {
    const icon = document.createElement('img');
    icon.src = meta.icon;
    icon.alt = '';
    barChildren.push(icon);
  }
  const titleLabel = document.createElement('span');
  titleLabel.className = 'titlebar-title';
  titleLabel.textContent = title;
  const close = document.createElement('button');
  close.className = 'close';
  close.textContent = '×';
  close.setAttribute('aria-label', 'Close ' + title);
  barChildren.push(titleLabel, close);
  bar.append(...barChildren);
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

const desktop = await createApp(WAUI_WASM_BYTES, {
  registry: DEMOS,
  onOpenWindow: (name, x, y, w, h) => openWindow(name, x, y, w, h),
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

// A double-clicked tile opens its entry. The host maps Enter/Space on a
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
