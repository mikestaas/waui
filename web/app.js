// The DOM host: rebuilds the widget tree from the flat node list (the same
// contract the other hosts use) and renders it as plain elements.
//
// It follows the diff protocol (see `../wit/`): after each click,
// text-input edit, or scheduled wake the host checks the component's
// `changes()`. Content-only mutations (a text, button, or input whose
// position in the tree is unchanged) are applied in place — the existing
// element is updated, no DOM rebuild, and the delegated listeners survive.
// Anything structural re-queries `view()` (the full tree) and rebuilds: the
// diff cannot carry id-less nodes (the WIT identity note), so the snapshot
// is the source of truth; an empty diff does nothing.
//
// The container's tokens render as token classes (always emitted; the
// defaults are no-ops in the stylesheet) plus inline geometry: a
// `positioned` container is absolute at (x, y), stacked by `z`; the numerics
// also ride in the dataset, which the e2e DOM serializer reads back.
//
// The multi-component functions (the windowing side of the `app` world) are
// host duties: this file runs each opened entry in its own sandbox (a
// pre-compiled module, a fresh instance) and seats its tree in the opening
// component's content slot — the container whose id is the host handle. The
// opening component draws the rest (the window's chrome and geometry, its
// own tree); a panel whose slot the component no longer draws gets a
// centered, chrome-less fallback.
//
// Events route to the surface they landed on: the main app, or the panel
// whose slot contains the target (walking up to a `data-id` that is an open
// handle). A press reports `on-pointer-down` (the host raises nothing; the
// component does), a held movement `on-pointer-move`, a release
// `on-pointer-up` — and a release within a few pixels of the press a
// synthesized `on-click`, whose native twin the click listener drops, so one
// press-release is one activation. Two such clicks on the same `cursor:
// pointer` node, close in time and space, synthesize an `on-double-click`
// the same way (the capture set on the press retargets the native
// `dblclick` to the app root, off the node, so the synthesis is the
// delivery; the listener drops its twin). A key (outside a text input) goes
// to the main app.
//
// Time-based refresh is requested by the component: its `schedule` import
// arms (or stops, with 0) the host's wake loop, which calls `on-tick()`
// then `changes()`. Each panel keeps its own wake loop.
//
// This file is concatenated into the generated page by build.mjs, so it
// relies on `createApp`/`instantiateApp` from glue.js, `WAUI_WASM_BYTES`
// from the boot script, and `WAUI_REGISTRY` (the entry registry, empty for
// a plain demo).

// The entry registry, as the host provides it to the component's
// `list-entries` (each `{ name, data }`; `src` is host-internal).
const registry = typeof WAUI_REGISTRY !== 'undefined' ? WAUI_REGISTRY : [];

const rootEl = document.getElementById('app');
const badge = document.getElementById('badge');
// The windowing side fills the viewport (a stretch-both root fills it); the
// page's padding is the standalone presentation.
rootEl.classList.toggle('waui-registry', registry.length > 0);
badge.textContent =
  'waui (web) — renderer: static html, host: browser' + (registry.length > 0 ? ', windowing' : '');
// The menu bar shows the name; the badge would sit on it.
if (registry.length > 0) badge.style.display = 'none';

// The host's wake loop, armed (or stopped) by the component's `schedule`
// request. `arm` may run during `createApp`, before `app` is bound, so it
// only manages the timer; the interval callback does the calling.
let timer = null;
function arm(ms) {
  if (timer) clearInterval(timer);
  timer = ms > 0 ? setInterval(wake, ms) : null;
}
function wake() {
  app.tick();
  refreshMain();
}

// --------------------------------------------------------------------------
// The tree renderer: the retained node list, the diff protocol, and the DOM,
// scoped to one component's tree. The main app and each panel run one.

