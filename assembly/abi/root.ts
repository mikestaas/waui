// The world's `import` side, in raw canonical-ABI form.
//
// The `waui:ui/app` world imports two host functions: `now` (a clock in
// milliseconds) and `schedule` (a refresh-period request, in ms; 0 = no
// periodic wake). At the core-module level a world-level import lives in
// the pseudo-module `$root`, so each is exactly one `@external`. (The
// counter never calls either and does not import this file: the unused
// import is tree-shaken, as rustc drops the Rust side's unused generated
// import.)

@external('$root', 'now')
declare function now(): i64;


@external('$root', 'schedule')
declare function schedule(period: i64): void;

export { now, schedule };
