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
  │  asc  (stub runtime, -O; arena in page 0, static data from page 1)
  ▼
target/assembly/<demo>.raw.wasm          (exports onclick / ontick)
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

- **`asc`** compiles the `.ts` sources. Two toolchain facts drive the flags:
  asc 0.28 hardcodes `.ts` as its source extension (a `.as` entry fails with
  a baffling `file.as.ts not found`), and `--runtime stub` strips the AS
  runtime entirely — no allocator, no GC, no `env.abort` import. What the
  module *does* need is `--memoryBase 65536 --initialMemory 2`: static data
  (string literals, the app's state globals) starts at page 1, leaving page 0
  for the arena below.
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
crate. The demos are state and event handling plus a tree written once
through a small scope-based API; updates afterwards address nodes by their
slot handle:

```ts
beginColumn();            // open a scope — the indentation is the tree
id(1);
beginButton();
id(2);
label('+');               // reserve + write; reserve once for the longest
style(C_POS, SIZE_MEDIUM);// you will show, update in place
...
end();                    // close the scope
```

The API is `begin*`/`end` for structure, `id`/`color`/`size`/`style`/
`label`/`reserve` at build time, `set*`/`append*`/`labelReset`/`mark`/
`setColor`/`setSize` at runtime, and `view()`/`changes()` for the projection.
Deliberately no lambdas and no overloads: both pull runtime allocation this
toolchain does not have.

**One deliberate difference from the Rust layer, forced by the toolchain:
the tree is not a nested *value*.** `--runtime stub` strips the AS runtime —
no heap, no allocator — so there is nothing to nest with. The tree is a
table of fixed-size *slots* (10 words each: kind, id, parent, color, size,
direction, label pointer/length/capacity) in the state region, built once
through the scopes and addressed by slot handle afterwards. Same contract,
same output, a per-language ergonomics shape: the languages have different
memory models, so the *form* of the tree differs even where the *role* is
the same.

**Page 0 is split in two.** Both halves are bump regions; neither needs a
runtime:

```
[0 ......... RESULT_END = 32 KiB)   the result arena — reset by cabi_post_*
[RESULT_END ... 64 KiB)             the state region — slot table + label pool,
                                    built once, never freed
```

A projected node points its string field into the label pool — a region the
host can read anyway (the web glue `subarray`s at the pointer; the Rust hosts
read store memory) — while the result buffer stays a self-contained record
run, so one `arenaReset()` is still the entire free and the selftest's leak
check still holds. The tree's footprint is static: at most 32 slots
(1.25 KiB) plus the label capacity the author reserved — trivial against the
32 KiB halves.

## The ABI layer (`abi/`)

Under the tree, the hand-rolled canonical ABI against the WIT:

| file            | what it is |
| --------------- | ---------- |
| `arena.ts`      | the bump arena (page 0) + the world's `cabi_post_*` functions, which reset it — one result's buffers are the only live allocations, so a pointer-reset *is* the free: zero net growth across repeated `view()`/`changes()` calls, no GC, deterministic |
| `utf8.ts`       | byte-oriented emitters (`writeUtf8` incl. surrogate pairs, `writeInt32`/`writeInt64`) that write straight into a buffer — AssemblyScript strings are UTF-16, the ABI's are UTF-8 bytes; an opt-in `end` bound stops at a whole character / whole number |
| `node.ts`       | emitters for the ABI encodings of the `widget` types: the 32-byte `node` record, the 36-byte `mutation` record, and the `{ptr, len}` list descriptor (`makeList`) |
| `root.ts`       | the world's import side: `now` as a raw `@external('$root', 'now')` |

The tree layer projects its slots through these same emitters, so the demos'
output is byte-identical to what they emitted by hand before the layer
existed — the selftest is the proof — and the post functions are still the
only free.

## Things that will bite you

- **Precedence, twice.** AssemblyScript (like JavaScript) binds `!=` tighter
  than `&`, so `d & BIT != 0` silently tests the *constant* side — the
  stopwatch's bit tests are parenthesized on purpose. And it binds `<`
  tighter than `?:`, so `i < flag != 0 ? 36 : 32` parses as
  `(i < flag) != 0 ? 36 : 32`: a ternary *as* the loop condition, which is
  never zero, so the loop never exits. That is how the zero-clear in
  `ui/tree.ts` runs off the end of memory when unparenthesized (it now
  computes the bound first); the same shape that once escaped the selftest
  on a brand-new file.
- **Index access into a typed array** (read *or* write) under
  `--runtime stub --noAssert` makes asc pull the AS runtime back in as an
  `env.abort` import, and the lift then fails. Everything in these layers is
  raw `store`/`load` at computed offsets for that reason — including the
  tree, which is a slot table rather than an `Array`.
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
- The state region is built once and never freed; label updates overwrite
  their reserved buffer in place, and a buffer shorter than the new content
  clamps. Size the reservation for the *longest* string you will show.
- The result arena is capped at `RESULT_END` (a soft backstop: an over-big
  allocation clamps instead of walking into the state region), and the label
  pool is capped at the 64 KiB page edge; the selftest's leak check is the
  other backstop.
