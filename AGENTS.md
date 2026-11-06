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
- When generating Git commits, do not include any 'Co-Authored-By' trailers or external attributions.

## Verification

Before calling a change done: `make components-as`, `make components`,
`make web-test-as`, `make web-test`, `cargo test`, `make ui-test`,
`make site`, and a `host-cli` smoke run on the built components.
The web selftest is the byte-parity gate between the Rust and
AssemblyScript demos. Do not weaken an assertion to make a failing
check pass; extending the gate to cover a new contract field is
allowed. The component outputs it compares stay the source of truth.

## Multiple sessions

- When other sessions are active in this repo, work in your own git
  worktree, never in the main checkout. "The main checkout is clean"
  is not an exemption.
- Use a worktree, not a scratch directory: a worktree shares the repo's
  history, so `make`, the tests, and the git tooling work unchanged.
- Create one with `git worktree add ../waui-<task> -b <branch>`, then
  move the session there.
- Commit work once it is done and verified. Don't leave finished work
  uncommitted.
