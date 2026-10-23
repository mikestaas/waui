# AssemblyScript components

The two demos in AssemblyScript: the same components as
[`crates/demo-counter`](../crates/demo-counter/) and
[`crates/demo-stopwatch`](../crates/demo-stopwatch/), the same
[`waui:ui/app`](../wit/app.wit) world, the same four hosts.

There is no `wit-bindgen` for AssemblyScript, so the canonical ABI is
hand-rolled against the WIT (`abi/`), and the lift is typed by an explicit
`wasm-tools component embed wit/` step. The WIT file is the only shared spec.

## Build pipeline

```
assembly/<demo>/index.ts
  │  asc  -O --noAssert --runtime incremental --memoryBase 65536 --initialMemory 2
  ▼
target/assembly/<demo>.raw.wasm        exports onclick/ontick; imports env.abort
  │  node assembly/patch-env-abort.mjs
  ▼
target/assembly/<demo>.core.wasm
  │  node assembly/rename-exports.mjs ... onclick=on-click ontick=on-tick
  ▼
target/assembly/<demo>-as.wasm         (the web host runs this one)
  │  wasm-tools component embed wit/  →  wasm-tools component new
  ▼
target/components/<demo>-as.component.wasm
```

- **`asc`** compiles the sources. In 0.28.20 only two runtime names resolve
  against the installed package: `stub` (no runtime, no heap) and
  `incremental` (in-module TLSF allocator + conservative GC, one import:
  `env.abort`, its OOM path). `static` and `full` fail with "runtime not
  found". The tree needs a heap, so `incremental` is the one that works.
  Also: asc hardcodes `.ts` as the source extension; `--memoryBase 65536`
  moves static data to page 1, leaving page 0 for the arena and pool below.
