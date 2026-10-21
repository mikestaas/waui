# waui

[WebAssembly](https://webassembly.org/) user interface experiment.

The goal: a **renderer-agnostic WIT world** for wasm components to expose a
user interface. A component owns *all* UI state and logic; it exports its
current widget tree, and any host renders it with whatever toolkit it happens
to be written in. The host is a dumb renderer — if you can add a new host
without touching the component, the experiment is working.

## The WIT world

The whole contract lives in [`wit/`](wit/) — package `waui:ui`:

- **`widget` interface** — a deliberately tiny, renderer-agnostic widget tree:
  `text`, `button`, and `container` (row/column) records, and a `node`
  variant. WIT types may not be recursive, so the tree is encoded as a
  **flat list of nodes in depth-first pre-order**, each carrying a `parent`
  id and an optional `id` — the same flat-list encoding real UI serialization
  formats use. Each host rebuilds the tree from the list — the Rust hosts
  share a single reference rebuild (`crates/host-shared`), and the web host
  re-implements the same logic in JavaScript. `text` and `button` nodes
  also carry a `style`: a **semantic** `color` (`neutral`/`accent`/
  `positive`/`negative`/`warning`) and `size` (`small`/`medium`/`large`).
  The contract stops at intent, not pixels — a TUI can't render `#f7768e` but
  it *can* render "negative", so each host maps the tokens onto its own
  palette and hosts that can't express a token degrade gracefully. (WIT
  enums with ≤256 variants lay out as single bytes, so the two tokens cost
  two bytes per node.) The `id` is a node's *identity*: what `on-click`
  targets and what `changes` diffs key on. A node that is never clicked,
  diffed, or removed may omit it — but a container *must* keep one (its
  children name it), and an id-less node is unaddressable.
- **`app` world** — the component `export`s `view` (its current flat node
  list), `changes` (the diff of that list, per `mutation`), `on-click(id)` and
  `on-tick()`; it `import`s `now` (a host clock, in ms).

The interaction model is *pull with diff*: the host bootstraps from `view()`
(a full snapshot) and, after each event it drives the component with, applies
the component's `changes()` to its retained tree — only the nodes the
component changed since the last export, never the whole tree. A `mutation`
is either `changed(node)` (keyed by the node's `id`; the host replaces its
retained copy in place) or `removed(id)` (the host prunes the node and, for a
container, its subtree). The component is in a position no host is: it *knows*
which of its nodes its state touches, so the diff is computed where it's
cheapest. The component never reaches into the host's toolkit; time-based UIs
get a tick from the host and poll `now()` when they want a clock.

Two rules keep the protocol safe. First, **`view()` is always the full
truth** — a host that ignores `changes` and simply re-queries `view()` after
every event stays correct, so diffing is an optimization, opt-in per host
(host-cli opts out; iced, the TUI, and the web host use it). Second, a
component that has *never* exported its tree reports every node as `changed`,
so a host may bootstrap from `changes` alone.

The contract is versioned by its `waui:ui` package name, and additions are
built to be **host-compatible**. The contract-level half of a Rust host — the
WIT bindings, the documented list→tree rebuild, and the retained tree the diff
applies against — lives in the shared `host-shared` core, so a contract
addition lands in one place; what each host then *chooses* to express is its
renderer. When `style` was added, `host-cli`'s ASCII renderer simply never
draws it, while iced, the TUI, and the web page each express it in their own
idiom. Components are the exception: a node's layout is part of the ABI, so
they must be built against the WIT they target.

Authoring a UI *as the flat list* — hand-assigning parent ids, hand-ordering
the pre-order walk — is not a pleasant way to write a component, so the
component side has its own mirror of `host-shared`: the
[`component-ui`](crates/component-ui/) crate supplies a **nested** `Tree`
(authors write `Tree::column([...])` with the nesting carrying parentage) and
a `lower` that projects it into the contract's flat list — the component-side
counterpart of the hosts' list→tree rebuild. It is deliberately
dependency-free: its token types are plain Rust mirrors of the WIT's, and the
projection is generic over the output type, because each `wit-bindgen
generate!` is crate-local and a component must project onto *its own*
generated `Node` (the demos each write that one small projection). A
declarative macro over this tree is the natural next layer, held back for a
deliberate syntax decision.

And the contract is **language-agnostic**, the same way it is renderer-agnostic.
The two demos also exist in [AssemblyScript](assembly/), and all four hosts run
the AssemblyScript components with zero changes: iced, the TUI, the CLI, and
the web page (which runs the pre-lift module through the same unmodified
`glue.js`). No `wit-bindgen` exists for that language, so instead of generated
bindings the canonical ABI is hand-rolled against the WIT (in `assembly/abi/`),
and the lift is typed by an explicit `wasm-tools component embed wit/` step —
the WIT file, not the toolchain, is the shared spec. The ergonomics are
per-language in a small way worth noting: the AS demos author through
`assembly/ui/`, a scope-built tree of fixed slots that projects through those
same emitters — the *form* of the tree differs because `--runtime stub` has no
heap to nest values with, the *role* is identical, and the output is
byte-identical. One wrinkle is worth
naming: the world's `on-click`/`on-tick` names contain hyphens that
AssemblyScript identifiers cannot hold, so the build renames those two exports
after compilation — a 60-line script, part of the pipeline rather than a hole
in the claim. `make components-as` builds them, and the round-trip and web
selftests run the same assertions against both producer languages.

## Layout