// One element for one node, its children attached (the pre-order list makes
// a subtree a contiguous run; `childrenOf` groups it).
function elFor(n, childrenOf, elements) {
  let el;
  if (n.kind === 'container') {
    el = document.createElement('div');
    // The direction sets the axis; the align token the cross-axis placement.
    // The other tokens ride as classes, always: the e2e DOM serializer reads
    // them back, and the defaults are no-ops in the stylesheet, so a
    // container with all-default tokens renders as a plain row/column.
    el.className = [
      'waui',
      `waui-${n.direction}`,
      `waui-align-${n.align}`,
      `waui-stretch-${n.stretch}`,
      `waui-overflow-${n.overflow}`,
      `waui-bg-${n.background}`,
      `waui-border-${n.border}`,
      `waui-radius-${n.radius}`,
      `waui-shadow-${n.shadow}`,
      `waui-cursor-${n.cursor}`,
      n.positioned ? 'waui-positioned' : '',
    ]
      .filter(Boolean)
      .join(' ');
    // The geometry, in the dataset: what the token classes do not carry
    // (the numerics), for the e2e DOM serializer.
    el.dataset.x = n.x;
    el.dataset.y = n.y;
    el.dataset.w = n.w;
    el.dataset.h = n.h;
    el.dataset.z = n.z;
    el.dataset.gap = n.gap;
    el.dataset.padding = n.padding;
    if (n.positioned) {
      // Out of flow, at (x, y) relative to its parent (its nearest
      // positioned ancestor; every container is one, so its parent).
      el.style.left = n.x + 'px';
      el.style.top = n.y + 'px';
      if (n.z > 0) el.style.zIndex = n.z;
    }
    if (n.w > 0) el.style.width = n.w + 'px';
    if (n.h > 0) el.style.height = n.h + 'px';
    if (n.gap > 0) el.style.gap = n.gap + 'px';
    if (n.padding > 0) el.style.padding = n.padding + 'px';
    // An explicit dimension wins on the same axis (the iced host's rule):
    // it disables the stretch on that axis (the token class stays — the
    // stylesheet's flex-grow would otherwise grow past the size).
    const mainIsWidth = n.direction === 'row';
    if ((mainIsWidth ? n.w > 0 : n.h > 0) && (n.stretch === 'main' || n.stretch === 'both')) {
      el.style.flexGrow = '0';
    }
  } else if (n.kind === 'button') {
    el = document.createElement('button');
    // The component's semantic style, as token classes (see index.html).
    el.className = `waui waui-button waui-${n.style.color} waui-${n.style.size}`;
    el.textContent = n.label;
    if (n.id !== null) elements.set(n.id, el);
  } else if (n.kind === 'input') {
    el = document.createElement('input');
    // The component's semantic style, as token classes (see index.html).
    el.className = `waui waui-input waui-${n.style.color} waui-${n.style.size}`;
    el.value = n.content;
    // The field's width in characters, as an inline `ch` size; `0` leaves
    // it to the stylesheet's default. The dataset copy is what the e2e
    // DOM serializer reads back.
    el.dataset.width = n.width;
    el.style.width = n.width > 0 ? `${n.width}ch` : '';
    if (n.id !== null) elements.set(n.id, el);
  } else if (n.kind === 'image') {
    el = document.createElement('img');
    // An image has no style token: the `src` and a `waui-image` class.
    // The host resolves `src` (a URL or path); a src it cannot load
    // renders the browser's broken-image placeholder.
    el.className = 'waui waui-image';
    el.src = n.src;
    el.alt = '';
    // Images are natively draggable. A press whose target the host's rebuild
    // replaces can re-resolve the drag source at the press position, onto an
    // icon behind it; a native drag of the icon fires `pointercancel` and
    // drops the host's pointer capture mid-gesture. The host owns the pointer
    // model, so the icon is not a drag source.
    el.draggable = false;
    // The size in the host's units (pixels); `0` leaves it to the
    // stylesheet's default. The dataset copy is what the e2e DOM
    // serializer reads back.
    el.dataset.width = n.width;
    el.dataset.height = n.height;
    el.style.width = n.width > 0 ? `${n.width}px` : '';
    el.style.height = n.height > 0 ? `${n.height}px` : '';
    if (n.id !== null) elements.set(n.id, el);
  } else {
    el = document.createElement('div');
    el.className = `waui waui-text waui-${n.style.color} waui-${n.style.size}`;
    el.textContent = n.content;
    if (n.id !== null) elements.set(n.id, el);
  }
  // Nodes with an id carry it in the DOM: it keys in-place updates (and the
  // event delegation). Id-less nodes are unaddressable — see the WIT
  // identity note.
  if (n.id !== null) el.dataset.id = n.id;
  for (const child of childrenOf.get(n.id) ?? []) {
    el.appendChild(elFor(child, childrenOf, elements));
  }
  return el;
}

