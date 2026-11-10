# WAT components

The three demos in hand-rolled WAT: the same components as
[`crates/demo-counter`](../crates/demo-counter/),
[`crates/demo-stopwatch`](../crates/demo-stopwatch/), and
[`crates/demo-todo`](../crates/demo-todo/), the same
[`waui:ui/app`](../wit/app.wit) world, the same four hosts.

There is no WAT compiler for a higher-level language to lower into, so the
modules are written directly: the WASM GC (structs, arrays, refs) holds the
state, and the canonical ABI is written by hand against the WIT. The WIT
file is the only shared spec.

## Build pipeline

```
wat/<demo>.wat
  │  wasm-tools parse
  ▼
target/wat/<demo>.wasm              (the web host runs this one)
  │  wasm-tools component embed wit/
  ▼
target/wat/<demo>-wat.embedded.wasm
  │  wasm-tools component new
  ▼
target/components/<demo>-wat.component.wasm
```

`make components-wat` runs all of it. Needs `wasm-tools` on the PATH. (wabt
cannot be the tool: its WAT parser predates the final GC proposal and
rejects `struct`/`array` types outright.)

## The modules

One self-contained file per demo — no imports of shared WAT, no build
magic; the scaffolding (the records, the diff, the canonical-ABI glue) is
duplicated per file, as the demos are self-contained demos.

- **State is on the GC heap.** The export is a flat, pre-ordered node list
  (the contract's tree) in a struct array; the todo's list is a struct array
  of todo structs. Labels are byte arrays. Nothing in the state needs linear
  memory, so linear memory holds only what the ABI requires.
- **The memory is fixed at two pages.** Page 0: the result arena (the first
  60 KiB, a bump that resets on every `cabi_post_*`) and the host scratch
  (the top 4 KiB, the `cabi_realloc` region for string parameters). Page 1:
  the string literals. It never grows, so the selftest's leak check passes
  trivially.
- **No `memory.fill`.** Node's V8 (24.x) miscompiles it: in-bounds fills
  trap or silently no-op. The record emitters write every byte a host reads
  instead of zeroing the record first.
- **The parser's gaps.** This wasm-tools WAT parser has no `array.copy` (the
  todo's list splices are element loops) and rejects local initializer
  expressions; a non-nullable ref local set in both branches of a statement
  if fails validation (the todo's add uses if-as-expressions instead).

The hosts are unchanged: the web host runs the pre-lift core module through
`glue.js`; the wasmtime hosts run the lifted component through
`host-shared`.
