# Agent notes

## Prose style (comments and docs)

- Short declarative sentences. Facts only; one reason per comment, and only
  for the non-obvious (an ABI offset, a memory band, a language precedence
  trap, a GC retention rule). If the code says it, don't.
- No intensifiers or hedges: "actually", "exactly", "really", "very", "just".
- No "and nothing else", "— which is the point", "accounts for everything",
  or other self-congratulatory clauses; no design rationale repeated across
  a function doc, a header, and a README (state it once, where it belongs).
- No anthropomorphism: nodes and hosts don't "see", "know", or "say".
- No buzzwords or marketing: "powerful", "robust", "seamless", "elegantly".
- "Not implemented" is stated as such in the README, never euphemized.
- When in doubt, cut the sentence; a reader can ask.

## Verification

Before calling a change done: `make components-as`, `make components`,
`make web-test-as`, `make web-test`, `cargo test`, and a `host-cli` smoke run
on the built components. The web selftest is the byte-parity gate between
the Rust and AssemblyScript demos; its assertions must not be edited to fit.

## Multiple sessions

- Multiple sessions run at once. A session that needs its own checkout
  takes a git worktree, not a scratch directory: a worktree is a real
  checkout that shares the repo's history, so `make`, the tests, and the
  git tooling work unchanged.
- Commit work once it is done and verified. Don't leave finished work
  uncommitted.