// A renderer for one component's tree, mounted into `mount()` (a function,
// so a panel's — the opening component's content slot, recreated by its
// rebuilds — is resolved at render time, not captured).
function makeTreeRenderer(api, mount) {
  // The retained node list, as the diff protocol maintains it: a full
  // The retained node list: a snapshot from `view()`, re-queried whenever
  // the component reports a structural change (the diff cannot carry
  // id-less nodes, so the snapshot is the source of truth), and kept
  // current in place by content-only updates.
  let retained = api.view();
  // Widget id -> its DOM element, for in-place updates.
  let elements = new Map();
  // The element at the top of this tree (the root, or a host wrapper).
  let top = null;

  // Keep the retained list current through a content-only diff: each changed
  // node replaces its retained copy in place (the caller's in-place check
  // guarantees the position is unchanged).
  function apply(mutations) {
    for (const m of mutations) {
      const i = retained.findIndex((x) => x.id === m.node.id);
      if (i >= 0) retained[i] = m.node;
    }
  }

  // Rebuild the element tree from the retained list. The tree is built
  // pre-order (parents precede children), so children can be attached as
  // they are encountered in order.
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
    const roots = retained
      .filter((n) => n.parent === null)
      .map((n) => elFor(n, childrenOf, elements));
    if (roots.length === 0) {
      top = document.createElement('div');
    } else if (roots.length === 1) {
      top = roots[0];
    } else {
      top = document.createElement('div');
      top.className = 'waui waui-column waui-align-center';
      for (const r of roots) top.appendChild(r);
    }
  }

  // Restyle an element in place from its node. Returns whether the element
  // was found.
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
      el.style.width = n.width > 0 ? `${n.width}px` : '';
      el.style.height = n.height > 0 ? `${n.height}px` : '';
    }
    return true;
  }

  function render() {
    if (!top) rebuild();
    mount().replaceChildren(top);
  }

  // The focused element of this tree, or null: a rebuild detaches it, and
  // focus does not survive a detach, so capture its id and selection before.
  function captureFocus() {
    const el = document.activeElement;
    if (!el || !top || !top.contains(el)) return null;
    return {
      id: el.dataset.id !== undefined ? Number(el.dataset.id) : null,
      selStart: 'selectionStart' in el ? el.selectionStart : null,
      selEnd: 'selectionEnd' in el ? el.selectionEnd : null,
    };
  }

  // Restore a captured focus after a rebuild: the new element with the same
  // id, refocused, with the selection restored. A no-op when the focus never
  // left (an in-place restyle).
  function restoreFocus(f) {
    if (!f || f.id === null) return;
    if (document.activeElement !== document.body) return;
    const el = top?.querySelector(`[data-id="${f.id}"]`);
    if (!el) return;
    el.focus();
    if (f.selStart !== null && 'setSelectionRange' in el) {
      el.setSelectionRange(f.selStart, f.selEnd ?? f.selStart);
    }
  }

  // Ask the component what changed, and update. Returns whether anything did.
  function refresh() {
    const mutations = api.changes();
    if (mutations.length === 0) return false; // nothing changed: do nothing

    // Content-only? A changed leaf (text, button, or input) with an id,
    // whose position in the tree is unchanged, can be restyled in place.
    // Anything else (a container — restyle cannot update one — or an id-less
    // node) forces a rebuild.
    const inPlace = mutations.every((m) => {
      if (m.kind !== 'changed' || m.node.id === null) return false;
      if (m.node.kind === 'container') return false;
      const prev = retained.find((n) => n.id === m.node.id);
      return prev !== undefined && prev.parent === m.node.parent;
    });

    const focus = captureFocus();
    if (inPlace) {
      apply(mutations);
      for (const m of mutations) restyle(m.node);
      // The tree is already mounted: the elements were updated in place, so
      // a re-mount is skipped — `replaceChildren` on an already-attached
      // node detaches it, and focus does not survive a detach.
    } else {
      // Structural: the diff cannot carry id-less nodes (see the WIT
      // identity note) — a new or moved one would be lost — so the retained
      // tree is re-queried from view, which is always the full tree (the
      // WIT's re-query fallback). The rebuilt top is not in the DOM yet, so
      // it is mounted.
      retained = api.view();
      rebuild();
      mount().replaceChildren(top);
    }
    restoreFocus(focus);
    return true;
  }

  return {
    refresh,
    render,
    get top() {
      return top;
    },
  };
}

