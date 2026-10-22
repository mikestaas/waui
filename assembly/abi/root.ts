// The world's `import` side, in raw canonical-ABI form.
//
// The `waui:ui/app` world imports `now: func() -> u64` — a host clock in
// milliseconds. At the core-module level a world-level import lives in the
// pseudo-module `$root`, so it is exactly this `@external`. (The counter
// never calls a clock and does not import this file: the unused import is
// tree-shaken, as rustc drops the Rust side's unused generated import.)

@external('$root', 'now')
declare function now(): i64;

export { now };
