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
  use. Each host rebuilds the tree from the list. `text` and `button` nodes
  also carry a `style`: a **semantic** `color` (`neutral`/`accent`/
  `positive`/`negative`/`warning`) and `size` (`small`/`medium`/`large`).
  The contract stops at intent, not pixels — a TUI can't render `#f7768e` but
  it *can* render "negative", so each host maps the tokens onto its own
  palette and hosts that can't express a token degrade gracefully. (WIT
  enums with ≤256 variants lay out as single bytes, so the two tokens cost
  two bytes per node.)
- **`app` world** — the component `export`s `view` (its current flat node
  list), `on-click(id)` and `on-tick()`; it `import`s `now` (a host clock, in
  ms).

The interaction model is *pull*: the host re-queries `view()` after every
event, and the component never reaches into the host's toolkit. Time-based UIs
get a tick from the host and poll `now()` when they want a clock.

The contract is versioned by its `waui:ui` package name, and additions are
built to be **host-compatible**: when `style` was added to `text` and
`button`, only hosts that *use* it had to change — `host-cli` needed zero
changes (it simply never reads the new field). Components are the exception:
a node's layout is part of the ABI, so they must be built against the WIT
they target.

## Layout

| crate / dir            | what it is                                                                     |
| ---------------------- | ------------------------------------------------------------------------------ |
| `crates/host-iced`     | a windowed host: the tree with [iced](https://iced.rs); style → palette colors + point sizes |
| `crates/host-tui`      | a terminal host: real layout, framed buttons, host-local focus ring (ratatui); style → ANSI colors + weight |
| `crates/host-cli`      | a minimal terminal host: the tree as ASCII, clicks via stdin; ignores style (proof hosts can lag the contract) |
| `web/`                 | a web host: one self-contained HTML page per component, zero dependencies; style → CSS tokens |
| `crates/demo-counter`  | a counter — UI *and* logic live entirely in the component (the number asks for "negative" below zero) |
| `crates/demo-stopwatch`| a stopwatch — exercises the import side (`now`) plus host-driven `on-tick` (glows "accent" while running) |

The wasmtime hosts (iced, tui, cli) load the same lifted component file —
**one component, many renderers** — that is the claim, demonstrated rather
than asserted. The web host runs the *pre-lift* core module instead (browsers
instantiate core wasm, not components) with a hand-rolled JS layer over the
calling convention the toolchain emits, so it exercises the same WIT contract
through a different transport.

## Quick start

Prerequisites: a recent stable Rust toolchain, the `wasm32-unknown-unknown`
target (`rustup target add wasm32-unknown-unknown`),
[`wasm-tools`](https://github.com/bytecodealliance/wasm-tools) on your
`PATH` (used to lift core wasm modules into components), and Node.js (for the
web host).

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
```

`make web` produces a single HTML file per component with the wasm module
inlined as base64 — double-click it and it runs: no server, no npm, no
network. (Internals: `web/glue.js` implements the component's ABI in plain
JS; `web/app.js` renders the node list as DOM elements; `web/build.mjs`
assembles the page.)

The round-trip test (`crates/host-cli/tests/roundtrip.rs`) builds the counter
component, drives `on-click` through the same wasmtime host code the real
hosts use, and asserts on the returned widget tree — its content *and* its
semantic style — so the WIT world is testable in CI without a window.
`make web-test` does the same for the browser path from Node.

## Status

The loop — **component-owned state → WIT widget tree → host-rendered UI** —
now works in a window, in a terminal, and in a browser, from one component,
including styling: the counter's number asks for "negative" when it goes
below zero, and each host honors it in its own idiom — iced paints a palette
color, the TUI paints bold ANSI red, the web page a CSS token. Focus is a
host concern: the TUI keeps a local ring over the component's buttons while
the component stays unaware of it. Deliberately not solved yet, in order of
importance: diffing (we re-export the whole tree per event),
component-directed focus and text input, images (a `resource`), and
multi-window/multi-component hosts.