// --------------------------------------------------------------------------
// The windowing layer: the host's side of the multi-component functions.

const HANDLE_BASE = 10000; // the first handle (above the component's fixed ids)
let nextHandle = HANDLE_BASE;
// An open panel: the entry's component (its own instance, its own wake loop)
// and its tree's renderer. The panel has no geometry of its own: the opening
// component draws the window, and the host seats the panel's tree in the
// component's content slot (the container whose id is the handle).
const panels = new Map(); // handle -> panel

// The entry modules, pre-compiled at startup and keyed by name: `open-entry`
// is a sync wasm call, so the instantiation must be sync too (a fresh
// instance on the pre-compiled module). An entry that will not compile is
// unavailable: `open-entry` reports `not-found`, and the component degrades.
const modules = new Map();
async function preloadEntries() {
  await Promise.all(
    registry.map(async (entry) => {
      try {
        const res = await fetch(entry.src);
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        const bytes = new Uint8Array(await res.arrayBuffer());
        modules.set(entry.name, await WebAssembly.compile(bytes));
      } catch {
        // unavailable: open-entry reports not-found
      }
    }),
  );
}

// The content slot of `handle` in the main tree (the container the opening
// component drew for it), or null when it no longer draws one.
function slotEl(handle) {
  return rootEl.querySelector(`[data-id="${handle}"]`);
}

// Seat a panel's tree in its slot (or its fallback when the slot is gone).
// The existing element is moved, not re-rendered: a rebuild of the panel's
// own tree is the panel renderer's to handle (and it keeps the focus). A
// re-mount of an already-seated tree is skipped — `replaceChildren` on an
// already-attached node detaches it, and focus does not survive a detach.
function seat(handle) {
  const p = panels.get(handle);
  if (!p) return;
  const mount = slotEl(handle) ?? p.fallback;
  if (!p.renderer.top) {
    p.renderer.render();
  } else if (p.renderer.top.parentElement !== mount) {
    mount.replaceChildren(p.renderer.top);
  }
}

// Reseat every panel (after a main rebuild, which recreates the slots).
function reseatAll() {
  for (const handle of [...panels.keys()]) seat(handle);
}

// The focused element, when it is in a panel's tree: a main rebuild detaches
// the panel's tree, and focus does not survive a detach, so capture its
// handle, id, and selection for the restore after the reseat.
function capturePanelFocus() {
  const el = document.activeElement;
  if (!el || el === document.body) return null;
  const surface = surfaceOf(el);
  if (surface.kind !== 'panel') return null;
  return {
    handle: surface.handle,
    id: el.dataset.id !== undefined ? Number(el.dataset.id) : null,
    selStart: 'selectionStart' in el ? el.selectionStart : null,
    selEnd: 'selectionEnd' in el ? el.selectionEnd : null,
  };
}

// Restore a captured panel focus after a reseat: the reattached element with
// the same id, refocused, with the selection restored.
function restorePanelFocus(f) {
  if (!f || f.id === null) return;
  if (document.activeElement !== document.body) return; // the focus never left
  const p = panels.get(f.handle);
  if (!p) return;
  const el = (slotEl(f.handle) ?? p.fallback)?.querySelector(`[data-id="${f.id}"]`);
  if (!el) return;
  el.focus();
  if (f.selStart !== null && 'setSelectionRange' in el) {
    el.setSelectionRange(f.selStart, f.selEnd ?? f.selStart);
  }
}

// Open a window running the entry with `name` (the component's `open-entry`
// request). Returns the host's handle, or null if the entry cannot be run.
function openWindow(name, x, y, w, h) {
  const compiled = modules.get(name);
  if (!compiled) return null; // not an entry the host can run: not-found
  let handle;
  let timer = null;
  const api = instantiateApp(compiled, {
    // The entry's time-based refresh, on a per-panel wake loop.
    onSchedule: (ms) => {
      if (timer) clearInterval(timer);
      timer =
        ms > 0
          ? setInterval(() => {
              api.tick();
              refreshPanel(handle);
            }, ms)
          : null;
    },
  });
  handle = nextHandle++;
  // A centered, chrome-less home for the panel's tree when the component no
  // longer draws its slot. It carries the slot's id, so events inside it
  // still route to the panel.
  const fallback = document.createElement('div');
  fallback.className = 'waui-fallback';
  fallback.dataset.id = handle;
  document.body.append(fallback);
  panels.set(handle, {
    api,
    fallback,
    renderer: makeTreeRenderer(api, () => slotEl(handle) ?? fallback),
    stop: () => {
      if (timer) clearInterval(timer);
    },
  });
  return handle;
}

