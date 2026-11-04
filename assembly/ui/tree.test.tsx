// A standalone check of the tree layer's diff channel — the AS twin of the
// Rust `component-ui` unit tests. Compiled to a core module and run via Node
// (`make ui-test`); `run()` drives `viewOf`/`diffOf` through the structural
// cases (bootstrap, value change, add, remove, re-parent, id-less, a
// spliced list child, an input), decodes the diff from the arena, and
// returns 0 on success (the first failing scenario's index otherwise).
//
// It is import-free after `../patch-env-abort.mjs` (the incremental runtime's
// one import is its OOM path), so Node instantiates it directly.

import { arenaReset } from '../abi/arena';
import {
  COLOR_NEGATIVE,
  COLOR_NEUTRAL,
  COLOR_POSITIVE,
  MUT_CHANGED,
  MUT_REMOVED,
  SIZE_LARGE,
  SIZE_MEDIUM,
  TAG_CONTAINER,
  TAG_INPUT,
} from '../abi/node';
import { readUtf8 } from '../abi/utf8';
import { diffOf, h, viewOf, Widget } from './tree';

// The trees are written in JSX; `../jsx.mjs` lowers them to `h` calls before
// `asc` compiles this module, so the structural cases below exercise the
// factory as well as the diff channel.

// The counter's shape at value v: a box (1) with the "+" button (2), the
// readout (3; its color is state), and the "-" button (4).
function counter(v: i32): Widget {
  return (
    <column id={1}>
      <button id={2} label="+" color={COLOR_POSITIVE} size={SIZE_MEDIUM} />
      <text
        id={3}
        label={`${v}`}
        color={v < 0 ? COLOR_NEGATIVE : COLOR_NEUTRAL}
        size={SIZE_LARGE}
      />
      <button id={4} label="-" color={COLOR_NEGATIVE} size={SIZE_MEDIUM} />
    </column>
  );
}

// The counter without the readout (for the removal case).
function slim(): Widget {
  return (
    <column id={1}>
      <button id={2} label="+" color={COLOR_POSITIVE} size={SIZE_MEDIUM} />
      <button id={4} label="-" color={COLOR_NEGATIVE} size={SIZE_MEDIUM} />
    </column>
  );
}

// The counter with the readout re-parented under a new row (5).
function reparented(): Widget {
  return (
    <column id={1}>
      <button id={2} label="+" color={COLOR_POSITIVE} size={SIZE_MEDIUM} />
      <row id={5}>
        <text id={3} label="1" color={COLOR_NEUTRAL} size={SIZE_LARGE} />
      </row>
      <button id={4} label="-" color={COLOR_NEGATIVE} size={SIZE_MEDIUM} />
    </column>
  );
}

// A box with an id'd readout and an id-less sibling.
function withIdless(label: string): Widget {
  return (
    <column id={1}>
      <text id={3} label="a" color={COLOR_NEUTRAL} size={SIZE_LARGE} />
      <text label={label} color={COLOR_NEUTRAL} size={SIZE_LARGE} />
    </column>
  );
}

// A column whose rows are a child of its own (a dynamic list, the factory's
// spliced-children path).
function spliced(n: i32): Widget {
  const kids: Widget[] = new Array<Widget>();
  for (let i = 0; i < n; i++) {
    kids.push(<text id={10 + i} label={`r${i}`} color={COLOR_NEUTRAL} size={SIZE_LARGE} />);
  }
  return <column id={1}>{kids}</column>;
}

// A column with an input (3) — the fourth node kind, the same record shape
// as text and button.
function withInput(content: string): Widget {
  return (
    <column id={1}>
      <input id={3} label={content} size={SIZE_MEDIUM} />
    </column>
  );
}

// ------------------------------------------------------------------ decoding

/// One decoded mutation: the tag, and (for `changed`) the node's fields and
/// its index.
class Mut {
  tag: i32 = -1; // MUT_CHANGED / MUT_REMOVED
  kind: i32 = -1; // node tag (`changed` only)
  id: i32 = 0;
  parent: i32 = 0;
  color: i32 = -1;
  size: i32 = -1;
  direction: i32 = -1;
  label: string = '';
  index: i32 = -1; // sibling index (`changed` only)
}

/// Decode one 40-byte mutation record at `rec`.
function decodeMut(rec: i32): Mut {
  const m = new Mut();
  m.tag = load<i32>(rec + 0);
  if (m.tag == MUT_REMOVED) {
    m.id = load<i32>(rec + 4);
    return m;
  }
  // A `changed` mutation carries the node one word in (after the tag word).
  const n = rec + 4;
  m.kind = load<i8>(n + 0);
  if (load<i8>(n + 4) != 0) m.id = load<i32>(n + 8);
  if (load<i8>(n + 12) != 0) m.parent = load<i32>(n + 16);
  if (m.kind == TAG_CONTAINER) {
    m.direction = load<i8>(n + 20);
  } else {
    const str = load<i32>(n + 20);
    const len = load<i32>(n + 24);
    m.color = load<i8>(n + 28);
    m.size = load<i8>(n + 29);
    m.label = str != 0 ? readUtf8(str, len) : '';
  }
  // The index sits after the 32-byte node.
  m.index = load<i32>(rec + 36);
  return m;
}

