// The DOM host: rebuilds the widget tree from the flat node list (the same
// contract the other hosts use) and renders it as plain elements.
//
// It follows the diff protocol: the initial `view()` is a full snapshot,
// and after each click or tick the host applies the component's
// `changes()` to a retained copy of the node list. Content-only mutations
// (a text or button whose position in the tree is unchanged) are applied
// *in place* — the existing element is restyled, no DOM is rebuilt, and
// the delegated click listener keeps working. Anything structural
// (removals, additions, re-parenting, container changes) falls back to a
// full rebuild from the retained tree. A no-op event reports an empty
// diff, and the host does nothing at all.
//
// This file is concatenated into the generated page by build.mjs, so it
// relies on `createApp` from glue.js and `WAUI_WASM_BYTES` from the page.

const app = await createApp(WAUI_WASM_BYTES);

const rootEl = document.getElementById('app');
const badge = document.getElementById('badge');
badge.textContent = 'waui (web) — renderer: static html, host: browser';

// The retained node list, as the diff protocol maintains it: a full
// snapshot from `view()` at startup, then `changes()` applied by node id.
let retained = app.view();
// Widget id -> its DOM element, for in-place updates.
let elements = new Map();

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
          retained[i] = n; // a changed node keeps its position
        } else {
          // Re-parented: move it to its canonical position so the list
          // stays pre-ordered.
          retained.splice(i, 1);
          insertNode(n);
        }
      } else {
        insertNode(n); // a node the host has never seen
      }
    }
  }
}

// Insert a node the host does not have: just after its parent (past any
// of its own descendants still in the list, so a re-added container lands
// after its children), or at the end if it is a new root.
function insertNode(n) {
  const p = retained.findIndex((x) => x.id === n.parent);
  if (p < 0) {
    retained.push(n);
    return;
  }
  let i = p + 1;
  while (i < retained.length && descendantOf(retained[i], n.id)) i++;
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

function descendantOf(node, ancestor) {
  let cur = node.parent;
  while (cur !== null) {
    if (cur === ancestor) return true;
    const parent = retained.find((n) => n.id === cur);
    cur = parent ? parent.parent : null;
  }
  return false;
}

// Rebuild the DOM from the retained list. The tree is built pre-order
// (parents precede children), so children can be attached as they are
// encountered in order.
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
  const elFor = (n) => {
    let el;
    if (n.kind === 'container') {
      el = document.createElement('div');
      el.className = 'waui waui-' + n.direction;
    } else if (n.kind === 'button') {
      el = document.createElement('button');
      // The component's semantic style, as token classes (see index.html).
      el.className =
        `waui waui-button waui-${n.style.color} waui-${n.style.size}`;
      el.textContent = n.label;
      if (n.id !== null) elements.set(n.id, el);
    } else {
      el = document.createElement('div');
      el.className = `waui waui-text waui-${n.style.color} waui-${n.style.size}`;
      el.textContent = n.content;
      if (n.id !== null) elements.set(n.id, el);
    }
    // Nodes with an id carry it in the DOM: it keys in-place updates (and
    // the click delegation). Id-less nodes are unaddressable — see the
    // WIT identity note.
    if (n.id !== null) el.dataset.id = n.id;
    for (const child of childrenOf.get(n.id) ?? []) {
      el.appendChild(elFor(child));
    }
    return el;
  };

  const roots = retained.filter((n) => n.parent === null).map(elFor);
  let top;
  if (roots.length === 0) {
    top = document.createElement('div');
  } else if (roots.length === 1) {
    top = roots[0];
  } else {
    top = document.createElement('div');
    top.className = 'waui waui-column';
    for (const r of roots) top.appendChild(r);
  }
  rootEl.replaceChildren(top);
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
  }
  return true;
}

// After an event: ask the component what changed, and update.
function refresh() {
  const mutations = app.changes();
  if (mutations.length === 0) return; // nothing changed: do nothing

  // Content-only? A changed text or button with an id, whose position in
  // the tree is unchanged, can be restyled in place. Anything else (that
  // includes id-less nodes) forces a rebuild.
  const inPlace = mutations.every((m) => {
    if (m.kind !== 'changed' || m.node.id === null) return false;
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

// Event delegation: one listener survives re-renders.
rootEl.addEventListener('click', (e) => {
  const btn = e.target.closest('button[data-id]');
  if (!btn) return;
  app.click(Number(btn.dataset.id));
  refresh();
});

// The host's tick: drive time-based UIs, then apply whatever the
// component reports (an empty diff on ticks that changed nothing).
setInterval(() => {
  app.tick();
  refresh();
}, 250);

rebuild();
