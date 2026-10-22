# waui

A WIT world for UI components. The component owns all UI state and logic; it
exports its widget tree, and any host renders it. Adding a host requires no
component changes.

## The contract (`wit/`, package `waui:ui`)

- **`widget` interface** — `text`, `button`, `container` (row/column) records,
  a `node` variant, a semantic `style` (a `color` token and a `size` token),
  and a `mutation` variant (`changed(node)` / `removed(u32)`).
  - WIT types cannot be recursive, so the tree is a **flat list of nodes in
    depth-first pre-order**; each node carries a `parent` id and hosts rebuild
    the tree from the list (Rust hosts: `host-shared`; web: `web/app.js`).
  - A node's `id` is what `on-click` targets and what `changed` keys on. A
    node never clicked, diffed, or removed may omit it; a node with children
    must have one (they name it); a `changed` for an id-less node is
    unaddressable, and hosts ignore it.
  - `style` is intent, not pixels: hosts map the tokens onto their own
    palette, and a host that cannot express a token renders without it. WIT
    enums with ≤256 variants lay out as one byte, so `style` costs two bytes
    per node.
- **`app` world** — exports `view() -> list<node>` (the current tree),
  `changes() -> list<mutation>` (the diff since the last export), `on-click(u32)`,
  `on-tick()`; imports `now() -> u64` (host clock, ms).

Protocol rules:

- `view()` is always the full tree; a host may re-query it after every event.
- `changes()` is empty when nothing changed; a component that has never
  exported its tree reports every node as `changed`, so a host may bootstrap
  from `changes` alone.
- Hosts may ignore parts of the contract (host-cli ignores `style` and
  `changes`). Components cannot: the node layout is part of the ABI, so a
  component must be built against the WIT it targets.

**Authoring.** Hand-writing the flat list (parent ids, pre-order) is
error-prone, so component authors write a nested tree and project it:

- Rust: the [`component-ui`](crates/component-ui/) crate — a nested `Tree`
  with `.id()`/`.style()` builders and a `lower` that emits the flat list.
  It is dependency-free, and `lower` is generic over the output type because
  each `wit-bindgen generate!` is crate-local, so each demo writes one small
  projection onto its own generated `Node`.
- AssemblyScript: `assembly/ui/tree` — the same shape in AS (see
  [`assembly/README.md`](assembly/README.md)).

## Layout

| path                     | what it is                                                                                          |
| ------------------------ | --------------------------------------------------------------------------------------------------- |
| `wit/`                   | the WIT contract (`widget` + `app`)                                                                   |
| `crates/host-shared`     | wasmtime host core: the bindings, the list→tree rebuild, the retained tree diffs apply against        |
| `crates/host-iced`       | windowed host (iced); style → palette colors + point sizes; applies diffs, skips rebuild on empty diff |
| `crates/host-tui`        | terminal host (ratatui): real layout, framed buttons, a host-local focus ring; style → ANSI colors + weight |
| `crates/host-cli`        | terminal host, ASCII tree, clicks via stdin; ignores `style` and the diff channel                      |
| `web/`                   | web host: one self-contained HTML page per component (`glue.js` + `app.js` + base64 wasm)             |
| `crates/demo-counter`    | the counter: only the readout depends on state, so a click reports one mutation                       |
| `crates/demo-stopwatch`  | the stopwatch: polls `now()`, advances on `on-tick`; diff is a bit mask over its two state nodes      |
| `assembly/`              | the same two demos in AssemblyScript: hand-rolled ABI (`abi/`), object tree layer (`ui/`), no bindings |

The three wasmtime hosts load the same component file. The web host runs the
*pre-lift* core module instead (browsers instantiate core wasm, not components)
through a hand-rolled JS layer over the same ABI.

## Quick start

Prerequisites: stable Rust with the `wasm32-unknown-unknown` target
(`rustup target add wasm32-unknown-unknown`), `wasm-tools` on the PATH,
Node.js with `npm install` run (AS demos), and wabt (AS builds).

```sh
make components      # build the demos, lift them into target/components/
make run-cli         # counter, ASCII terminal host
make run-tui         # counter, TUI host
make run-iced        # counter, window
make run-stop-tui    # stopwatch, TUI host
make run-stop-iced   # stopwatch, window
make web             # self-contained HTML pages in target/web/
make web-test        # headless (Node) check of the web host's ABI layer
make test            # cargo test, all crates

make components-as   # build the AS demos, lift them
make run-tui-as      # counter from the AS component
make web-as          # HTML pages for the AS components
make web-test-as     # the web selftest against the AS modules
```

`make web` inlines the wasm module as base64 into one HTML file per component;
it runs from disk with no server and no network.

## Tests

- `cargo test`: `component-ui` unit tests (lowering pre-order and parent
  links), `host-shared` tests (diff application: in-place replace, subtree
  removal, re-parenting, insertion, id-less skip), and the round-trip test
  (`crates/host-cli/tests/roundtrip.rs`), which drives the counter through the
  same wasmtime host code the real hosts use and asserts on the node content,
  the style, and the diff channel. It runs against the Rust component and, as
  `*_as`, against the AssemblyScript one (skipped when `node_modules` is
  missing).
- `make web-test` (and `-as`): the same assertions from Node, against the core
  modules, through the same `glue.js` the browser page uses.

## Not implemented

Component-directed focus, text input, images (a `resource`), and
multi-component hosts.