/// Decode the mutations behind a list descriptor at `desc`.
function decodeList(desc: i32): Mut[] {
  const out = new Array<Mut>();
  const ptr = load<i32>(desc + 0);
  const len = load<i32>(desc + 4);
  for (let i = 0; i < len; i++) {
    out.push(decodeMut(ptr + i * 40));
  }
  return out;
}

// ------------------------------------------------------------------ the cases

/// Count the mutations whose tag and id match.
function countTag(muts: Mut[], tag: i32, id: i32): i32 {
  let n = 0;
  for (let i = 0; i < muts.length; i++) {
    if (muts[i].tag == tag && muts[i].id == id) n += 1;
  }
  return n;
}

/// The `changed` mutation with `id`, or null.
function changed(muts: Mut[], id: i32): Mut | null {
  for (let i = 0; i < muts.length; i++) {
    if (muts[i].tag == MUT_CHANGED && muts[i].id == id) return muts[i];
  }
  return null;
}

/// One scenario: project the diff for `tree` (after `viewOf` set the
/// reference where noted), decode it, and check it. `check` returns 0 on
/// success and the scenario's code otherwise. The arena is reset after each,
/// so the scenarios are independent.
function scenario(tree: Widget, check: (muts: Mut[]) => i32): i32 {
  const desc = diffOf(tree);
  const muts = decodeList(desc);
  arenaReset();
  return check(muts);
}

export function run(): i32 {
  // 1. Bootstrap: a host that calls changes() first gets every id'd node.
  {
    const muts = decodeList(diffOf(counter(0)));
    arenaReset();
    if (
      muts.length != 4 ||
      countTag(muts, MUT_CHANGED, 1) != 1 ||
      countTag(muts, MUT_CHANGED, 2) != 1 ||
      countTag(muts, MUT_CHANGED, 3) != 1 ||
      countTag(muts, MUT_CHANGED, 4) != 1
    ) {
      return 1;
    }
  }

  // 2. Unchanged: a rebuilt tree with nothing new reports an empty diff.
  if (scenario(counter(0), (m) => (m.length == 0 ? 0 : 2)) != 0) return 2;

  // 3. Value change: the readout's content and color, one mutation.
  {
    const r = scenario(counter(1), (m) => {
      if (m.length != 1) return 3;
      const c = changed(m, 3);
      // The readout is the box's second child (index 1); in place, so the
      // host ignores the index, but the diff still carries it.
      if (
        c == null ||
        c.label != '1' ||
        c.color != COLOR_NEUTRAL ||
        c.size != SIZE_LARGE ||
        c.index != 1
      )
        return 3;
      return 0;
    });
    if (r != 0) return 3;
  }

  // 4. Removal: the readout is gone from the tree.
  viewOf(counter(1)); // the reference: box, +, readout, -
  arenaReset();
  if (
    scenario(slim(), (m) => (m.length == 1 && m[0].tag == MUT_REMOVED && m[0].id == 3 ? 0 : 4)) != 0
  ) {
    return 4;
  }

  // 5. Addition: the readout re-appears (a new id for the host).
  viewOf(slim()); // the reference: box, +, -
  arenaReset();
  {
    const r = scenario(counter(1), (m) => {
      if (m.length != 1) return 5;
      const c = changed(m, 3);
      // The readout re-appears as the box's second child (index 1).
      return c != null && c.label == '1' && c.index == 1 ? 0 : 5;
    });
    if (r != 0) return 5;
  }

  // 6. Re-parent: the readout moves under a new row; both are reported.
  viewOf(counter(1)); // the reference: box, +, readout, -
  arenaReset();
  {
    const r = scenario(reparented(), (m) => {
      if (m.length != 2) return 6;
      const rowM = changed(m, 5);
      const readM = changed(m, 3);
      // The row is the box's second child (index 1); the readout, re-parented
      // under it, is its only child (index 0).
      if (rowM == null || rowM.kind != TAG_CONTAINER || rowM.parent != 1 || rowM.index != 1)
        return 6;
      if (readM == null || readM.parent != 5 || readM.label != '1' || readM.index != 0) return 6;
      return 0;
    });
    if (r != 0) return 6;
  }

  // 7. Id-less: a modified id-less node is unaddressable and never reported.
  viewOf(withIdless('x')); // the reference: box, readout (3), id-less "x"
  arenaReset();
  if (scenario(withIdless('y'), (m) => (m.length == 0 ? 0 : 7)) != 0) return 7;

  // 8. The spliced list child: the rows lower like their constructor-built
  //    twins, and the diff sees them.
  viewOf(spliced(2)); // the reference: the column (1), the rows (10, 11)
  arenaReset();
  {
    const r = scenario(spliced(3), (m) => {
      if (m.length != 1) return 8;
      const c = changed(m, 12);
      // The third row: index 2 among the column's children.
      return c != null && c.parent == 1 && c.label == 'r2' && c.index == 2 ? 0 : 8;
    });
    if (r != 0) return 8;
  }

  // 9. Input: the fourth node kind; a content change is one mutation.
  viewOf(withInput('')); // the reference: the column (1), the input (3)
  arenaReset();
  {
    const r = scenario(withInput('hi'), (m) => {
      if (m.length != 1) return 9;
      const c = changed(m, 3);
      // The input is the column's only child (index 0).
      return c != null && c.kind == TAG_INPUT && c.label == 'hi' && c.index == 0 ? 0 : 9;
    });
    if (r != 0) return 9;
  }

  return 0;
}
