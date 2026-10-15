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
  **flat list of nodes in depth-first pre-order**, each carrying an `id` and
  a `parent` id — the same flat-list encoding real UI serialization formats
  use. Each host rebuilds the tree from the list.
- **`app` world** — the component `export`s `view` (its current flat node
  list), `on-click(id)` and `on-tick()`; it `import`s `now` (a host clock, in
  ms).

The interaction model is *pull*: the host re-queries `view()` after every
event, and the component never reaches into the host's toolkit. Time-based UIs
get a tick from the host and poll `now()` when they want a clock.

## Layout

| crate               | what it is                                                                     |
| ------------------- | ------------------------------------------------------------------------------ |
| `crates/host-iced`  | a windowed host: renders the component's tree with [iced](https://iced.rs)     |
| `crates/host-cli`   | a terminal host: renders the same tree as ASCII, clicks via stdin              |
| `crates/demo-counter`   | a counter — UI *and* logic live entirely in the component                      |
| `crates/demo-stopwatch` | a stopwatch — exercises the import side (`now`) plus host-driven `on-tick`   |

Both hosts load the same component file: **one component, two renderers** —
that is the claim, demonstrated rather than asserted.

## Quick start

Prerequisites: a recent stable Rust toolchain, the `wasm32-unknown-unknown`
target (`rustup target add wasm32-unknown-unknown`), and
[`wasm-tools`](https://github.com/bytecodealliance/wasm-tools) on your
`PATH` (used to lift core wasm modules into components).

```sh
make components      # build the demos, lift them into target/components/
make run-cli         # counter, in the terminal
make run-iced        # counter, in a window
make run-stop-cli    # stopwatch, in the terminal
make test            # headless round-trip contract test
```

The round-trip test (`crates/host-cli/tests/roundtrip.rs`) builds the counter
component, drives `on-click` through the same wasmtime host code the real
hosts use, and asserts on the returned widget tree — the WIT world is testable
in CI without a window.

## Status

This is the first proof of the loop: **component-owned state → WIT widget
tree → host-rendered UI**. Deliberately not solved yet, in order of
importance: diffing (we re-export the whole tree per event), styling, focus
and input, images (a `resource`), and multi-window/multi-component hosts.
