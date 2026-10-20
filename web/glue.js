// The web host's ABI layer.
//
// Browsers instantiate core wasm *modules*, not components, so this host
// runs the pre-lift module — the same rustc artifact that `make components`
// lifts for the wasmtime hosts — and speaks the calling convention
// wit-bindgen 0.63 emits for wasm32-unknown-unknown:
//
//   view() -> list<node>     : `view() -> i32`, a pointer to a 16-byte
//                              little-endian {ptr, len} descriptor; the
//                              items are a contiguous run of 32-byte
//                              nodes. The result is guest-allocated and
//                              must be returned to the guest with
//                              `cabi_post_view(desc)`.
//   changes() -> list<mut.>  : same shape; the items are 36-byte
//                              mutations and the buffer is returned with
//                              `cabi_post_changes(desc)`.
//   on-click(id: u32)        : `on-click(i32)` — narrow values pass directly.
//   on-tick()                : `on-tick()`.
//   now() -> u64             : imported as `$root"."now"`, `() -> i64`
//                              (JS BigInt). Provided unconditionally; the
//                              counter never calls it.
//
// The 32-byte node (all little-endian):
//   bytes [0]       variant tag: 0 = text, 1 = button, 2 = container
//                   (the WIT case order in waui:ui/widget)
//   bytes [1..4]    padding
//   bytes [4]       id option flag: 0 = none (an unaddressable node)
//   bytes [5..8]    padding
//   bytes [8..12]   id: u32 (valid only when the flag is set)
//   bytes [12]      parent option flag: 0 = none
//   bytes [13..16]  padding
//   bytes [16..20]  parent id: u32 (valid only when the flag is set)
//   bytes [20]      container: direction (u8, 0 = row, 1 = column)
//   bytes [20..24]  text/button: string pointer
//   bytes [24..28]  text/button: string length (UTF-8 bytes)
//   bytes [28]      text/button: style.color (u8, WIT case order in
//                   waui:ui/widget: 0 = neutral, 1 = accent, 2 = positive,
//                   3 = negative, 4 = warning)
//   bytes [29]      text/button: style.size (u8: 0 = small, 1 = medium,
//                   2 = large)
//   bytes [30..32]  padding
//
// The 36-byte mutation (a variant with a node- or u32-sized payload):
//   bytes [0..4]    tag word: 0 = changed, 1 = removed
//   changed   : the 32-byte node above, starting at byte 4
//   removed   : the node id, u32 at byte 4
//
// Note: a WIT enum lowers to a single byte in the canonical ABI when it has
// 256 or fewer variants (wasmtime's generated bindings assert SIZE32 = 1),
// so the two style tokens pack into two adjacent bytes.
//
// This module is written to run in both a browser and Node so the same code
// can be exercised headlessly (see selftest.mjs).

export async function createApp(wasmBytes) {
  const { instance } = await WebAssembly.instantiate(wasmBytes, {
    $root: {
      // The world's `now: func() -> u64` import, in host milliseconds.
      now: () => BigInt(Date.now()),
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
      : { kind: 'text', id, parent, content: s, style };
  };

  // Decode a `list` result: a 16-byte {ptr, len} descriptor, then `len`
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
    /// { kind: 'changed', node } or { kind: 'removed', id }.
    changes: () =>
      readList(ex.changes, ex.cabi_post_changes, 36, (i32, u8, o) => {
        return i32[o >> 2] === 1
          ? { kind: 'removed', id: i32[(o + 4) >> 2] }
          : { kind: 'changed', node: decodeNode(i32, u8, o + 4) };
      }),

    click: (id) => ex['on-click'](id),
    tick: () => ex['on-tick'](),

    /// The guest's linear memory, in 64 KiB pages (for leak checks).
    memoryPages: () => ex.memory.buffer.byteLength / 65536,
  };
}
