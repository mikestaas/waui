// The DOM host: rebuilds the widget tree from the flat node list (the same
// contract the other hosts use) and renders it as plain elements.
//
// This file is concatenated into the generated page by build.mjs, so it
// relies on `createApp` from glue.js and `WAUI_WASM_BYTES` from the page.

const app = await createApp(WAUI_WASM_BYTES);

const rootEl = document.getElementById('app');
const badge = document.getElementById('badge');
badge.textContent = 'waui (web) — renderer: static html, host: browser';

// The tree is rebuilt from the flat list on every refresh, mirroring the
// rebuild in the other hosts: parents precede children (pre-order), so
// children can be attached as they are encountered in order.
function rebuild(nodes) {
  const childrenOf = new Map();
  for (const n of nodes) {
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
      el.className = 'waui waui-button';
      el.dataset.id = n.id;
      el.textContent = n.label;
    } else {
      el = document.createElement('div');
      el.className = 'waui waui-text';
      el.textContent = n.content;
    }
    for (const child of childrenOf.get(n.id) ?? []) {
      el.appendChild(elFor(child));
    }
    return el;
  };

  const roots = nodes.filter((n) => n.parent === null).map(elFor);
  if (roots.length === 0) return document.createElement('div');
  if (roots.length === 1) return roots[0];
  const wrap = document.createElement('div');
  wrap.className = 'waui waui-column';
  for (const r of roots) wrap.appendChild(r);
  return wrap;
}

let lastJson = '';
function refresh() {
  const nodes = app.view();
  // Skip the DOM rebuild when the tree is byte-for-byte unchanged (the
  // common case between ticks); the stringify is cheap for a small tree.
  const json = JSON.stringify(nodes);
  if (json === lastJson) return;
  lastJson = json;
  rootEl.replaceChildren(rebuild(nodes));
}

// Event delegation: one listener survives re-renders.
rootEl.addEventListener('click', (e) => {
  const btn = e.target.closest('button[data-id]');
  if (!btn) return;
  app.click(Number(btn.dataset.id));
  refresh();
});

// The host's tick: drive time-based UIs and re-query the view.
setInterval(() => {
  app.tick();
  refresh();
}, 250);

refresh();
