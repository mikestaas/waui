// The bump arena for ABI buffers, and the world's "post" functions.
//
// Page 0 of linear memory holds all *dynamic* memory, split in two:
//
//   [0 ......... RESULT_END)   this arena — the result buffers (node lists,
//                              mutation lists, itoa scratch)
//   [RESULT_END . 64 KiB)      the `../ui/tree` label pool
//
// Static data (string literals, state globals) lives from page 1 up
// (`--memoryBase 65536`), so the regions cannot collide.
//
// `cabi_post_view` / `cabi_post_changes` are the canonical-ABI "post"
// functions: the host calls one after consuming a list result, and the guest
// must free everything that result allocated. Every byte of a result comes
// from the arena, so a single reset of the bump pointer is the whole free.

/// Where the result arena ends and the label pool begins.
export const RESULT_END = 32 * 1024;

var arena: i32 = 0;

/// Bump-allocate `n` bytes, 8-byte aligned, from the arena, capped at
/// `RESULT_END` so a pathological result can never walk into the label
/// pool. Callers size their buffers to fit; the cap is the backstop.
export function arenaAlloc(n: i32): i32 {
  const p = arena;
  let e = p + ((n + 7) & ~7);
  if (e > RESULT_END) e = RESULT_END;
  arena = e;
  return p;
}

/// Return the arena to its base: frees everything allocated since the last
/// reset. One result's buffers are the only live arena allocations (the
/// host posts before the next query).
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
