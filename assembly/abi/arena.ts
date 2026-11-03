// The bump arena for ABI buffers, and the world's "post" functions.
//
// Page 0 of linear memory holds all *dynamic* memory, split in three:
//
//   [0 ......... RESULT_END)          this arena — the result buffers (node
//                                      lists, mutation lists, itoa scratch)
//   [RESULT_END . HOST_SCRATCH_BASE)  the `../ui/tree` label pool
//   [HOST_SCRATCH_BASE . 64 KiB)      the host's scratch region (see
//                                      `host_scratch` and `cabi_realloc`)
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

/// Where the label pool ends and the host's scratch region begins: the top
/// 4 KiB of page 0.
export const HOST_SCRATCH_BASE = 60 * 1024;
/// The scratch region's capacity in bytes.
export const HOST_SCRATCH_CAP = 65536 - HOST_SCRATCH_BASE;

/// The base of the host's scratch region: a fixed 4 KiB of page 0 that
/// neither the arena nor the label pool ever allocates. The web host —
/// which runs the pre-lift core module and has no memory of its own —
/// writes an `on-text` string here before calling the export (the canonical
/// ABI wants the string in the guest's memory). Not part of the WIT world;
/// a convention of the core-module ABI (see `web/glue.js`). The wasmtime
/// hosts run the lifted component, whose lift glue allocates the same region
/// through `cabi_realloc` instead, and never call this.
export function host_scratch(): i32 {
  return HOST_SCRATCH_BASE;
}

// The bump cursor of the scratch region (see `cabi_realloc`).
var reallocCur: i32 = HOST_SCRATCH_BASE;

/// Round `n` up to `align` (a power of two).
function roundUp(n: i32, align: i32): i32 {
  return (n + (align - 1)) & ~(align - 1);
}

/// The canonical ABI's `cabi_realloc`: a buffer in the module's own memory,
/// for a string parameter. The wasmtime hosts run the lifted component, and
/// its lift glue allocates the `on-text` string through this (the canonical
/// ABI wants a parameter string in the callee's memory). The web host runs
/// the pre-lift module and never calls it (it writes to the region directly,
/// see `host_scratch`).
///
/// The region is a bump allocator: one buffer is live at a time (the glue
/// allocates, copies, calls, and frees), so a free returns the region to its
/// base. A buffer longer than the 4 KiB region traps.
export function cabi_realloc(oldPtr: i32, oldLen: i32, newLen: i32, align: i32): i32 {
  const base = HOST_SCRATCH_BASE;
  const end = base + HOST_SCRATCH_CAP;
  const a = align > 0 ? align : 1;
  if (newLen <= 0) {
    // A free: the one live buffer is gone; the region returns to its base.
    reallocCur = base;
    return 0;
  }
  if (oldPtr >= base && oldPtr < end && oldPtr + roundUp(oldLen, a) == reallocCur) {
    // The top block: resize in place.
    const sz = roundUp(newLen, a);
    if (oldPtr + sz > end) unreachable();
    reallocCur = oldPtr + sz;
    return oldPtr;
  }
  // A fresh buffer from the bump.
  const p = roundUp(reallocCur, a);
  const sz = roundUp(newLen, a);
  if (p + sz > end) unreachable();
  reallocCur = p + sz;
  return p;
}

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
