// The web host's ABI layer.
//
// Browsers instantiate core wasm *modules*, not components, so this host
// runs the pre-lift module — the same rustc artifact that `make components`
// lifts for the wasmtime hosts — and speaks the calling convention
// wit-bindgen 0.63 emits for wasm32-unknown-unknown:
//
//   view() -> list<node>     : `view() -> i32`, a pointer to an 8-byte
//                              little-endian {ptr, len} descriptor; the
//                              items are a contiguous run of 36-byte
//                              nodes. The result is guest-allocated and
//                              must be returned to the guest with
//                              `cabi_post_view(desc)`.
//   changes() -> list<mut.>  : same shape; the items are 44-byte
//                              mutations and the buffer is returned with
//                              `cabi_post_changes(desc)`.
//   on-click(id: u32)        : `on-click(i32)` — passes directly.
//   on-text(id: u32, text)   : `on-text(i32, i32, i32)` — the id, then the
//                              string's pointer + length, in the *guest's*
//                              memory (the canonical ABI). The host has no
//                              memory of its own, so `text()` allocates the
//                              buffer through the guest's `cabi_realloc` and
//                              writes to it (see the `text` doc for why the
//                              buffer must come from the guest's allocator).
//   on-tick()                : `on-tick()`.
//   on-double-click(id: u32) : `on-double-click(i32)` — passes directly.
//   on-window-close(w: u32)  : `on-window-close(i32)` — passes directly.
//   schedule(p: u64)         : `$root"."schedule"`, `(i64)` — the
//                              component's refresh request in ms (0 = no
//                              periodic wake). Delivered to the
//                              `onSchedule` hook, if `createApp` was
//                              passed one; a no-op without it.
//   now() -> u64             : imported as `$root"."now"`, `() -> i64`
//                              (JS BigInt). Provided unconditionally; the
//                              counter never calls it.
//   list-demos() -> list<demo>
//                              : `$root"."list-demos"`, `(i32 retPtr)` —
//                              the guest passes a return pointer; the host
//                              writes the 8-byte {ptr, len} descriptor to
//                              it, with the demo records in the guest's
//                              memory. Each record is 40 bytes: four
//                              (ptr, len) strings, then two u32 sizes. The
//                              registry comes from the `registry` hook;
//                              empty without it.
//   open-window(src, title,
//               x, y, w, h) -> result<u32, error>
//                              : `$root"."open-window"`, the two strings'
//                              pointer + length (in the guest's memory),
//                              the four u32s, then the return pointer. The
//                              result is an 8-byte variant written to it:
//                              byte 0 is the case (0 = Ok, 1 = Err), and
//                              the u32 at byte 4 is the handle (Ok) or the
//                              error tag (Err). Delivered to the
//                              `onOpenWindow` hook; without it the result
//                              is `not-found`.
//
// The 36-byte node (all little-endian):
//   bytes [0]       variant tag: 0 = text, 1 = button, 2 = container,
//                   3 = input, 4 = image (the WIT case order in waui:ui/widget)
//   bytes [1..4]    padding
//   bytes [4]       id option flag: 0 = none (an unaddressable node)
//   bytes [5..8]    padding
//   bytes [8..12]   id: u32 (valid only when the flag is set)
//   bytes [12]      parent option flag: 0 = none
//   bytes [13..16]  padding
//   bytes [16..20]  parent id: u32 (valid only when the flag is set)
//   bytes [20]      container: direction (u8, 0 = row, 1 = column)
//   bytes [21]      container: align (u8, 0 = start, 1 = center, 2 = end)
//   bytes [20..24]  text/button/input/image: string pointer
//   bytes [24..28]  text/button/input/image: string length (UTF-8 bytes)
//   bytes [28]      text/button/input: style.color (u8, WIT case order in
//                   waui:ui/widget: 0 = neutral, 1 = accent, 2 = positive,
//                   3 = negative, 4 = warning)
//   bytes [29]      text/button/input: style.size (u8: 0 = small,
//                   1 = medium, 2 = large)
//   bytes [30..32]  padding (text/button/input)
//   bytes [28..32]  image: width (u32, host units; 0 = the host's default)
//   bytes [32..36]  input: width (u32, characters; 0 = the host's default)
//                   image: height (u32, host units; 0 = the host's default)
//
// The 44-byte mutation (a variant with a record- or u32-sized payload):
//   bytes [0]       tag: 0 = changed, 1 = removed (one byte, like the node
//                   tag — the canonical ABI sizes a variant's discriminant
//                   to the case count: u8 for 256 or fewer cases)
//   bytes [1..4]    padding
//   changed   : the 36-byte node above at byte 4, then the node's index
//               (a u32, its 0-based position among its parent's children)
//               at byte 40
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

