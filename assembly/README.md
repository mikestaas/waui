# AssemblyScript components

The two demos, rewritten in [AssemblyScript](https://www.assemblyscript.org/) —
the same components as [`crates/demo-counter`](../crates/demo-counter/) and
[`crates/demo-stopwatch`](../crates/demo-stopwatch/), speaking the same
[`waui:ui/app`](../wit/app.wit) world through the same four hosts.

They exist to prove the one direction the Rust demos can't: that the
contract is language-agnostic. No `wit-bindgen` exists for AssemblyScript, so
there are **no generated bindings here at all** — the canonical ABI is written
by hand against the WIT, and the WIT is the only shared spec. The claim,
verified by the hosts rather than asserted: a different compiler, a different
toolchain, the same component.

## The build pipeline

```
assembly/<demo>/index.ts
  │  asc  (incremental runtime, -O --noAssert; static data from page 1)
  ▼
target/assembly/<demo>.raw.wasm          (exports onclick / ontick; imports env.abort)
  │  patch-env-abort.mjs  (env.abort → an in-module definition, same index)
  ▼
target/assembly/<demo>.core.wasm
  │  rename-exports.mjs  (onclick → on-click, ontick → on-tick)
  ▼
target/assembly/<demo>-as.wasm           (the web host consumes this one)
  │  wasm-tools component embed wit/
  ▼
target/assembly/<demo>-as.embedded.wasm
  │  wasm-tools component new
  ▼
target/components/<demo>-as.component.wasm
```

Each step exists for a specific reason:

- **`asc`** compiles the `.ts` sources. Two toolchain facts drive the flags.
  First, asc 0.28 hardcodes `.ts` as its source extension (a `.as` entry
  fails with a baffling `file.as.ts not found`). Second, the runtime: the
  tree is a real nested *object* tree (see below), and objects need a heap —
  which in 0.28.20 only `--runtime incremental` provides. The other documented
  names do not resolve against the installed package (`static` and `full`
  fail with "runtime not found"; `stub` strips the runtime entirely), so
  *incremental is a necessity, not a choice*: an in-module heap (TLSF
  allocator + conservative GC, all static within the module) in exchange for
  a single `env.abort` import — its OOM path. The module still needs
  `--memoryBase 65536 --initialMemory 2`: static data (string literals, the
  app's state globals) and the runtime heap start at page 1, leaving page 0
  for the arena and the label pool below.
- **`patch-env-abort.mjs`** removes that one import so the lift stays
  import-free. A component's *world* imports are part of its lifted type, so
  an `env.abort` import would leave a stray, unsatisfiable import in the
  component; and the WIT — the only shared spec — must not be bent to
  accommodate a runtime detail. The patch converts the import into an
  *in-module definition at the same function index* (body: `unreachable`)
  and renumbers every later function reference to keep the indices stable —
  removing an import outright would renumber all later functions and
  silently corrupt the `call` sites, which validation cannot catch. It
  round-trips through WAT (`wasm2wat`/`wat2wasm`, from
  [wabt](https://github.com/WebAssembly/wabt)), re-inserting the converted
  definition first (WAT requires imports before definitions), asserting the
  computed old→new index map is total and injective, and `WebAssembly.validate`-ing
  the result. The behavior is identical — both trap on OOM — and the web
  host needs nothing either way. After the patch, the counter is fully
  import-free; the stopwatch keeps only the world's own `$root.now`.
  Wabt is a hard build dependency, the same class of system tool as
  `wasm-tools`.
- **`rename-exports.mjs`** rewrites two export names. The world spells them
  `on-click` and `on-tick`; WIT uses those names verbatim as raw wasm export
  names; and AssemblyScript identifiers cannot contain hyphens. A 60-line,
  dependency-free export-section rewriter bridges that one gap.
- **`component embed wit/`** is the explicit stand-in for `wit-bindgen`: it
  embeds the WIT world as component-type metadata into the hand-rolled core
  module, and **`component new`** reads that metadata to type the lift. Rust
  gets this for free from `generate!`; a language without a bindings
  generator must say, out loud, which world it implements. The WIT file is
  the spec, machine-checked at lift time.
- **The hosts** — iced, the TUI, the CLI, and the web page — need no changes
  whatsoever. The web host runs the renamed *core* module (browsers
  instantiate core wasm, not components) through the same unmodified
  `glue.js`; the wasmtime hosts run the lifted component through the same
  unmodified `host-shared`.

`make components-as` runs all of it; `make web-test-as` and the `*_as`
round-trip tests run the same checks the Rust components pass.

## The ergonomics layer (`ui/tree`)

Where the Rust demos share `component-ui` (a nested `Tree` that lowers to the
flat list), the AS demos share `ui/tree.ts` — the AssemblyScript twin of that
crate, and structurally the *same thing*: nodes are `Widget` **objects on the
runtime heap**, nested through their `children` arrays, exactly the shape of
the Rust `Tree::column(vec![...])`. There is no slot table, so there is no
slot cap — a tree's size is limited by the heap, not by an arbitrary 32. The
demos author it the way the Rust demos do:

```ts
/// The counter's whole UI, nested — the same expression the Rust demo writes.
function tree(v: i32): Widget {
  return column([
    button('+', COLOR_POSITIVE, SIZE_MEDIUM).id(INC),
    readout(v),
    button('-', COLOR_NEGATIVE, SIZE_MEDIUM).id(DEC),
  ]).id(BOX);
}
```

The API: constructors `text(content, color, size)` / `button(label, color,
size)` / `row(children)` / `column(children)`; chainable `id()` / `label()`
/ `reserve()` / `style()` for the state-dependent bits; a plain field
`children`. The id is set with the `.id(n)` builder — the twin of the Rust
crate's, which returns `this` so a whole tree can be one nested expression.
(AssemblyScript cannot name a field and a method alike the way Rust can, so
the builder's storage field wears an underscore; address the id through
`id()`. And as before: no lambdas, no overloads — the layer stays a handful
of functions.)

One behavior of the Rust model carries over exactly: **the tree is rebuilt
from state on every export** — `view()` and `changes()` each call `tree(value)`,
and state is the source of truth. The `Widget` objects are local to that
call: the conservative GC reclaims them when the export returns, and the
selftest's leak check (a few hundred rebuilds, zero page growth) is the
proof that nothing lingers.

**Diffing is the demo's bookkeeping, not the layer's** — as in the Rust
demos. The layer provides only the tree, its pool, and the projection:
`viewOf(root)` (the whole tree), `mutationsOf(root, parent)` (one subtree,
as `changed` mutations with an explicit parent), `mutationsOfAll(roots,
parent)` (the dirty set), and `count`. The counter keeps a single `dirty`
flag; the stopwatch keeps the Rust demo's `AtomicU8` bit mask (bit 0 the
readout, bit 1 the button). `changes()` is the bootstrap (the whole tree, on
first contact) and then the re-projection of exactly the dirty subtrees.

### Labels, and the one place the heap must not rule

A projected record carries a *pointer* into a node's label bytes, and the
host keeps that copy after the result is posted — so the bytes must outlive
the tree that pointed at them, and the GC reclaims that tree on the way out.
The label bytes therefore live in the **label pool**: raw linear memory the
GC never scans, one partition per stable node id. `label()` only stores the
UTF-16 source; the pool write happens at projection time, keyed by the node's
*final* id — authoring order is free, exactly as in Rust, and there is no
id-before-label footgun. A partition is allocated at the id's first sight
(honoring `reserve`), and never moves or grows: later content overwrites it
in place and clamps to fit, so `reserve` for the longest content you will
show. A labeled node *without* an id gets a fresh partition per build — a
changing label that never carries a stable id would grow the pool until it
is exhausted, which then renders an empty label rather than corrupting
anything.

## The memory layout

Page 0 keeps the two regions the projection needs, and the heap takes the
rest:

```
[0 ......... RESULT_END = 32 KiB)   the result arena — reset by cabi_post_*
[RESULT_END ... 64 KiB)             the label pool — one partition per stable id
[64 KiB ...)                        static data + the runtime heap, from page 1 up
```

A projected node points its string field into the pool — a region the host
can read anyway (the web glue `subarray`s at the pointer; the Rust hosts
read store memory) — while the result buffer stays a self-contained record
run, so one `arenaReset()` in `cabi_post_*` is still the entire free and the
selftest's leak check still holds. The counter's pool footprint after its
first build is three partitions — 16 + 8 + 8 bytes; the tree's *structure*
(a few hundred bytes of `Widget` objects per rebuild, on the heap) is
invisible to the steady state: the counter idles at two pages, before and
after a few hundred rebuilds.

## The ABI layer (`abi/`)

Under the tree, the hand-rolled canonical ABI against the WIT:

| file            | what it is |
| --------------- | ---------- |
| `arena.ts`      | the bump arena (page 0) + the world's `cabi_post_*` functions, which reset it — one result's buffers are the only live allocations, so a pointer-reset *is* the free: zero net growth across repeated `view()`/`changes()` calls, no GC, deterministic |
| `utf8.ts`       | byte-oriented emitters (`writeUtf8` incl. surrogate pairs, `writeInt32`/`writeInt64`) that write straight into a buffer — AssemblyScript strings are UTF-16, the ABI's are UTF-8 bytes; an opt-in `end` bound stops at a whole character / whole number |
| `node.ts`       | emitters for the ABI encodings of the `widget` types: the 32-byte `node` record, the 36-byte `mutation` record, and the `{ptr, len}` list descriptor (`makeList`) |
| `root.ts`       | the world's import side: `now` as a raw `@external('$root', 'now')` |

The tree layer projects its objects through these same emitters, so the
demos' output is byte-identical to the Rust demos' — the selftest is the
proof — and the post functions are still the only free.

## Things that will bite you

- **`Map.get` throws on an absent key.** Unlike JavaScript, the
  AssemblyScript stdlib cannot represent `undefined`, so `Map.get` on a
  missing key *throws* — and a throw in wasm is an `abort` trap. (It is
  `has(key) ? get(key) : …`, always: `has` is a pure bool.) The pool
  registry's lookup does exactly that; a JS-muscle-memory `get` is a
  guaranteed trap on the first build.
- **Conservative GC keeps anything a module global references.** This is
  the pool retention rule, working as intended: the id→partition registry is
  a module-level `Map`, so every partition it ever allocated lives for the
  module's lifetime, and a partition never moves. It is also the trap: a
  `Widget` parked in a module global would pin its whole subtree forever.
  The demos keep trees local to the call that built them.
- **`incremental` is the only runtime with a heap in asc 0.28.20** (see the
  pipeline), and its one `env.abort` import costs a build step — the patch.
  If a future asc ships `full`/`static` properly, the flag can move; the
  patch stays until the OOM path stops importing.
- **Precedence, twice.** AssemblyScript (like JavaScript) binds `!=` tighter
  than `&`, so `d & BIT != 0` silently tests the *constant* side — the
  stopwatch's bit tests are parenthesized on purpose. And it binds `<`
  tighter than `?:`, so an unparenthesized `… != 0 ? 36 : 32` parses as a
  ternary *as* the condition. `lower` therefore computes its stride and its
  bound into scalars *first* — the same shape that, once, ran a zero-clear
  off the end of memory.
- **No `undefined`, no flow narrowing.** Use `null`; and a nullable variable
  assigned inside an `if` is still nullable after it, so branches end in
  plain scalars (the `materializeLabel` pattern) rather than trusting a
  narrowed type.
- **`static` is a class-field keyword** in 0.28 at module scope; module
  globals are `var`.
- **`var x = <non-literal const expression>`** compiles to a zeroed `mut`
  global plus a start-section init thunk (and, in `-O0` builds, a funcref
  table) — the value is only final *after* instantiation's start runs.
  (Lifted components may have no start of their own, which is why the demos
  build lazily on first export rather than in one.)
- String literals live in the data segment as UTF-16 (from page 1 up, thanks
  to `--memoryBase 65536`); the `utf8.ts` emitters do the UTF-8 conversion
  at write time.
- The pool is allocated and never freed: a partition's first allocation
  sizes it (honoring `reserve`), later content clamps. The result arena is
  capped at `RESULT_END`, the pool at the 64 KiB page edge — an over-big
  request renders an empty label rather than walk into anything — and the
  selftest's leak check is the other backstop.
