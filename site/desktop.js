// The desktop: a grid of demo icons, and the windows they open.
//
// Each window embeds a demo page (make site) in an iframe, so the demo runs
// exactly as it does standalone, and a trap in one window (the Rust todo can
// run out of heap) is contained to it.
//
// This is a classic script, not a module: Chrome blocks file:// module
// *files*, and the launcher must run straight from disk.

const MENUBAR_H = 34;

const DEMOS = [
  {
    name: 'Counter',
    lang: 'Rust',
    page: 'demo-counter.html',
    icon: 'icons/counter-rust.svg',
    w: 300,
    h: 380,
  },
  {
    name: 'Counter',
    lang: 'AssemblyScript',
    page: 'demo-counter-as.html',
    icon: 'icons/counter-as.svg',
    w: 300,
    h: 380,
  },
  {
    name: 'Stopwatch',
    lang: 'Rust',
    page: 'demo-stopwatch.html',
    icon: 'icons/stopwatch-rust.svg',
    w: 300,
    h: 380,
  },
  {
    name: 'Stopwatch',
    lang: 'AssemblyScript',
    page: 'demo-stopwatch-as.html',
    icon: 'icons/stopwatch-as.svg',
    w: 300,
    h: 380,
  },
  {
    name: 'Todo',
    lang: 'Rust',
    page: 'demo-todo.html',
    icon: 'icons/todo-rust.svg',
    w: 360,
    h: 500,
    note: 'note: the Rust web todo can run out of heap after a few adds',
  },
  {
    name: 'Todo',
    lang: 'AssemblyScript',
    page: 'demo-todo-as.html',
    icon: 'icons/todo-as.svg',
    w: 360,
    h: 500,
  },
];

const grid = document.getElementById('grid');
const windows = document.getElementById('windows');
const count = document.getElementById('window-count');

let z = 10;
let cascade = 0;

function updateCount() {
  const n = windows.children.length;
  count.textContent = n + (n === 1 ? ' window' : ' windows');
}

function raise(win) {
  win.style.zIndex = ++z;
  win.dataset.z = z;
}

function closeWindow(win) {
  win.remove();
  updateCount();
}

// Open the demo in a small window. Several windows may be open at once,
// including several of the same demo.
function openWindow(demo) {
  const win = document.createElement('div');
  win.className = 'window';
  win.style.width = demo.w + 'px';
  win.style.height = demo.h + 'px';

  // New windows cascade down-right, then wrap back to the top-left.
  const step = 28;
  const n = cascade++ % 6;
  const left = Math.max(8, Math.min(24 + n * step, innerWidth - demo.w - 8));
  const top = Math.max(MENUBAR_H + 4, Math.min(48 + n * step, innerHeight - demo.h - 8));
  win.style.left = left + 'px';
  win.style.top = top + 'px';

  const bar = document.createElement('div');
  bar.className = 'titlebar';
  const icon = document.createElement('img');
  icon.src = demo.icon;
  icon.alt = '';
  const title = document.createElement('span');
  title.className = 'titlebar-title';
  title.textContent = demo.name + ' — ' + demo.lang;
  const close = document.createElement('button');
  close.className = 'close';
  close.textContent = '×';
  close.setAttribute('aria-label', 'Close ' + title.textContent);
  close.addEventListener('click', (e) => {
    e.stopPropagation();
    closeWindow(win);
  });
  bar.append(icon, title, close);

  win.append(bar);
  if (demo.note) {
    const note = document.createElement('div');
    note.className = 'note';
    note.textContent = demo.note;
    win.append(note);
  }

  // Clicks inside the iframe do not reach this document, so the window is
  // raised when the iframe element itself gains focus.
  const frame = document.createElement('iframe');
  frame.src = demo.page;
  frame.title = title.textContent;
  frame.addEventListener('focus', () => raise(win));
  win.append(frame);

  win.addEventListener('pointerdown', (e) => {
    if (e.target === frame) return; // the focus event covers the iframe
    raise(win);
    startDrag(e, win, bar);
  });

  windows.append(win);
  raise(win);
  updateCount();
}

// Drag by the title bar, clamped so the bar stays on screen and clear of
// the menu bar.
function startDrag(e, win, bar) {
  if (e.button !== 0) return;
  e.preventDefault();
  const rect = win.getBoundingClientRect();
  const dx = e.clientX - rect.left;
  const dy = e.clientY - rect.top;
  bar.classList.add('dragging');
  const move = (ev) => {
    const x = Math.max(8 - rect.width + 64, Math.min(ev.clientX - dx, innerWidth - 64));
    const y = Math.max(MENUBAR_H, Math.min(ev.clientY - dy, innerHeight - bar.offsetHeight));
    win.style.left = x + 'px';
    win.style.top = y + 'px';
  };
  const up = () => {
    removeEventListener('pointermove', move);
    removeEventListener('pointerup', up);
    bar.classList.remove('dragging');
  };
  addEventListener('pointermove', move);
  addEventListener('pointerup', up);
}

// The icon grid: one tile per demo. A click selects; a double-click (or
// Enter/Space) opens a window.
for (const demo of DEMOS) {
  const tile = document.createElement('button');
  tile.className = 'tile';
  tile.type = 'button';
  const img = document.createElement('img');
  img.src = demo.icon;
  img.width = 112;
  img.height = 112;
  img.alt = '';
  const name = document.createElement('span');
  name.className = 'tile-name';
  name.textContent = demo.name;
  const lang = document.createElement('span');
  lang.className = 'tile-lang';
  lang.textContent = demo.lang;
  tile.append(img, name, lang);
  tile.title = demo.note
    ? demo.name + ' (' + demo.lang + ') — ' + demo.note
    : demo.name + ' (' + demo.lang + ') — double-click to open';
  tile.addEventListener('click', () => {
    for (const t of grid.children) t.classList.toggle('selected', t === tile);
  });
  tile.addEventListener('dblclick', () => openWindow(demo));
  tile.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      openWindow(demo);
    }
  });
  grid.append(tile);
}

// Escape closes the frontmost window (when the desktop has focus; keys typed
// inside a window go to its iframe).
addEventListener('keydown', (e) => {
  if (e.key !== 'Escape') return;
  let top = null;
  for (const w of windows.children) {
    if (!top || Number(w.dataset.z) > Number(top.dataset.z)) top = w;
  }
  if (top) closeWindow(top);
});

updateCount();