// The world's imports (now, schedule, list-demos, open-window), as the
// pre-lift module expects. `hooks.onSchedule` arms the host's wake loop (see
// app.js); a no-op without it. `hooks.registry` is the demo registry
// `list-demos` returns; empty without it. `hooks.onOpenWindow(src, title,
// x, y, w, h)` opens a window and returns its handle; `null` (or its
// absence) reports `not-found`.
//
// `getInstance` yields the instance once it exists: the import functions
// run only after instantiation, but the import object is built before it,
// so the instance is fetched lazily, not captured at construction.
function appImports(hooks = {}, getInstance) {
  const enc = new TextEncoder();
  const dec = new TextDecoder();
  const registry = hooks.registry ?? [];
  return {
    $root: {
      // The world's `now: func() -> u64` import, in host milliseconds.
      now: () => BigInt(Date.now()),
      // The world's `schedule: func(u64)` import: the component's refresh
      // request, in ms; 0 = no periodic wake.
      schedule: (p) => {
        if (hooks.onSchedule) hooks.onSchedule(Number(p));
      },
      // The world's `list-demos: func() -> list<demo>` import. The guest
      // passes a return pointer; the host writes the 8-byte {ptr, len}
      // descriptor to it, with the demo records in the guest's memory.
      // Each record is 40 bytes: four (ptr, len) strings, then two u32
      // sizes.
      'list-demos': (retPtr) => {
        const ex = getInstance().exports;
        const allocString = (s) => {
          const b = enc.encode(s);
          // `cabi_realloc(old_ptr, old_len, align, new_len)`: the align comes
          // before the new length in the 0.63 ABI. A fresh alloc is
          // (0, 0, align, len) — passing (0, 0, len, align) would size the
          // block by the alignment and overflow it.
          const ptr = ex.cabi_realloc(0, 0, 1, b.length);
          const u8 = new Uint8Array(ex.memory.buffer);
          u8.set(b, ptr);
          return [ptr, b.length];
        };
        const n = registry.length;
        // The record array: 40 bytes each, 4-aligned. Same (align, len) order.
        const arrPtr = ex.cabi_realloc(0, 0, 4, n * 40);
        for (let k = 0; k < n; k++) {
          const d = registry[k];
          const [nameP, nameL] = allocString(d.name);
          const [langP, langL] = allocString(d.lang);
          const [iconP, iconL] = allocString(d.icon);
          const [srcP, srcL] = allocString(d.src);
          // The string allocs may have grown the memory: rebuild the view.
          const i32 = new Int32Array(ex.memory.buffer);
          const o = arrPtr + k * 40;
          i32[o >> 2] = nameP;
          i32[(o + 4) >> 2] = nameL;
          i32[(o + 8) >> 2] = langP;
          i32[(o + 12) >> 2] = langL;
          i32[(o + 16) >> 2] = iconP;
          i32[(o + 20) >> 2] = iconL;
          i32[(o + 24) >> 2] = srcP;
          i32[(o + 28) >> 2] = srcL;
          i32[(o + 32) >> 2] = d.w;
          i32[(o + 36) >> 2] = d.h;
        }
        const i32 = new Int32Array(ex.memory.buffer);
        i32[retPtr >> 2] = arrPtr;
        i32[(retPtr + 4) >> 2] = n;
      },
      // The world's `open-window: func(src, title, x, y, w, h) ->
      // result<u32, error>` import. The strings are in the guest's memory.
      // The result is an 8-byte variant (a discriminant byte, then a u32)
      // written to the return pointer: byte 0 is the case (0 = Ok, 1 = Err),
      // and the u32 at byte 4 is the handle (Ok) or the error tag (Err).
      'open-window': (srcPtr, srcLen, titlePtr, titleLen, x, y, w, h, retPtr) => {
        const ex = getInstance().exports;
        const u8 = new Uint8Array(ex.memory.buffer);
        const src = dec.decode(u8.subarray(srcPtr, srcPtr + srcLen));
        const title = dec.decode(u8.subarray(titlePtr, titlePtr + titleLen));
        // The windowing layer opens the window; it returns the handle, or
        // null if it cannot run the demo.
        const handle = hooks.onOpenWindow ? hooks.onOpenWindow(src, title, x, y, w, h) : null;
        const i32 = new Int32Array(ex.memory.buffer);
        if (handle === null || handle === undefined) {
          u8[retPtr] = 1; // Err
          i32[(retPtr + 4) >> 2] = 0; // not-found
        } else {
          u8[retPtr] = 0; // Ok
          i32[(retPtr + 4) >> 2] = handle;
        }
      },
    },
  };
}

