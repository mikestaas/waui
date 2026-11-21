// The world's windowing imports, in raw canonical-ABI form — the
// `list-entries`, `open-entry`, `close-entry`, and `viewport` of the `app`
// world (see `root.ts` for the clock imports, `now` and `schedule`).
//
// A world-level import lives in the pseudo-module `$root`, so each is exactly
// one `@external`. Only components that run *other* components (a window
// manager) import this file; a demo that opens no windows never calls any of
// them, and
// the unused imports are tree-shaken, as rustc drops the Rust side's unused
// generated imports.
//
// `list-entries() -> list<entry>`: the guest passes a return pointer (an
// 8-byte {ptr, len} descriptor in the guest's memory); the host writes the
// entry records there, in the guest's memory. Each record is 16 bytes: two
// (ptr, len) strings (the name, the `data`), in the guest's memory (the host
// allocates them through the guest's `cabi_realloc`).
//
// `open-entry(name, x, y, w, h) -> result<u32, error>`: the name is a (ptr,
// len) string in the guest's memory (the guest allocates it); the four u32s;
// then a return pointer (8 bytes in the guest's memory). The result is an
// 8-byte variant written to it: byte 0 is the case (0 = Ok, 1 = Err), and
// the u32 at byte 4 is the handle (Ok) or the error tag (Err).
//
// `close-entry(handle)`: one u32. The host drops the sandbox it opened for
// `handle`; idempotent, so a double close is a no-op.
//
// `viewport() -> viewport-size`: the guest passes a return pointer (8 bytes
// in the guest's memory); the host writes the size there as two u32s (w at
// byte 0, h at byte 4). A host with no viewport writes (0, 0), which the
// guest treats as unknown.

@external('$root', 'list-entries')
declare function listEntries(retPtr: i32): void;


@external('$root', 'open-entry')
declare function openEntry(
  namePtr: i32,
  nameLen: i32,
  x: i32,
  y: i32,
  w: i32,
  h: i32,
  retPtr: i32,
): void;


@external('$root', 'close-entry')
declare function closeEntry(handle: i32): void;


@external('$root', 'viewport')
declare function viewport(retPtr: i32): void;

export { listEntries, openEntry, closeEntry, viewport };
