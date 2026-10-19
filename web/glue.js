// The web host's ABI layer.
//
// Browsers instantiate core wasm *modules*, not components, so this host
// runs the pre-lift module — the same rustc artifact that `make components`
// lifts for the wasmtime hosts — and speaks the calling convention
// wit-bindgen 0.63 emits for wasm32-unknown-unknown:
//
//   view() -> list<node> : `view() -> i32`, a pointer to a 16-byte
//                           little-endian {ptr, len} descriptor; the items
//                           are a contiguous run of 28-byte records. The
//                           result is guest-allocated and must be returned
//                           to the guest with `cabi_post_view(desc)`.
//   on-click(id: u32)    : `on-click(i32)` — narrow values pass directly.
//   on-tick()            : `on-tick()`.
//   now() -> u64         : imported as `$root"."now"`, `() -> i64`
//                           (JS BigInt). Provided unconditionally; the
//                           counter never calls it.
//
// The 28-byte node record (all little-endian):
//   bytes [0]       variant tag: 0 = text, 1 = button, 2 = container
//                   (the WIT case order in waui:ui/widget)
//   bytes [4..8]    id: u32
//   bytes [8]       parent option flag: 0 = none
//   bytes [12..16]  parent id: u32 (valid only when the flag is set)
//   bytes [16..20]  text/button: string pointer; container: direction
//                   (u8, 0 = row, 1 = column)
//   bytes [20..24]  text/button: string length (UTF-8 bytes)
//   bytes [24]      text/button: style.color (u8, WIT case order in
//                   waui:ui/widget: 0 = neutral, 1 = accent, 2 = positive,
//                   3 = negative, 4 = warning)
//   bytes [25]      text/button: style.size (u8: 0 = small, 1 = medium,
//                   2 = large)
//   bytes [26..28]  padding (record is 4-byte aligned)
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

  return {
    /// Decode `view()`'s result into a flat list of node objects.
    view() {
      const desc = ex.view();
      const i32 = new Int32Array(mem());
      const u8 = new Uint8Array(mem());
      const ptr = i32[desc >> 2];
      const len = i32[(desc + 4) >> 2];

      // The semantic tokens each host maps onto its own palette, in the
      // WIT case order of waui:ui/widget (unknown values degrade safely).
      const COLORS = ['neutral', 'accent', 'positive', 'negative', 'warning'];
      const SIZES = ['small', 'medium', 'large'];

      const nodes = new Array(len);
      for (let k = 0; k < len; k++) {
        const o = ptr + k * 28;
        const id = i32[(o + 4) >> 2];
        const parent = u8[o + 8] !== 0 ? i32[(o + 12) >> 2] : null;
        const tag = u8[o];
        if (tag === 2) {
          nodes[k] = {
            kind: 'container',
            id,
            parent,
            direction: u8[o + 16] === 1 ? 'column' : 'row',
          };
        } else {
          const sp = i32[(o + 16) >> 2];
          const sl = i32[(o + 20) >> 2];
          const s = new TextDecoder().decode(u8.subarray(sp, sp + sl));
          const style = {
            color: COLORS[u8[o + 24]] ?? 'neutral',
            size: SIZES[u8[o + 25]] ?? 'medium',
          };
          nodes[k] =
            tag === 1
              ? { kind: 'button', id, parent, label: s, style }
              : { kind: 'text', id, parent, content: s, style };
        }
      }

      // Give the guest-allocated buffer back (frees the records and the
      // strings they point at) so long sessions don't leak.
      ex.cabi_post_view(desc);
      return nodes;
    },

    click: (id) => ex['on-click'](id),
    tick: () => ex['on-tick'](),

    /// The guest's linear memory, in 64 KiB pages (for leak checks).
    memoryPages: () => ex.memory.buffer.byteLength / 65536,
  };
}