// The world's ABI layer over an instance's exports: the `view`/`changes`
// decoders and the event callers. Shared by `createApp` (async, from bytes)
// and `instantiateApp` (sync, from a pre-compiled module).
// The semantic tokens each host maps onto its own palette, in the WIT case
// order of waui:ui/widget (unknown values degrade safely).
const COLORS = ['neutral', 'accent', 'positive', 'negative', 'warning'];
const SIZES = ['small', 'medium', 'large'];
const ALIGNS = ['start', 'center', 'end'];

// Decode one 36-byte node record at byte offset `o`.
function decodeNode(i32, u8, o) {
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
  // An image has no style: its [28..36) is width + height, not the style
  // bytes, so it branches before the style read. The string slot is `src`.
  if (tag === 4) {
    return {
      kind: 'image',
      id,
      parent,
      src: s,
      width: i32[(o + 28) >> 2],
      height: i32[(o + 32) >> 2],
    };
  }
  const style = {
    color: COLORS[u8[o + 28]] ?? 'neutral',
    size: SIZES[u8[o + 29]] ?? 'medium',
  };
  return tag === 1
    ? { kind: 'button', id, parent, label: s, style }
    : tag === 3
      ? { kind: 'input', id, parent, content: s, style, width: i32[(o + 32) >> 2] }
      : { kind: 'text', id, parent, content: s, style };
}

// Decode a `list` result: an 8-byte {ptr, len} descriptor, then `len` items
// of `stride` bytes, then hand the guest-allocated buffer back. `mem` is the
// host's view of the guest's linear memory (a function, so it re-reads after
// a grow).
function readList(mem, call, post, stride, item) {
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
}

function makeAppApi(ex) {
  // `memory.grow` detaches the previous ArrayBuffer, so typed views are
  // created fresh inside each call, after the guest has settled.
  const mem = () => ex.memory.buffer;
  const enc = new TextEncoder();
  // The AssemblyScript scratch region's capacity (the top 4 KiB of page 0,
  // a bump allocator through `cabi_realloc`). `text()` clamps an over-long
  // buffer to it so the AssemblyScript demo (bounded region) and the Rust
  // demo (unbounded heap) stay byte-parity.
  const SCRATCH_CAP = 4096;

  return {
    /// Decode `view()`'s result into a flat list of node objects.
    view: () => readList(mem, ex.view, ex.cabi_post_view, 36, decodeNode),

    /// Decode `changes()`'s result into a list of mutation objects:
    /// { kind: 'changed', node, index } or { kind: 'removed', id }.
    changes: () =>
      readList(mem, ex.changes, ex.cabi_post_changes, 44, (i32, u8, o) => {
        // The tag is one byte (see the layout note above); the padding after
        // it is unwritten, so it must not be read as part of the tag.
        return u8[o] === 1
          ? { kind: 'removed', id: i32[(o + 4) >> 2] }
          : {
              kind: 'changed',
              node: decodeNode(i32, u8, o + 4),
              index: i32[(o + 40) >> 2],
            };
      }),

    click: (id) => ex['on-click'](id),
    tick: () => ex['on-tick'](),

    /// The world's `on-double-click` export: the node with `id` was
    /// double-activated.
    doubleClick: (id) => ex['on-double-click'](id),

    /// The world's `on-window-close` export: the window with `handle` was
    /// closed by the user.
    windowClose: (handle) => ex['on-window-close'](handle),

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
          // `cabi_realloc(old_ptr, old_len, align, new_len)`: the align comes
          // before the new length in the 0.63 ABI, so a fresh alloc is
          // (0, 0, 1, len).
          const ptr = ex.cabi_realloc(0, 0, 1, bytes.length);
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

// Instantiate a component from its bytes: the standalone demo pages, the
// desktop page, and the Node selftest boot this way. `wasmBytes` may be a
// promise of the bytes (the site's fetch boot); awaiting a non-promise is the
// identity, so the inlined form works too. The hooks are as in
// `appImports`; the windowing ones (`registry`, `onOpenWindow`) are used by
// the desktop, ignored by the plain demos.
export async function createApp(wasmBytes, hooks = {}) {
  const bytes = await wasmBytes;
  let instance;
  const imports = appImports(hooks, () => instance);
  ({ instance } = await WebAssembly.instantiate(bytes, imports));
  return makeAppApi(instance.exports);
}

// A synchronous instantiation of a component from an already compiled module.
// The windowing host pre-compiles its demos at startup, then opens a window
// with this: the `open-window` import is a sync wasm call, so the
// instantiation must be sync too (`WebAssembly.instantiate` is not).
export function instantiateApp(compiledModule, hooks = {}) {
  // `new WebAssembly.Instance` returns the instance directly (unlike the async
  // `WebAssembly.instantiate`, which resolves to { module, instance }).
  let instance;
  const imports = appImports(hooks, () => instance);
  instance = new WebAssembly.Instance(compiledModule, imports);
  return makeAppApi(instance.exports);
}
