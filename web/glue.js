// The web host's ABI layer.
//
// Browsers instantiate core wasm *modules*, not components, so this host
// runs the pre-lift module — the same rustc artifact that `make components`
// lifts for the wasmtime hosts — and speaks the calling convention
// wit-bindgen 0.63 emits for wasm32-unknown-unknown:
//
//   view() -> list<node>     : `view() -> i32`, a pointer to an 8-byte
//                              little-endian {ptr, len} descriptor; the
//                              items are a contiguous run of 32-byte
//                              nodes. The result is guest-allocated and
//                              must be returned to the guest with
//                              `cabi_post_view(desc)`.
//   changes() -> list<mut.>  : same shape; the items are 40-byte
//                              mutations and the buffer is returned with
//                              `cabi_post_changes(desc)`.
//   on-click(id: u32)        : `on-click(i32)` — passes directly.
//   on-text(id: u32, text)   : `on-text(i32, i32, i32)` — the id, then the
//                              string's pointer + length, in the *guest's*
//                              memory (the canonical ABI). The host has no
//                              memory of its own, so `text()` allocates the
//                              buffer through the guest's `cabi_realloc`
//                              (the canonical allocator) and writes to it.
//                              The buffer is a live allocation the callee
//                              owns: a Rust component wraps it into a String
//                              and frees it on drop, so it must come from the
//                              guest's allocator, not a fixed region.
//   on-tick()                : `on-tick()`.
//   schedule(p: u64)         : `$root"."schedule"`, `(i64)` — the
//                              component's refresh request in ms (0 = no
//                              periodic wake). Delivered to the
//                              `onSchedule` hook, if `createApp` was
//                              passed one; a no-op without it.
//   now() -> u64             : imported as `$root"."now"`, `() -> i64`
//                              (JS BigInt). Provided unconditionally; the
//                              counter never calls it.
//
// The 32-byte node (all little-endian):
//   bytes [0]       variant tag: 0 = text, 1 = button, 2 = container,
//                   3 = input (the WIT case order in waui:ui/widget)
//   bytes [1..4]    padding
//   bytes [4]       id option flag: 0 = none (an unaddressable node)
//   bytes [5..8]    padding
//   bytes [8..12]   id: u32 (valid only when the flag is set)
//   bytes [12]      parent option flag: 0 = none
//   bytes [13..16]  padding
//   bytes [16..20]  parent id: u32 (valid only when the flag is set)
//   bytes [20]      container: direction (u8, 0 = row, 1 = column)
//   bytes [21]      container: align (u8, 0 = start, 1 = center, 2 = end)
//   bytes [20..24]  text/button/input: string pointer
//   bytes [24..28]  text/button/input: string length (UTF-8 bytes)
//   bytes [28]      text/button/input: style.color (u8, WIT case order in
//                   waui:ui/widget: 0 = neutral, 1 = accent, 2 = positive,
//                   3 = negative, 4 = warning)
//   bytes [29]      text/button/input: style.size (u8: 0 = small,
//                   1 = medium, 2 = large)
//   bytes [30..32]  padding
//
// The 40-byte mutation (a variant with a record- or u32-sized payload):
//   bytes [0]       tag: 0 = changed, 1 = removed (one byte, like the node
//                   tag — the canonical ABI sizes a variant's discriminant
//                   to the case count: u8 for 256 or fewer cases)
//   bytes [1..4]    padding
//   changed   : the 32-byte node above at byte 4, then the node's index
//               (a u32, its 0-based position among its parent's children)
//               at byte 36
//   removed   : the node id, u32 at byte 4
//
// The tag is read as a single byte, not a word: the padding bytes are not
// written by the producer, so a word read would pick up stale heap data and
// misread a `removed` (tag 1) as a `changed`.
//
// Note: a WIT enum lowers to a single byte in the canonical ABI when it has
// 256 or fewer variants (wasmtime's generated bindings assert SIZE32 = 1),
// so the two style tokens pack into two adjacent bytes.
//
// This module is written to run in both a browser and Node so the same code
// can be exercised headlessly (see selftest.mjs).