// Close the window with `handle` (the component's `close-entry` request):
// drop the sandbox. Idempotent, per the WIT.
function closeWindow(handle) {
  const p = panels.get(handle);
  if (!p) return; // an unknown or already-closed handle
  p.stop();
  panels.delete(handle);
  if (p.renderer.top) p.renderer.top.remove(); // detach it from its seat
  p.fallback.remove();
}

function refreshPanel(handle) {
  const p = panels.get(handle);
  if (!p) return;
  if (p.renderer.refresh()) seat(handle);
}

// --------------------------------------------------------------------------
// The main app.

// Boot: pre-compile the entries, then run the component. The pre-compile is
// awaited so an `open-entry` always finds a ready module.
await preloadEntries();

const app = await createApp(WAUI_WASM_BYTES, {
  onSchedule: arm,
  // The viewport, in the host's units (pixels): a function, so a resize is
  // seen (the component clamps a window drag to it; without it the host
  // reports (0, 0) = unknown).
  viewport: () => [window.innerWidth, window.innerHeight],
  registry,
  onOpenWindow: (name, x, y, w, h) => openWindow(name, x, y, w, h),
  onCloseWindow: (handle) => closeWindow(handle),
});

const main = makeTreeRenderer(app, () => rootEl);

function refreshMain() {
  const focus = capturePanelFocus();
  if (main.refresh()) {
    reseatAll(); // a rebuild recreates the slots
    restorePanelFocus(focus);
  }
}

// --------------------------------------------------------------------------
// The events: routed to the surface they landed on.

// The surface an event landed on: the main app, or the panel whose slot
// contains the target (walking up to a `data-id` that is an open handle).
// The main app's and a panel's id spaces overlap, so a node id alone does
// not name its target.
//
// One exception: the slot itself. It is a main-tree node whose id is the
// handle, so a press *on* it is a press on the main app's content area (the
// component raises its window); only a press on a node *inside* it is the
// panel's. A press on the fallback (a host artifact in the body, outside the
// app) is the panel's.
function surfaceOf(el) {
  for (let n = el; n; n = n.parentElement) {
    if (n.dataset && n.dataset.id !== undefined && panels.has(Number(n.dataset.id))) {
      if (n === el) {
        return rootEl.contains(n)
          ? { kind: 'main' }
          : { kind: 'panel', handle: Number(n.dataset.id) };
      }
      return { kind: 'panel', handle: Number(n.dataset.id) };
    }
  }
  return { kind: 'main' };
}

function apiOf(surface) {
  return surface.kind === 'main' ? app : panels.get(surface.handle)?.api;
}

function refresh(surface) {
  if (surface.kind === 'main') {
    refreshMain();
  } else {
    refreshPanel(surface.handle);
  }
}

// The press in progress: its surface, its node, and its position (a release
// within a few pixels of it is a click; the held movement drives
// `on-pointer-move`).
let press = null;
// The last synthesized click, to drop its native twin.
let lastSynth = null;
// The previous synthesized click, for the double-click synthesis (a second
// click on the same node, close in time and space, is a double-activation).
let lastClick = null;
// The last synthesized double-click, to drop its native twin.
let lastSynthDbl = null;

// The delegation is on the document, not the app: a panel in its fallback
// (outside the app) is still its panel's, and a press on the page's
// background resolves to no node (a no-op).
document.addEventListener('pointerdown', (e) => {
  if (e.button !== 0) return;
  const el = e.target.closest('[data-id]');
  if (!el) return; // a press on an unaddressable node has no id to report
  const surface = surfaceOf(el);
  const api = apiOf(surface);
  if (!api) return;
  press = { surface, id: Number(el.dataset.id), el, x: e.clientX, y: e.clientY };
  api.pointerDown(press.id, e.clientX, e.clientY);
  // The capture keeps the moves flowing here (and on, so the delegation
  // sees them) even over other windows or outside the app; without it, a
  // drag that leaves the pressed element would stop reporting. A synthetic
  // (untrusted) press has no active pointer to capture; the throw would
  // kill the rest of the handler, so skip it.
  if (e.isTrusted) rootEl.setPointerCapture(e.pointerId);
  refresh(surface);
});

