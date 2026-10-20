// The bump arena for ABI buffers, and the world's "post" functions.
//
// Page 0 of linear memory is a monotonically growing arena; all *static*
// data (string literals, the app's state globals) lives from page 1 up,
// because these components are compiled with `--memoryBase 65536`, so the
// two regions can never collide.
//
// `cabi_post_view` / `cabi_post_changes` are the canonical-ABI "post"
// functions: the host calls one after consuming a list result, and the
// guest must free everything that result allocated. Because every byte of
// a result comes from the arena, a single reset of the bump pointer *is*
// the whole free — deterministic, zero net growth across repeated
// view()/changes() calls, and it needs no AS runtime (the module compiles
// with `--runtime stub`), which keeps the component import-free.

var arena: i32 = 0;

/// Bump-allocate `n` bytes, 8-byte aligned, from the arena.
export function arenaAlloc(n: i32): i32 {
  const p = arena;
  arena = p + ((n + 7) & ~7);
  return p;
}

/// Return the whole arena to its base: frees every byte allocated since the
/// last reset. One result's buffers are the only live arena allocations at
/// any time (the host posts before the next query), so this is exact.
export function arenaReset(): void {
  arena = 0;
}

// The world's two list-returning exports each own a canonical-ABI "post"
// function, named `cabi_post_<function>`; both free the same arena.
export function cabi_post_view(_desc: i32): void {
  arenaReset();
}

export function cabi_post_changes(_desc: i32): void {
  arenaReset();
}