- **`patch-env-abort.mjs`** turns the `env.abort` import into an in-module
  `unreachable` definition at the same function index, and renumbers every
  later function reference. (Imports and defined functions share one index
  space; deleting the import would renumber everything after it and corrupt
  `call` sites in a way `WebAssembly.validate` cannot catch.) It round-trips
  through WAT (wabt's `wasm2wat`/`wat2wasm`) and checks the result: valid,
  no `env` imports, same export set. Both old and new bodies trap on OOM, so
  behavior is unchanged. After the patch: the counter is import-free; the
  stopwatch keeps only the world's own `$root.now`.
- **`rename-exports.mjs`** rewrites two export names: the world spells them
  `on-click`/`on-tick` (hyphens, which WIT uses verbatim as wasm export
  names), and AssemblyScript identifiers cannot contain hyphens, so the demos
  export `onclick`/`ontick` and are renamed after compilation.
- **`component embed` + `new`** is the stand-in for Rust's `generate!`: the
  WIT world is embedded into the core module as component-type metadata, and
  the lift is typed against it.
- **The hosts** are unchanged: the web host runs the renamed core module
  through `glue.js`; the wasmtime hosts run the lifted component through
  `host-shared`.

`make components-as` runs all of it. Requires Node + `npm install` (asc) and
wabt on the PATH (the patch step).

## The tree layer (`ui/tree`)

Nodes are `Widget` **objects on the incremental runtime heap**, nested through
their `children` arrays — the same shape as the Rust
`Tree::column(vec![...])`. Tree size is limited only by the heap.

The counter's shape: the box and both buttons are static, the readout is the
only state-dependent node, so it is a persistent object.

```ts
var readout = text('', COLOR_NEUTRAL, SIZE_LARGE).id(READOUT);
readout.reserve(16); // "-2147483648" is 11 bytes

/// The readout's label and color, derived from state on every build.
function applyReadout(v: i32): void {
  readout
    .label(`${v}`)
    .style(v < 0 ? COLOR_NEGATIVE : COLOR_NEUTRAL, SIZE_LARGE);
}

function tree(v: i32): Widget {
  applyReadout(v);
  return column([
    button('+', COLOR_POSITIVE, SIZE_MEDIUM).id(INC),
    readout, // the persistent leaf, embedded by reference
    button('-', COLOR_NEGATIVE, SIZE_MEDIUM).id(DEC),
  ]).id(BOX);
}
```

API:

- constructors: `text(content, color, size)`, `button(label, color, size)`,
  `row(children)`, `column(children)`
- chainable: `.id(n)`, `.label(s)`, `.reserve(bytes)`, `.style(color, size)`,
  `.markClean()` (AssemblyScript cannot name a field and a method alike the
  way Rust can, so the id's storage is `_id`; set it with `.id()`)
- plain fields: `children`, `dirty` (the stale mark, set by the modifiers)
- projection: `viewOf(root)`, `diffOf(root)`, `count(w)`

Behavior:

- The demos rebuild the tree's *structure* from state on every export, as the
  Rust demos do; the structural `Widget` objects are local to the call and the
  conservative GC reclaims them when the export returns. The state-dependent
  nodes (the counter's readout; the stopwatch's readout and button) are the
  exception: module globals, kept for the module's lifetime, so their dirty
  marks survive between exports. A few bytes per node, never growing.
- **Labels** live in the static **label pool** (raw linear memory; the GC
  never scans it). The label is kept as UTF-16 source; the UTF-8 pool write
  happens at projection, keyed by the node's id. A partition is
  allocated at the id's first build (sized by `reserve`), never moves or
  grows, and later content overwrites it in place, clamped to fit. Why static:
  a projected record carries a *pointer* into the label bytes and the host
  keeps the result, so the bytes must outlive the tree. A labeled node without
  an id gets a fresh partition per build — a changing label with no stable id
  grows the pool until it is exhausted, which then renders an empty label
  rather than corrupting memory.
- **The diff is a stale mark on the node, set by the modifiers.**
  `label(s)` and `style(color, size)` mark a node stale when the value
  actually changes (identical values do not re-mark), so the demos' mutation
  handlers mutate state and nothing else: the next build re-derives the
  state-dependent nodes, and the marks follow. `changes` calls
  `diffOf(root)`: the stale nodes, in depth-first pre-order, with the parent
  taken from the structure, as `changed` mutations, marks cleared. `viewOf`
  clears the marks too — a full snapshot accounts for everything. A
  component that has never exported its tree reports the whole tree from
  `diffOf`: the WIT's bootstrap rule, held here rather than in each demo.
  The Rust `component-ui` reaches the same result another way (its `Tree` is
  immutable, so it retains the last export and compares; it also expresses
  node add/remove/re-parent, which the mark diff does not); the exported
  byte stream is the same for these demos.

## Memory layout

```
[0 ......... RESULT_END = 32 KiB)   result arena, reset by cabi_post_*
[RESULT_END ... 64 KiB)             the label pool (above)
[64 KiB ...)                        static data + runtime heap, from page 1
```

Every byte of a result comes from the arena, so one pointer reset in
`cabi_post_*` is the whole free. The counter's pool holds 40 bytes after its
first build (16 + 12 + 12); the module stays at its 2 initial pages across
hundreds of rebuilds — the selftest's leak check allows <4 pages of growth
over 120 calls. The demos' persistent state-dependent nodes add a constant
few hundred bytes of retained heap, never growing.

## The ABI layer (`abi/`)

| file        | what it is                                                                                                                                     |
| ----------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| `arena.ts`  | the page-0 bump arena (capped at `RESULT_END`) + the world's `cabi_post_view`/`cabi_post_changes` (both reset it)                              |
| `utf8.ts`   | `writeUtf8` (surrogate pairs; stops at a whole character at the `end` bound) and `writeInt32`/`writeInt64` (a number that would not fit whole is not written) |
| `node.ts`   | emitters for the ABI encodings: the 32-byte `node` record, the 36-byte `mutation` record, the `{ptr,len}` list descriptor (`makeList`)           |
| `root.ts`   | the world's import side: `now` as `@external('$root', 'now')`                                                                                   |

AS strings are UTF-16; the ABI's are UTF-8 bytes, converted at write time.
The demos' output is byte-identical to the Rust demos' — the selftest asserts
it.

## Things that will bite you (asc 0.28.20)

- **`Map.get` on an absent key throws** (the stdlib cannot represent
  `undefined`), and a throw in wasm is an `abort` trap. Always
  `has(key) ? get(key) : …`.
- **The conservative GC keeps anything a module global references** for the
  module's lifetime. That is how the pool registry keeps partitions stable
  across rebuilds, and how the demos' persistent state-dependent nodes keep
  their dirty marks: park only the leaves. A whole tree parked in a global
  would pin its whole subtree forever, so the structure stays local to the
  call.
- **Precedence is JavaScript's**: `!=` binds tighter than `&` (so
  `d & BIT != 0` tests the constant — parenthesize), and `<` binds tighter
  than `?:` (so an unparenthesized `… != 0 ? 36 : 32` parses as a ternary *as*
  the condition, which never terminates a loop). Compute such values into
  locals first, as `lower` does.
- **String `==` compares contents** (unlike JavaScript's reference semantics
  for constructed strings): two independently built strings with the same
  content are equal. The label setter's change detection relies on this.
- **No `undefined`** (use `null`), **no flow narrowing** (a nullable variable
  assigned inside an `if` is still nullable after it; end branches in scalars).
- **`static` is a class-field keyword** at module scope; module globals are
  `var`.
- **`var x = <non-literal expression>`** compiles to a zeroed global plus a
  start-section init thunk; the value is final only after the start runs. The
  demos use it for their persistent nodes; in a component the start runs at
  instantiation, before the host calls anything, so the globals are ready by
  the first export.
- **String literals are UTF-16** in the data segment (page 1 up, via
  `--memoryBase 65536`); the `utf8.ts` emitters convert at write time.
