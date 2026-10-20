// The world's `import` side, in raw canonical-ABI form.
//
// The `waui:ui/app` world imports `now: func() -> u64` — a host clock in
// milliseconds. At the core-module level a world-level import lives in the
// pseudo-module `$root`, so it is exactly this `@external`. (The counter
// never calls a clock and so does not import this file: an unreferenced
// import would be tree-shaken away, mirroring the Rust component, whose
// unused generated import rustc drops the same way.)

@external('$root', 'now')
declare function now(): i64;

export { now };
