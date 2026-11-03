// Byte-oriented string/number converters: the canonical ABI encodes strings
// as UTF-8 byte runs in linear memory, while AssemblyScript strings are
// UTF-16. The writers go straight into arena memory — no AS heap allocation
// (which would require a runtime), just `store<i8>` loops.
//
// Each writer returns the *cursor* — the position after what it wrote — so
// writes compose:
//
//   const str = arenaAlloc(24);
//   let p = writeInt64(str, elapsed);
//   p = writeUtf8(p, ' ms');
//   emitText(rec, id, parent, str, p - str, color, size);
//
// `readUtf8` goes the other way: a UTF-8 byte run in linear memory (the
// `on-text` string the web host writes to the scratch region) into an AS
// string.

import { arenaAlloc } from './arena';

/// Encode `s` as UTF-8 (surrogate pairs included) at `dst`, writing at
/// most until `end`. A character that would not fit *whole* stops the
/// write — no partial multi-byte sequences. The default bound (i32 max)
/// makes `end` opt-in; the unbounded callers are unchanged.
export function writeUtf8(dst: i32, s: string, end: i32 = 2147483647): i32 {
  let p = dst;
  const n = s.length;
  for (let i = 0; i < n; i++) {
    let c = s.charCodeAt(i);
    if (c < 0x80) {
      if (p + 1 > end) break;
      store<i8>(p++, c);
    } else if (c < 0x800) {
      if (p + 2 > end) break;
      store<i8>(p++, 0xc0 | (c >> 6));
      store<i8>(p++, 0x80 | (c & 0x3f));
    } else if (c >= 0xd800 && c < 0xdc00 && i + 1 < n) {
      if (p + 4 > end) break;
      const lo = s.charCodeAt(++i);
      c = (0x10000 + ((c - 0xd800) << 10)) | (lo - 0xdc00);
      store<i8>(p++, 0xf0 | (c >> 18));
      store<i8>(p++, 0x80 | ((c >> 12) & 0x3f));
      store<i8>(p++, 0x80 | ((c >> 6) & 0x3f));
      store<i8>(p++, 0x80 | (c & 0x3f));
    } else {
      if (p + 3 > end) break;
      store<i8>(p++, 0xe0 | (c >> 12));
      store<i8>(p++, 0x80 | ((c >> 6) & 0x3f));
      store<i8>(p++, 0x80 | (c & 0x3f));
    }
  }
  return p;
}

/// Decode a UTF-8 byte run at `[ptr, ptr + len)` (surrogate pairs included)
/// into an AS string. A truncated final sequence (a partial multi-byte
/// character) stops the decode, like the writers.
export function readUtf8(ptr: i32, len: i32): string {
  let s = '';
  let i = 0;
  while (i < len) {
    const c = (load<i8>(ptr + i) as u32) & 0xff;
    if (c < 0x80) {
      s += String.fromCharCode(c as i32);
      i += 1;
    } else if (c < 0xe0) {
      if (i + 2 > len) break;
      const c2 = (load<i8>(ptr + i + 1) as u32) & 0x3f;
      s += String.fromCharCode((((c & 0x1f) << 6) | c2) as i32);
      i += 2;
    } else if (c < 0xf0) {
      if (i + 3 > len) break;
      const c2 = (load<i8>(ptr + i + 1) as u32) & 0x3f;
      const c3 = (load<i8>(ptr + i + 2) as u32) & 0x3f;
      s += String.fromCharCode((((c & 0x0f) << 12) | (c2 << 6) | c3) as i32);
      i += 3;
    } else {
      if (i + 4 > len) break;
      const c2 = (load<i8>(ptr + i + 1) as u32) & 0x3f;
      const c3 = (load<i8>(ptr + i + 2) as u32) & 0x3f;
      const c4 = (load<i8>(ptr + i + 3) as u32) & 0x3f;
      const cp = ((c & 0x07) << 18) | (c2 << 12) | (c3 << 6) | c4;
      const v = cp - 0x10000;
      s += String.fromCharCode(0xd800 + (v >> 10), 0xdc00 + (v & 0x3ff));
      i += 4;
    }
  }
  return s;
}

// The absolute value is computed by *unsigned* negation (modular): for
// the most negative value this wraps to its correct magnitude, and for
// every other value it equals the ordinary absolute value — so no
// special case (and no min-value literal, which AS cannot spell) is
// needed.

/// Write `v` as decimal ASCII at `dst`, at most until `end`. A number
/// that would not fit whole is not written at all (a truncated magnitude
/// is a different number). The default bound makes `end` opt-in.
export function writeInt32(dst: i32, v: i32, end: i32 = 2147483647): i32 {
  let p = dst;
  let t: u32;
  let minus: i32;
  if (v < 0) {
    minus = 1;
    t = 0 - changetype<u32>(v);
  } else {
    minus = 0;
    t = changetype<u32>(v);
  }
  // Digits in reverse into arena scratch (10 bytes fits any u32; 16 after
  // 8-byte alignment), then emitted reversed.
  const scratch = arenaAlloc(16);
  let count = 0;
  do {
    store<i8>(scratch + count++, 48 + <i32>(t % 10));
    t = t / 10;
  } while (t != 0);
  if (p + minus + count > end) return p;
  if (minus != 0) store<i8>(p++, 45 /* '-' */);
  for (let i = count - 1; i >= 0; i--) {
    store<i8>(p++, load<i8>(scratch + i));
  }
  return p;
}

/// Write `v` (any i64) as decimal ASCII at `dst`, at most until `end`
/// (same whole-number rule as `writeInt32`).
export function writeInt64(dst: i32, v: i64, end: i32 = 2147483647): i32 {
  let p = dst;
  let t: u64;
  let minus: i32;
  if (v < 0) {
    minus = 1;
    t = 0 - changetype<u64>(v);
  } else {
    minus = 0;
    t = changetype<u64>(v);
  }
  // Digits in reverse into arena scratch (20 bytes fits any u64; 32 after
  // 8-byte alignment), then emitted reversed.
  const scratch = arenaAlloc(32);
  let count = 0;
  do {
    store<i8>(scratch + count++, 48 + <i32>(t % 10));
    t = t / 10;
  } while (t != 0);
  if (p + minus + count > end) return p;
  if (minus != 0) store<i8>(p++, 45 /* '-' */);
  for (let i = count - 1; i >= 0; i--) {
    store<i8>(p++, load<i8>(scratch + i));
  }
  return p;
}
