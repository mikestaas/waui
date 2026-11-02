// A todo list, in AssemblyScript — the same component as
// `crates/demo-todo`, same `waui:ui/app` world. The structural demo: the
// contract's diff expresses add, remove, and re-parent, and this drives
// them through a real component.
//
// The contract has no text input (the README's roadmap), so the "add"
// button cycles a preset list of labels instead of a text box.
//
//   view:     a column (1): the "todos" header (2), the active column (3)
//             and the done column (4) stacked, and the footer row (5) with
//             the count (6, small), the add button (7, positive), and the
//             clear button (8, negative)
//   on-click: 7 adds the next preset todo; 8 clears the done column; a
//             row's checkbox (101+4i) moves the todo between the columns;
//             its delete (103+4i) removes it
//   changes:  the diff: the new or moved nodes, the count, and the removed
//             ids (the whole tree on first contact)
//   on-tick:  nothing (a todo list has no clock)
//
// **The ids.** A todo created i-th gets its row 100+4i while active and
// 500+4i while done; its checkbox, label, and delete keep 101+4i, 102+4i,
// and 103+4i in both columns. The row's id changes with the column because
// the contract moves a container by removing the subtree and re-adding it
// (a host links a container's children in list order, and a re-parented
// container would leave them behind); the children's ids do not, so a move
// re-parents them — same id, new parent.
//
// **The order.** Both columns are newest first: the host inserts a new or
// moved node just after its parent, the top of the column, so the lists
// keep that order and view() and the retained list agree.
//
// The whole UI is JSX, as in the other demos; the columns' rows come in
// through the `kids` attribute (a dynamic list — the factory takes one
// widget per child, and AssemblyScript has no union types for a
// widget-or-list argument). `../jsx.mjs` lowers it to `h` calls before
// `asc` compiles it (asc's parser has no JSX). `onclick`/`ontick` are
// renamed to the world's hyphenated names after compilation (see
// `../rename-exports.mjs`).

import { cabi_post_changes, cabi_post_view } from '../abi/arena';
import {
  COLOR_ACCENT,
  COLOR_NEUTRAL,
  COLOR_NEGATIVE,
  COLOR_POSITIVE,
  COLOR_WARNING,
  SIZE_LARGE,
  SIZE_MEDIUM,
  SIZE_SMALL,
} from '../abi/node';
import { button, diffOf, h, text, viewOf, Widget } from '../ui/tree';

// The world requires both post functions on the module itself.
export { cabi_post_changes, cabi_post_view };

// Widget ids (the contract's identity convention).
const BOX = 1;
const HEADER = 2;
const ACTIVE_COL = 3;
const DONE_COL = 4;
const FOOTER = 5;
const COUNT = 6;
const ADD = 7;
const CLEAR = 8;

/// One todo. `idx` is its creation index, the source of its node ids (never
/// reused); `label` is a preset; `done` picks its column and its row's style.
class Todo {
  idx: i32 = 0;
  label: string = '';
  done: i32 = 0;
}

/// The labels the "add" button cycles (the contract has no text input).
const PRESETS: string[] = [
  'buy milk',
  'walk the dog',
  'water plants',
  'write code',
  'ship the demo',
  'take a break',
];

// The two lists, newest first (see the header, *The order*). Module
// globals, so the conservative GC keeps their todos for the module's
// lifetime; the row structure is rebuilt local to each export.
var active: Todo[] = new Array<Todo>();
var done: Todo[] = new Array<Todo>();
var inited: i32 = 0;
/// The next creation index; the seeded todos take the first three.
var nextIdx: i32 = 3;
/// The next preset label for the add button; the seed consumed the first
/// three.
var presetIdx: i32 = 3;

/// The count, the only state-dependent leaf outside the rows. A module
/// global, so it survives between exports; derived from the active list on
/// every build.
var count = text('', COLOR_NEUTRAL, SIZE_SMALL).id(COUNT);
count.reserve(16); // "100 left" is 8 bytes
// The footer's buttons are static.
var addBtn = button('add', COLOR_POSITIVE, SIZE_MEDIUM).id(ADD);
var clearBtn = button('clear done', COLOR_NEGATIVE, SIZE_MEDIUM).id(CLEAR);

/// Seed the lists once: the first three presets, the last one done, added
/// in index order (so the newest, 1, sits at the front of the active list).
function seed(): void {
  if (inited != 0) return;
  inited = 1;
  const t0 = new Todo();
  t0.idx = 0;
  t0.label = PRESETS[0];
  const t1 = new Todo();
  t1.idx = 1;
  t1.label = PRESETS[1];
  const t2 = new Todo();
  t2.idx = 2;
  t2.label = PRESETS[2];
  t2.done = 1;
  active.push(t1);
  active.push(t0);
  done.push(t2);
}