document.addEventListener('pointermove', (e) => {
  if (!press) return; // an idle pointer never wakes the component (the WIT)
  apiOf(press.surface)?.pointerMove(e.clientX, e.clientY);
  refresh(press.surface);
});

document.addEventListener('pointerup', (e) => {
  if (!press) return;
  const { surface, id, el, x, y } = press;
  press = null;
  const api = apiOf(surface);
  if (!api) return;
  api.pointerUp(e.clientX, e.clientY);
  // A release within a few pixels of the press is a click: the WIT lets a
  // pointer host synthesize one from its input model.
  if (Math.hypot(e.clientX - x, e.clientY - y) <= 4) {
    api.click(id);
    lastSynth = { x: e.clientX, y: e.clientY, t: Date.now() };
    // A second click on the same node, close in time and space, is a
    // double-activation: the WIT lets a pointer host synthesize one from
    // its input model the same way. The synthesis is the delivery, not a
    // duplicate: the capture set on the press retargets the native
    // `dblclick` to the app root, off the node, so without it a
    // double-activation never reaches the component. Main surface only —
    // a panel's tree is its own component's to drive.
    if (
      surface.kind === 'main' &&
      el?.classList.contains('waui-cursor-pointer') &&
      lastClick &&
      lastClick.id === id &&
      Date.now() - lastClick.t < 500 &&
      Math.hypot(e.clientX - lastClick.x, e.clientY - lastClick.y) <= 8
    ) {
      app.doubleClick(id);
      lastSynthDbl = { x: e.clientX, y: e.clientY, t: Date.now() };
    }
    lastClick = { id, x: e.clientX, y: e.clientY, t: Date.now() };
  }
  refresh(surface);
});

// A click: the DOM's native one (a button, or a keyboard activation). The
// host's synthesized click (a press-release on a non-button) already
// activated the node, so its native twin — the browser's click for the same
// gesture — is dropped, keeping one press-release one activation.
document.addEventListener('click', (e) => {
  if (
    lastSynth &&
    Date.now() - lastSynth.t < 500 &&
    Math.hypot(e.clientX - lastSynth.x, e.clientY - lastSynth.y) <= 8
  ) {
    lastSynth = null;
    return;
  }
  const btn = e.target.closest('button[data-id]');
  if (!btn) return;
  const surface = surfaceOf(btn);
  const api = apiOf(surface);
  if (!api) return;
  api.click(Number(btn.dataset.id));
  refresh(surface);
});

// A double-activation of a `cursor: pointer` node (a tile). The host's
// synthesized double-click (from its pointer model, on the release) already
// activated the node, so the native twin for the same gesture — which the
// press's pointer capture retargets to the app root — is dropped, keeping
// one double-activation one.
document.addEventListener('dblclick', (e) => {
  if (
    lastSynthDbl &&
    Date.now() - lastSynthDbl.t < 500 &&
    Math.hypot(e.clientX - lastSynthDbl.x, e.clientY - lastSynthDbl.y) <= 8
  ) {
    lastSynthDbl = null;
    return;
  }
  const el = e.target.closest('[data-id].waui-cursor-pointer');
  if (!el) return;
  if (surfaceOf(el).kind !== 'main') return;
  app.doubleClick(Number(el.dataset.id));
  refreshMain();
});

// The `input` event bubbles, so one delegated listener covers every field —
// including a panel's, which routes to the panel. The component's answer for
// the buffer lands through the diff, like any other change.
document.addEventListener('input', (e) => {
  const field = e.target.closest('input[data-id]');
  if (!field) return;
  const surface = surfaceOf(field);
  const api = apiOf(surface);
  if (!api) return;
  api.text(Number(field.dataset.id), field.value);
  refresh(surface);
});

// A key: the host's name for it. Keys typed into a text input go to
// `on-text` (the input event), not here. The contract's windowing keys are
// `"Escape"`, `"Tab"`, and `"Shift+Tab"` (the host's convention).
addEventListener('keydown', (e) => {
  if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;
  const name = e.key === 'Tab' && e.shiftKey ? 'Shift+Tab' : e.key;
  app.key(name);
  refreshMain();
});

// The initial render (and the seat of any panel — there are none at boot).
main.render();
reseatAll();
