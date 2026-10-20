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

## The shared layer (`abi/`)

Where the Rust demos share `component-ui` (a nested `Tree` that lowers to
the flat list), the AS demos share a **canonical-ABI layer** instead:

| file            | what it is |
| --------------- | ---------- |
| `arena.ts`      | the bump arena (page 0) + the world's `cabi_post_*` functions, which reset it — one result's buffers are the only live allocations, so a pointer-reset *is* the free: zero net growth across repeated `view()`/`changes()` calls, no GC, deterministic |
| `utf8.ts`       | byte-oriented emitters (`writeUtf8` incl. surrogate pairs, `writeInt32`/`writeInt64`) that write straight into arena memory — AssemblyScript strings are UTF-16, the ABI's are UTF-8 bytes |
| `node.ts`       | emitters for the ABI encodings of the `widget` types: the 32-byte `node` record, the 36-byte `mutation` record, and the `{ptr, len}` list descriptor (`makeList`) |
| `root.ts`       | the world's import side: `now` as a raw `@external('$root', 'now')` |

The demos write the flat list directly through these emitters — no nested
tree. That is deliberate: the nested `Tree` is an *ergonomics* layer,
orthogonal to the language-agnosticism claim, and writing the flat list keeps
the WIT contract visible in the source. If an AS tree grows, `abi/` is the
right layer to put it on.

## Things that will bite you

- **Precedence.** AssemblyScript (like JavaScript) binds `!=` tighter than
  `&`, so `d & BIT != 0` silently tests the *constant* side. Rust binds `&`
  tighter, which is how the same expression looks in the original — the
  stopwatch's bit tests are parenthesized on purpose, with a comment.
- **`.ts` extension.** asc 0.28 only resolves `.ts` entries and imports.
- **`--optimize` is boolean** in 0.28; the level flag is `--optimizeLevel`.
  (A `--optimize 0` is parsed as a second entry file.)
- The demos' string buffers are arena-allocated and sized for demo-scale
  content; a `view()` that exceeded the 64 KiB arena would walk into the
  static data. The selftest's leak check is the backstop.