/// The count's content, derived from the active list on every build.
function applyCount(n: i32): void {
  count.label(`${n} left`).style(COLOR_NEUTRAL, SIZE_SMALL);
}

/// One todo's row: the checkbox, the label, and the delete button. The row's
/// id changes with the column (100+4i / 500+4i); the children's do not.
function todoRow(t: Todo): Widget {
  const base = t.idx * 4;
  const isDone = t.done != 0;
  return (
    <row id={base + (isDone ? 500 : 100)}>
      <button
        id={101 + base}
        label={isDone ? '✓' : '○'}
        color={isDone ? COLOR_POSITIVE : COLOR_NEUTRAL}
        size={SIZE_MEDIUM}
        reserve={8}
      />
      <text
        id={102 + base}
        label={t.label}
        color={isDone ? COLOR_WARNING : COLOR_NEUTRAL}
        size={SIZE_MEDIUM}
        reserve={16}
      />
      <button id={103 + base} label="✕" color={COLOR_NEGATIVE} size={SIZE_MEDIUM} reserve={8} />
    </row>
  );
}

/// A list's rows, in the list's order (newest first).
function rows(list: Todo[]): Widget[] {
  const out: Widget[] = new Array<Widget>();
  for (let i = 0; i < list.length; i++) out.push(todoRow(list[i]));
  return out;
}

/// `list` with `t` at the front, as a fresh array (the old one is reclaimed;
/// `splice` in this stdlib version deletes but does not insert).
function withFront(list: Todo[], t: Todo): Todo[] {
  const out: Todo[] = new Array<Todo>();
  out.push(t);
  for (let i = 0; i < list.length; i++) out.push(list[i]);
  return out;
}

/// The todo list's whole UI, nested: the structure is rebuilt, the count
/// and the footer's buttons are the persistent leaves. The columns' rows
/// come in through the `kids` attribute (a dynamic list).
function tree(): Widget {
  applyCount(active.length);
  return (
    <column id={BOX}>
      <text id={HEADER} label="todos" color={COLOR_ACCENT} size={SIZE_LARGE} />
      <column id={ACTIVE_COL} kids={rows(active)} />
      <column id={DONE_COL} kids={rows(done)} />
      <row id={FOOTER}>
        {count}
        {addBtn}
        {clearBtn}
      </row>
    </column>
  );
}

export function view(): i32 {
  seed();
  // viewOf records the snapshot (the next diff's reference) and sets
  // everExported.
  return viewOf(tree());
}

export function changes(): i32 {
  seed();
  // diffOf holds the bootstrap rule: the whole tree on first contact,
  // otherwise the changed nodes and the removed ids.
  return diffOf(tree());
}

export function onclick(id: i32): void {
  // → `on-click` after the export rename
  if (id == ADD) {
    // The next preset, newest first.
    const t = new Todo();
    t.idx = nextIdx;
    nextIdx += 1;
    t.label = PRESETS[presetIdx];
    presetIdx = (presetIdx + 1) % PRESETS.length;
    active = withFront(active, t);
    return;
  }
  if (id == CLEAR) {
    done.length = 0;
    return;
  }
  // A row's checkbox (101+4i) or delete (103+4i).
  const m = id % 4;
  let idx: i32;
  let remove: i32;
  if (m == 1) {
    idx = (id - 101) / 4;
    remove = 0;
  } else if (m == 3) {
    idx = (id - 103) / 4;
    remove = 1;
  } else {
    return;
  }
  let pos = -1;
  let inActive = 0;
  for (let i = 0; i < active.length; i++) {
    if (active[i].idx == idx) {
      pos = i;
      inActive = 1;
      break;
    }
  }
  if (pos < 0) {
    for (let i = 0; i < done.length; i++) {
      if (done[i].idx == idx) {
        pos = i;
        inActive = 0;
        break;
      }
    }
  }
  if (pos < 0) return; // a stale click: the todo is gone
  const t = inActive != 0 ? active.splice(pos, 1)[0] : done.splice(pos, 1)[0];
  if (remove != 0) return;
  t.done = inActive != 0 ? 1 : 0;
  // Newest first in the column it joins.
  if (inActive != 0) {
    done = withFront(done, t);
  } else {
    active = withFront(active, t);
  }
}

// → `on-tick` after the export rename. A todo list has no clock.
export function ontick(): void {}