| crate / dir            | what it is                                                                     |
| ---------------------- | ------------------------------------------------------------------------------ |
| `crates/host-shared`   | the shared core of the wasmtime hosts: the WIT bindings + the list→tree rebuild + the retained tree `changes` diffs apply against (contract-level code; hosts add only a renderer) |
| `crates/host-iced`     | a windowed host: the tree with [iced](https://iced.rs) on the web page's dark theme; style → palette colors + point sizes; applies diffs, skipping the rebuild on no-op ticks |
| `crates/host-tui`      | a terminal host: real layout, framed buttons, host-local focus ring (ratatui); style → ANSI colors + weight; applies diffs — at rest it makes zero calls into the component |
| `crates/host-cli`      | a minimal terminal host: the tree as ASCII, clicks via stdin; ignores *both* style and the diff channel (proof hosts can lag the contract) |
| `web/`                 | a web host: one self-contained HTML page per component, zero dependencies; style → CSS tokens; applies content diffs *in place* (no DOM rebuild) |
| `crates/component-ui`  | the component-side mirror of `host-shared`: a nested `Tree` + `lower` into the flat list (dependency-free; hosts add only a renderer, components add only a projection) |
| `crates/demo-counter`  | a counter — UI *and* logic live entirely in the component (the number asks for "negative" below zero) |
| `crates/demo-stopwatch`| a stopwatch — exercises the import side (`now`) plus host-driven `on-tick` (glows "accent" while running) |
| `assembly/`            | the same two demos in **AssemblyScript** — a hand-rolled canonical ABI (`abi/`), the demos' scope-built tree layer (`ui/`), no bindings, lifted against the same WIT via `component embed`; proof the contract is language-agnostic |

The wasmtime hosts (iced, tui, cli) load the same lifted component file —
**one component, many renderers** — that is the claim, demonstrated rather
than asserted. They share the `host-shared` core (bindings + the node-list
rebuild) and differ only in their renderer; the web host runs the *pre-lift*
core module instead (browsers instantiate core wasm, not components) with a
hand-rolled JS layer over the calling convention the toolchain emits, so it
exercises the same WIT contract through a different transport — including its
own JavaScript copy of the rebuild.

## Quick start

Prerequisites: a recent stable Rust toolchain, the `wasm32-unknown-unknown`
target (`rustup target add wasm32-unknown-unknown`),
[`wasm-tools`](https://github.com/bytecodealliance/wasm-tools) on your
`PATH` (used to lift core wasm modules into components), and Node.js (for the
web host and the AssemblyScript demos — `npm install` for the latter).

```sh
make components      # build the demos, lift them into target/components/
make run-cli         # counter, in the terminal (minimal ASCII host)
make run-tui         # counter, in the terminal (real TUI: focus ring, arrow keys, enter)
make run-iced        # counter, in a window
make run-stop-tui    # stopwatch, in the TUI
make run-stop-iced   # stopwatch, in a window
make web             # self-contained HTML pages in target/web/ — open directly, no server
make web-test        # headless (Node) check of the web host's ABI layer
make test            # headless round-trip contract test

# The same demos in AssemblyScript — the same hosts, a different compiler.
make components-as   # build the AS demos, lift them (needs `npm install` first)
make run-tui-as      # the counter, in the TUI, from the AssemblyScript component
make web-as          # self-contained HTML pages for the AS components
make web-test-as     # the web selftest against the AS modules (same assertions)
```

`make web` produces a single HTML file per component with the wasm module
inlined as base64 — double-click it and it runs: no server, no npm, no
network. (Internals: `web/glue.js` implements the component's ABI in plain
JS; `web/app.js` renders the node list as DOM elements; `web/build.mjs`
assembles the page.)

The round-trip test (`crates/host-cli/tests/roundtrip.rs`) builds the counter
component, drives `on-click` through the same wasmtime host code the real
hosts use, and asserts on the returned widget tree — its content *and* its
semantic style — plus on the diff channel: one click reports exactly one
changed node, no-op events report none, and a fresh instance that never calls
`view()` still yields its whole tree from the first `changes()`.
The same two scenarios run against the AssemblyScript counter as `*_as` tests,
which skip gracefully when the npm toolchain isn't installed — the Rust
coverage is the part CI must always run. `make web-test` does the same for
the browser path from Node (`make web-test-as` for the AS modules). `make test`
also runs the `component-ui` unit tests (the lowering's pre-order and
parent-link invariants, over the crate's own flat mirror) and the `host-shared`
tests (the diff machinery), all headless.

## Status

The loop — **component-owned state → WIT widget tree → host-rendered UI** —
now works in a window, in a terminal, and in a browser, from one component,
including styling: the counter's number asks for "negative" when it goes
below zero, and each host honors it in its own idiom — iced paints a palette
color, the TUI paints bold ANSI red, the web page a CSS token. And it works
*across languages*: the two demos also exist in AssemblyScript, hand-rolled
against the same WIT, and all four hosts run them unchanged — the contract is
renderer-agnostic *and* producer-agnostic. And it works
*incrementally*: the demos report only what changed (a counter click is one
mutation; a paused stopwatch is zero), so an idle UI costs the hosts nothing
— the TUI makes no component calls at rest, and the web page updates its
readout in place without rebuilding the DOM. And it is *pleasurable to
write*: the demos author their views as nested `component-ui` trees — or,
in AssemblyScript, the scope-built slot tree of `assembly/ui/` — and
project to the flat list at the ABI boundary, instead of hand-maintaining
the pre-order encoding. Focus is a host concern: the TUI keeps a local ring
over the component's buttons while the component stays unaware of it.
Deliberately not solved yet, in order of importance: a declarative macro
over the nested tree (a syntax decision first), component-directed focus and
text input, images (a `resource`), and multi-window/multi-component hosts.
