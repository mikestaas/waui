// The bump arena for ABI buffers, and the world's "post" functions.
//
// Page 0 of linear memory holds all *dynamic* memory, split in two:
//
//   [0 ......... RESULT_END)   this arena — the monotonically growing
//                              result buffers (node lists, mutation
//                              lists, the itoa scratch)
//   [RESULT_END . 64 KiB)      the `../ui/tree` label pool — one partition
//                              per stable node id, allocated at first
//                              export, never freed (the tree's *structure*
//                              is on the runtime heap, page 1 and up)
//
// All *static* data (string literals, the app's state globals) lives from
// page 1 up, because these components are compiled with
// `--memoryBase 65536`, so the regions can never collide with the data
// segment.
//
// `cabi_post_view` / `cabi_post_changes` are the canonical-ABI "post"
// functions: the host calls one after consuming a list result, and the
// guest must free everything that result allocated. Because every byte of
// a result comes from the arena, a single reset of the bump pointer *is*
// the whole free — deterministic, zero net growth across repeated
// view()/changes() calls. (Tree objects are rebuilt per call and are the
// GC's to reclaim; the label pool is static and never part of a result's
// allocation, so the post functions have exactly one job.)

/// Where the result arena ends and the label pool begins.
export const RESULT_END = 32 * 1024;

var arena: i32 = 0;

/// Bump-allocate `n` bytes, 8-byte aligned, from the arena.
///
/// The bump is capped at `RESULT_END` so a pathological result can never
/// walk into the label pool. Callers are expected to size their buffers
/// to fit (the `ui/tree` layer does, by construction); the cap is the
/// backstop, not the interface.
export function arenaAlloc(n: i32): i32 {
  const p = arena;
  let e = p + ((n + 7) & ~7);
  if (e > RESULT_END) e = RESULT_END;
  arena = e;
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