export async function createApp(wasmBytes, hooks = {}) {
  const { instance } = await WebAssembly.instantiate(wasmBytes, {
    $root: {
      // The world's `now: func() -> u64` import, in host milliseconds.
      now: () => BigInt(Date.now()),
      // The world's `schedule: func(u64)` import: the component's refresh
      // request, in ms; 0 = no periodic wake.
      schedule: (p) => {
        if (hooks.onSchedule) hooks.onSchedule(Number(p));
      },
    },
  });
  const ex = instance.exports;

  // `memory.grow` detaches the previous ArrayBuffer, so typed views are
  // created fresh inside each call, after the guest has settled.
  const mem = () => ex.memory.buffer;

  // The semantic tokens each host maps onto its own palette, in the WIT
  // case order of waui:ui/widget (unknown values degrade safely).
  const COLORS = ['neutral', 'accent', 'positive', 'negative', 'warning'];
  const SIZES = ['small', 'medium', 'large'];
  const ALIGNS = ['start', 'center', 'end'];
  const enc = new TextEncoder();
  // The AssemblyScript scratch region's capacity (the top 4 KiB of page 0,
  // a bump allocator through `cabi_realloc`). `text()` clamps an over-long
  // buffer to it so the AssemblyScript demo (bounded region) and the Rust
  // demo (unbounded heap) stay byte-parity.
  const SCRATCH_CAP = 4096;

  // Decode one 32-byte node record at byte offset `o`.
  const decodeNode = (i32, u8, o) => {
    const id = u8[o + 4] !== 0 ? i32[(o + 8) >> 2] : null;
    const parent = u8[o + 12] !== 0 ? i32[(o + 16) >> 2] : null;
    const tag = u8[o];
    if (tag === 2) {
      return {
        kind: 'container',
        id,
        parent,
        direction: u8[o + 20] === 1 ? 'column' : 'row',
        align: ALIGNS[u8[o + 21]] ?? 'center',
      };
    }
    const sp = i32[(o + 20) >> 2];
    const sl = i32[(o + 24) >> 2];
    const s = new TextDecoder().decode(u8.subarray(sp, sp + sl));
    const style = {
      color: COLORS[u8[o + 28]] ?? 'neutral',
      size: SIZES[u8[o + 29]] ?? 'medium',
    };
    return tag === 1
      ? { kind: 'button', id, parent, label: s, style }
      : tag === 3
        ? { kind: 'input', id, parent, content: s, style }
        : { kind: 'text', id, parent, content: s, style };
  };

  // Decode a `list` result: an 8-byte {ptr, len} descriptor, then `len`
  // items of `stride` bytes, then hand the guest-allocated buffer back.
  const readList = (call, post, stride, item) => {
    const desc = call();
    const i32 = new Int32Array(mem());
    const u8 = new Uint8Array(mem());
    const ptr = i32[desc >> 2];
    const len = i32[(desc + 4) >> 2];
    const out = new Array(len);
    for (let k = 0; k < len; k++) {
      out[k] = item(i32, u8, ptr + k * stride);
    }
    post(desc);
    return out;
  };

  return {
    /// Decode `view()`'s result into a flat list of node objects.
    view: () => readList(ex.view, ex.cabi_post_view, 32, decodeNode),

    /// Decode `changes()`'s result into a list of mutation objects:
    /// { kind: 'changed', node, index } or { kind: 'removed', id }.
    changes: () =>
      readList(ex.changes, ex.cabi_post_changes, 40, (i32, u8, o) => {
        // The tag is one byte (see the layout note above); the padding after
        // it is unwritten, so it must not be read as part of the tag.
        return u8[o] === 1
          ? { kind: 'removed', id: i32[(o + 4) >> 2] }
          : {
              kind: 'changed',
              node: decodeNode(i32, u8, o + 4),
              index: i32[(o + 36) >> 2],
            };
      }),

    click: (id) => ex['on-click'](id),
    tick: () => ex['on-tick'](),

    /// The world's `on-text` export: the input with `id` now holds `text`
    /// (the host's whole current buffer). The canonical ABI wants the string
    /// in the guest's memory, so the buffer is allocated through the guest's
    /// `cabi_realloc` (the canonical allocator) and written there. The buffer
    /// is a live allocation the callee owns — a Rust component wraps it into
    /// a String and frees it on drop — so it must come from the guest's
    /// allocator, not a fixed region. An over-long buffer is clamped to the
    /// AssemblyScript scratch region's 4 KiB, at a UTF-8 character boundary,
    /// so the two demos stay byte-parity (the Rust heap is unbounded).
    text: (id, text) => {
      let s = text;
      for (;;) {
        const bytes = enc.encode(s);
        if (bytes.length <= SCRATCH_CAP) {
          if (bytes.length === 0) {
            // An empty buffer needs no allocation: (0, 0) is a null,
            // zero-length string the callee can wrap or read safely.
            ex['on-text'](id, 0, 0);
            return;
          }
          const ptr = ex.cabi_realloc(0, 0, bytes.length, 1);
          // The allocation may have grown the guest's memory, detaching the
          // previous ArrayBuffer: build the view after it.
          const u8 = new Uint8Array(mem());
          u8.set(bytes, ptr);
          ex['on-text'](id, ptr, bytes.length);
          return;
        }
        // Drop the last code point (a surrogate pair, if any).
        const last = s.charCodeAt(s.length - 1);
        const units = last >= 0xdc00 && last <= 0xdfff ? 2 : 1;
        s = s.slice(0, s.length - units);
      }
    },

    /// The guest's linear memory, in 64 KiB pages (for leak checks).
    memoryPages: () => ex.memory.buffer.byteLength / 65536,
  };
}
