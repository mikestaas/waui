;; The desktop, in hand-rolled WAT on the WASM GC — the same component as
;; `crates/demo-desktop` and `assembly/demo-desktop`, same `waui:ui/app`
;; world. See the sibling `demo-counter.wat` for the shared scaffolding:
;; the $flat struct and $flats array (the contract's flat, pre-ordered
;; node list, on the GC heap), the two-page memory (result arena, host
;; scratch, static literals), and the canonical-ABI records.
;;
;; The just-an-app window manager: the whole interface is this component's
;; tree — the menu bar, the icon grid, and the open windows (their chrome,
;; their geometry, their stacking order). It draws the windows itself with
;; the contract's geometry tokens, and drives them through the pointer and
;; key channels: a press raises a window, a title-bar drag moves it, a grip
;; drag resizes it, the close marker and Escape close it, and a
;; double-clicked tile opens its entry.
;;
;; The *content* of an opened window is not this component's: it is a
;; separate component the host runs in its own sandbox. On a double-click
;; this component calls `open-entry` (the host instantiates the entry's
;; component and returns a handle) and draws a window whose content slot is
;; that handle: the host seats the opened component's tree there. Closing a
;; window calls `close-entry`, so the host drops the sandbox. This
;; component only ever sees its own tree; the opened component's tree is
;; the host's to render.
;;
;; The registry is fetched once from `list-entries`, on the first tree
;; build: each entry's name plus its parsed `data` blob (a JSON object of
;; host-defined metadata: the tile's label, caption, icon, and the window's
;; default size). A field the blob lacks is rendered without, and a blob
;; that will not parse renders the name only. The host writes the registry
;; into the module's scratch region (through `cabi_realloc`), so the
;; component copies it into GC byte arrays and frees the scratch.

(module

  ;; The world's windowing imports, in raw canonical-ABI form.
  ;; `list-entries() -> list<entry>`: the guest passes a return pointer (an
  ;; 8-byte {ptr, len} descriptor in the guest's memory); the host writes
  ;; the entry records there, in the guest's memory. Each record is 16
  ;; bytes: two (ptr, len) strings (the name, the `data`).
  (import "$root" "list-entries" (func $list-entries (param $ret-ptr i32)))
  ;; `open-entry(name, x, y, w, h) -> result<u32, error>`: the name is a
  ;; (ptr, len) string in the guest's memory (the guest allocates it); the
  ;; four u32s; then a return pointer (8 bytes in the guest's memory). The
  ;; result is an 8-byte variant written to it: byte 0 is the case
  ;; (0 = Ok, 1 = Err), and the u32 at byte 4 is the handle or the error.
  (import "$root" "open-entry"
    (func $open-entry (param i32) (param i32) (param i32)
      (param i32) (param i32) (param i32) (param i32)))
  ;; `close-entry(handle)`: the host drops the sandbox it is running.
  (import "$root" "close-entry" (func $close-entry (param $handle i32)))
  ;; `viewport() -> viewport-size`: the guest passes a return pointer (an
  ;; 8-byte {w, h} region in the guest's memory); the host writes the size
  ;; there, in the guest's memory. A host with no viewport writes (0, 0).
  (import "$root" "viewport" (func $viewport (param $ret-ptr i32)))

  (memory (export "memory") 2)

  ;; A label, as UTF-8 bytes on the GC heap. Each byte is held as an i32
  ;; (zero-extended): array.get returns an i32 for i32.store8, and the element
  ;; is mutable so array.set can fill it.
  (type $bytes (array (mut i32)))

  ;; One node of the export, as the contract's flat record plus its label:
  ;; the scalars mirror the ABI record's fields, the label is a GC byte
  ;; array (null = none). The contract's tree is a flat, pre-ordered list,
  ;; so the list is the tree.
  (type $flat (struct
    (field $kind i32)          ;; 0 text, 1 button, 2 container, 3 input, 4 image, 5 link
    (field $id i32)            ;; 0 = none
    (field $parent i32)        ;; 0 = none
    (field $color i32)         ;; the WIT color token
    (field $size i32)          ;; the WIT size token
    (field $direction i32)     ;; containers: 0 row, 1 column
    (field $align i32)         ;; containers: 0 start, 1 center, 2 end
    (field $width i32)         ;; inputs: characters; images and containers: host units
    (field $height i32)        ;; images and containers: host units (0 = the host's default)
    (field $positioned i32)    ;; containers: 1 = the x/y are relative to the parent
    (field $x i32)             ;; containers: the host's units
    (field $y i32)             ;; containers: the host's units
    (field $z i32)             ;; containers: the stacking order (0 = none)
    (field $gap i32)           ;; containers: the child gap (0 = the host's default)
    (field $padding i32)       ;; containers: the host's units
    (field $stretch i32)       ;; containers: 0 none, 1 main, 2 cross, 3 both
    (field $overflow i32)      ;; containers: 0 clip, 1 scroll
    (field $background i32)    ;; containers: 0 none, 1 panel, 2 bar
    (field $border i32)        ;; containers: 0 none, 1 line, 2 ring
    (field $radius i32)        ;; containers: 0 none, 1 small, 2 large
    (field $shadow i32)        ;; containers: 0 none, 1 raised
    (field $cursor i32)        ;; containers: 0 default, 1 pointer, 2 grab, 3 grabbing, 4 resize
    (field $label (ref null $bytes))
    (field $href (ref null $bytes))  ;; links: the URL the host opens (null = none)
    (field $index i32)         ;; position among the parent's children
  ))
  (type $flats (array (mut (ref null $flat))))

  ;; A registry entry, as retained state: the name (the open's identifier)
  ;; and the parsed `data` (the tile's presentation and the window's default
  ;; size). A null label/lang/icon is the fallback (the name / absent).
  (type $entry (struct
    (field $name (ref null $bytes))
    (field $label (mut (ref null $bytes)))
    (field $lang (mut (ref null $bytes)))
    (field $icon (mut (ref null $bytes)))
    (field $width (mut i32))
    (field $height (mut i32))
  ))
  (type $entries (array (mut (ref null $entry))))

  ;; An open window: the host handle (also the content slot's id), the
  ;; entry's presentation, and the window's geometry and stacking order.
  (type $win (struct
    (field $handle i32)
    (field $label (ref null $bytes))
    (field $icon (ref null $bytes))
    (field $x (mut i32))
    (field $y (mut i32))
    (field $w (mut i32))
    (field $h (mut i32))
    (field $z (mut i32))
  ))
  (type $wins (array (mut (ref null $win))))

  ;; The in-progress drag or resize: kind 0 moves the window (keeping the
  ;; grab offset dx/dy), kind 1 grows it from its origin (the start size).
  ;; State: the registry (null = not fetched yet), the open windows, the
  ;; next z to assign, the new-window placement counter, the selected tile
  ;; (-1 = none), the in-progress drag, the last viewport query, and the
  ;; last export (the diff's reference).
  ;;
  ;; The drag is plain globals, not a struct on the heap: a heap object
  ;; held only by a global is reclaimed by the collector the moment a later
  ;; allocation compacts, which dropped the resize mid-press. Scalars in
  ;; globals are roots the collector leaves alone.
  (global $entries (mut (ref null $entries)) (ref.null $entries))
  (global $wins (mut (ref null $wins)) (ref.null $wins))
  (global $win-len (mut i32) (i32.const 0))
  (global $next-z (mut i32) (i32.const 10))
  (global $cascade (mut i32) (i32.const 0))
  (global $selected (mut i32) (i32.const -1))
  (global $drag-active (mut i32) (i32.const 0))
  (global $drag-kind (mut i32) (i32.const 0)) ;; 0 move, 1 resize
  (global $drag-handle (mut i32) (i32.const 0))
  (global $drag-dx (mut i32) (i32.const 0))
  (global $drag-dy (mut i32) (i32.const 0))
  (global $drag-sx (mut i32) (i32.const 0))
  (global $drag-sy (mut i32) (i32.const 0))
  (global $drag-sw (mut i32) (i32.const 0))
  (global $drag-sh (mut i32) (i32.const 0))
  (global $vp-w (mut i32) (i32.const 0))
  (global $vp-h (mut i32) (i32.const 0))
  (global $last (mut (ref null $flats)) (ref.null $flats))

  ;; The result arena's cursor (8-aligned by construction).
  (global $arena (mut i32) (i32.const 0))
  ;; The host scratch region's cursor (the top 4 KiB of page 0).
  (global $scratch (mut i32) (i32.const 61440))
  ;; The JSON parser's cursor (the `data` blob is parsed in place).
  (global $p-cur (mut i32) (i32.const 0))

  ;; The string literals, from page 1.
  (data (i32.const 65536) "waui")
  (data (i32.const 65540) " window")
  (data (i32.const 65547) "s")
  (data (i32.const 65548) "label")
  (data (i32.const 65553) "lang")
  (data (i32.const 65557) "icon")
  (data (i32.const 65561) "width")
  (data (i32.const 65566) "height")
  (data (i32.const 65572) "Escape")
  (data (i32.const 65579) "Tab")
  (data (i32.const 65582) "Shift+Tab")
  (data (i32.const 65591) "\E2\9C\95") ;; the close glyph (U+2715)
  (data (i32.const 65594) "wasm ui demos")
  (data (i32.const 65607) "double-click an icon to open")
  (data (i32.const 65635) "GitHub")
  (data (i32.const 65641) "https://github.com/mikestaas/waui")

  ;; ================================================================ the ABI

  ;; Bump-allocate n bytes, 8-aligned, from the result arena.
  (func $arena-alloc (param $n i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $arena))
    (global.set $arena
      (i32.and
        (i32.add (i32.add (global.get $arena) (local.get $n)) (i32.const 7))
        (i32.const -8)))
    (local.get $p)
  )

  ;; The canonical-ABI post functions: the host calls one after consuming a
  ;; list result. Every byte of a result comes from the arena, so one reset
  ;; of the cursor is the whole free.
  (func (export "cabi_post_view") (param $desc i32)
    (global.set $arena (i32.const 0))
  )
  (func (export "cabi_post_changes") (param $desc i32)
    (global.set $arena (i32.const 0))
  )

  ;; Round n up to a (power-of-two) alignment.
  (func $round-up (param $n i32) (param $a i32) (result i32)
    (local $m i32)
    (local.set $m (i32.sub (local.get $a) (i32.const 1)))
    (i32.and (i32.add (local.get $n) (local.get $m)) (i32.xor (local.get $m) (i32.const -1)))
  )

  ;; x != 0. A binary compare: some wasm-tools WAT parsers lack the unary form.
  (func $nez (param $x i32) (result i32)
    (i32.ne (local.get $x) (i32.const 0))
  )

  ;; The canonical ABI's cabi_realloc: a buffer in the module's own memory,
  ;; for a string parameter. The parameter order is the canonical ABI's:
  ;; (old_ptr, old_len, align, new_len) — the align comes before the new
  ;; length (a fresh alloc is (0, 0, align, len); a free is (ptr, len,
  ;; align, 0)); the hosts call it in that order. A bump over the scratch
  ;; region: one buffer is live at a time (the host allocates, the callee
  ;; frees), so a free returns the region to its base. A buffer past the
  ;; region traps.
  (func $cabi-realloc (export "cabi_realloc")
    (param $old-ptr i32) (param $old-len i32) (param $align i32) (param $new-len i32)
    (result i32)
    (local $a i32)
    (local $p i32)
    (local $sz i32)
    (local $base i32)
    (local $end i32)
    (local.set $base (i32.const 61440))
    (local.set $end (i32.const 65536))
    (local.set $a (if (result i32) (i32.gt_s (local.get $align) (i32.const 0))
      (then (local.get $align))
      (else (i32.const 1))))
    (if (i32.le_s (local.get $new-len) (i32.const 0))
      (then
        (global.set $scratch (local.get $base))
        (return (i32.const 0))
      ))
    (if (i32.and
          (i32.ge_s (local.get $old-ptr) (local.get $base))
          (i32.and
            (i32.lt_s (local.get $old-ptr) (local.get $end))
            (i32.eq
              (i32.add (local.get $old-ptr) (call $round-up (local.get $old-len) (local.get $a)))
              (global.get $scratch))))
      (then
        (local.set $sz (call $round-up (local.get $new-len) (local.get $a)))
        (if (i32.gt_s (i32.add (local.get $old-ptr) (local.get $sz)) (local.get $end))
          (then (unreachable)))
        (global.set $scratch (i32.add (local.get $old-ptr) (local.get $sz)))
        (return (local.get $old-ptr))
      ))
    (local.set $p (call $round-up (global.get $scratch) (local.get $a)))
    (local.set $sz (call $round-up (local.get $new-len) (local.get $a)))
    (if (i32.gt_s (i32.add (local.get $p) (local.get $sz)) (local.get $end))
      (then (unreachable)))
    (global.set $scratch (i32.add (local.get $p) (local.get $sz)))
    (local.get $p)
  )

  ;; A list result in the canonical ABI: a pointer to an 8-byte {ptr, len}
  ;; descriptor in the module's memory.
  (func $make-list (param $buf i32) (param $n i32) (result i32)
    (local $d i32)
    (local.set $d (call $arena-alloc (i32.const 8)))
    (i32.store (local.get $d) (local.get $buf))
    (i32.store (i32.add (local.get $d) (i32.const 4)) (local.get $n))
    (local.get $d)
  )

  ;; The host's viewport, into $vp-w/$vp-h. The return buffer is
  ;; arena-allocated (the canonical ABI's return region); it dies with the
  ;; next post. A host with no viewport reports (0, 0), which the callers
  ;; treat as unknown and skip viewport-dependent math for.
  (func $refresh-viewport
    (local $ret i32)
    (local.set $ret (call $arena-alloc (i32.const 8)))
    (call $viewport (local.get $ret))
    (global.set $vp-w (i32.load (local.get $ret)))
    (global.set $vp-h (i32.load (i32.add (local.get $ret) (i32.const 4))))
  )

  ;; ======================================================== labels and text

  ;; A static literal, as a fresh GC byte array: the bytes are copied out of
  ;; the data segment.
  (func $label (param $d i32) (param $len i32) (result (ref $bytes))
    (local $a (ref $bytes))
    (local $i i32)
    (local.set $a (array.new_default $bytes (local.get $len)))
    (block $done
      (loop $lp
        (br_if $done (i32.ge_u (local.get $i) (local.get $len)))
        (array.set $bytes (local.get $a) (local.get $i)
          (i32.load8_u (i32.add (local.get $d) (local.get $i))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $lp)
      ))
    (local.get $a)
  )

  ;; A run of bytes in linear memory (the host's scratch region), as a fresh
  ;; GC byte array.
  (func $scratch-to-bytes (param $ptr i32) (param $len i32) (result (ref $bytes))
    (local $a (ref $bytes))
    (local $i i32)
    (local.set $a (array.new_default $bytes (local.get $len)))
    (block $done
      (loop $lp
        (br_if $done (i32.ge_u (local.get $i) (local.get $len)))
        (array.set $bytes (local.get $a) (local.get $i)
          (i32.load8_u (i32.add (local.get $ptr) (local.get $i))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $lp)
      ))
    (local.get $a)
  )

  ;; Copy the node's label bytes into the arena; store the (ptr, len) at the
  ;; record's string slot (+20/+24). A null or empty label stores (0, 0).
  (func $emit-label (param $rec i32) (param $label (ref null $bytes))
    (local $a (ref $bytes))
    (local $len i32)
    (local $i i32)
    (local $dst i32)
    (local.set $dst (i32.const 0))
    (local.set $len (i32.const 0))
    (if (ref.is_null (local.get $label))
      (then)
      (else
        (local.set $a (ref.cast (ref $bytes) (local.get $label)))
        (local.set $len (array.len (local.get $a)))
        (if (i32.eqz (local.get $len))
          (then)
          (else
            (local.set $dst (call $arena-alloc (local.get $len)))
            (block $done
              (loop $lp
                (br_if $done (i32.ge_u (local.get $i) (local.get $len)))
                (i32.store8 (i32.add (local.get $dst) (local.get $i))
                  (array.get $bytes (local.get $a) (local.get $i)))
                (local.set $i (i32.add (local.get $i) (i32.const 1)))
                (br $lp)
              ))
          ))
      ))
    (i32.store (i32.add (local.get $rec) (i32.const 20)) (local.get $dst))
    (i32.store (i32.add (local.get $rec) (i32.const 24)) (local.get $len))
  )

  ;; Copy a link's href bytes into the arena; store the (ptr, len) at the
  ;; record's href slot (+28/+32). A twin of $emit-label, whose slot is
  ;; +20/+24. A null or empty href stores (0, 0).
  (func $emit-href (param $rec i32) (param $href (ref null $bytes))
    (local $a (ref $bytes))
    (local $len i32)
    (local $i i32)
    (local $dst i32)
    (local.set $dst (i32.const 0))
    (local.set $len (i32.const 0))
    (if (ref.is_null (local.get $href))
      (then)
      (else
        (local.set $a (ref.cast (ref $bytes) (local.get $href)))
        (local.set $len (array.len (local.get $a)))
        (if (i32.eqz (local.get $len))
          (then)
          (else
            (local.set $dst (call $arena-alloc (local.get $len)))
            (block $done
              (loop $lp
                (br_if $done (i32.ge_u (local.get $i) (local.get $len)))
                (i32.store8 (i32.add (local.get $dst) (local.get $i))
                  (array.get $bytes (local.get $a) (local.get $i)))
                (local.set $i (i32.add (local.get $i) (i32.const 1)))
                (br $lp)
              ))
          ))
      ))
    (i32.store (i32.add (local.get $rec) (i32.const 28)) (local.get $dst))
    (i32.store (i32.add (local.get $rec) (i32.const 32)) (local.get $len))
  )

  ;; =========================================================== the records

  ;; The 60-byte node record for f at rec. Every byte a host reads is
  ;; written: the tag, both option flags (0 = none), the id and parent words
  ;; (when flagged), and the per-kind payload. An image's [28..36) is the
  ;; width and the height; a link's [28..36) is the href, its style at
  ;; [36]/[37]; a container's [20..59) is its 17 record fields.
  ;; The canonical ABI's padding is never written and never read (see
  ;; web/glue.js), so no zero-fill.
  (func $emit-node (param $rec i32) (param $f (ref $flat))
    (local $kind i32)
    (local $id i32)
    (local $parent i32)
    (local.set $kind (struct.get $flat $kind (local.get $f)))
     (i32.store8 (local.get $rec) (local.get $kind))
    (local.set $id (struct.get $flat $id (local.get $f)))
    (i32.store8 (i32.add (local.get $rec) (i32.const 4)) (call $nez (local.get $id)))
    (if (i32.ne (local.get $id) (i32.const 0))
      (then (i32.store (i32.add (local.get $rec) (i32.const 8)) (local.get $id))))
    (local.set $parent (struct.get $flat $parent (local.get $f)))
    (i32.store8 (i32.add (local.get $rec) (i32.const 12)) (call $nez (local.get $parent)))
    (if (i32.ne (local.get $parent) (i32.const 0))
      (then (i32.store (i32.add (local.get $rec) (i32.const 16)) (local.get $parent))))
    (if (i32.eq (local.get $kind) (i32.const 2))
      (then
         (i32.store8 (i32.add (local.get $rec) (i32.const 20))
          (struct.get $flat $direction (local.get $f)))
         (i32.store8 (i32.add (local.get $rec) (i32.const 21))
          (struct.get $flat $align (local.get $f)))
         (i32.store8 (i32.add (local.get $rec) (i32.const 22))
          (struct.get $flat $positioned (local.get $f)))
         (i32.store (i32.add (local.get $rec) (i32.const 24))
          (struct.get $flat $x (local.get $f)))
         (i32.store (i32.add (local.get $rec) (i32.const 28))
          (struct.get $flat $y (local.get $f)))
         (i32.store (i32.add (local.get $rec) (i32.const 32))
          (struct.get $flat $width (local.get $f)))
         (i32.store (i32.add (local.get $rec) (i32.const 36))
          (struct.get $flat $height (local.get $f)))
         (i32.store (i32.add (local.get $rec) (i32.const 40))
          (struct.get $flat $z (local.get $f)))
         (i32.store (i32.add (local.get $rec) (i32.const 44))
          (struct.get $flat $gap (local.get $f)))
         (i32.store (i32.add (local.get $rec) (i32.const 48))
          (struct.get $flat $padding (local.get $f)))
         (i32.store8 (i32.add (local.get $rec) (i32.const 52))
          (struct.get $flat $stretch (local.get $f)))
         (i32.store8 (i32.add (local.get $rec) (i32.const 53))
          (struct.get $flat $overflow (local.get $f)))
         (i32.store8 (i32.add (local.get $rec) (i32.const 54))
          (struct.get $flat $background (local.get $f)))
         (i32.store8 (i32.add (local.get $rec) (i32.const 55))
          (struct.get $flat $border (local.get $f)))
         (i32.store8 (i32.add (local.get $rec) (i32.const 56))
          (struct.get $flat $radius (local.get $f)))
         (i32.store8 (i32.add (local.get $rec) (i32.const 57))
          (struct.get $flat $shadow (local.get $f)))
         (i32.store8 (i32.add (local.get $rec) (i32.const 58))
          (struct.get $flat $cursor (local.get $f)))
      )
      (else
        (call $emit-label (local.get $rec) (struct.get $flat $label (local.get $f)))
        (if (i32.eq (local.get $kind) (i32.const 4))
          (then
            ;; An image: the width and the height, not the style bytes.
            (i32.store (i32.add (local.get $rec) (i32.const 28))
              (struct.get $flat $width (local.get $f)))
            (i32.store (i32.add (local.get $rec) (i32.const 32))
              (struct.get $flat $height (local.get $f)))
          )
          (else
            (if (i32.eq (local.get $kind) (i32.const 5))
              (then
                ;; A link: the href at +28/+32 (not the style bytes), the
                ;; style at +36/+37.
                (call $emit-href (local.get $rec) (struct.get $flat $href (local.get $f)))
                (i32.store8 (i32.add (local.get $rec) (i32.const 36))
                  (struct.get $flat $color (local.get $f)))
                (i32.store8 (i32.add (local.get $rec) (i32.const 37))
                  (struct.get $flat $size (local.get $f)))
              )
              (else
                (i32.store8 (i32.add (local.get $rec) (i32.const 28))
                  (struct.get $flat $color (local.get $f)))
                (i32.store8 (i32.add (local.get $rec) (i32.const 29))
                  (struct.get $flat $size (local.get $f)))
                (if (i32.eq (local.get $kind) (i32.const 3))
                  (then
                    (i32.store (i32.add (local.get $rec) (i32.const 32))
                      (struct.get $flat $width (local.get $f)))
                  ))
              ))
          ))
      ))
  )

  ;; A 68-byte `changed` mutation carrying f, at rec: the tag byte, the
  ;; 60-byte node one word in, and the index after it. The tag and the index
  ;; are the only bytes of this record a host reads besides the node's.
  (func $emit-changed (param $rec i32) (param $f (ref $flat))
     (i32.store8 (local.get $rec) (i32.const 0))
    (call $emit-node (i32.add (local.get $rec) (i32.const 4)) (local.get $f))
     (i32.store (i32.add (local.get $rec) (i32.const 64))
      (struct.get $flat $index (local.get $f)))
  )

  ;; A 68-byte `removed` mutation for id, at rec: the tag byte and the id.
  (func $emit-removed (param $rec i32) (param $id i32)
     (i32.store8 (local.get $rec) (i32.const 1))
     (i32.store (i32.add (local.get $rec) (i32.const 4)) (local.get $id))
  )

  ;; ================================================================ the diff

  ;; Whether two labels (null = empty) differ in content.
  (func $labels-differ (param $a (ref null $bytes)) (param $b (ref null $bytes)) (result i32)
    (local $na i32)
    (local $nb i32)
    (local $i i32)
    (if (ref.eq (local.get $a) (local.get $b))
      (then (return (i32.const 0))))
    (if (ref.is_null (local.get $a))
      (then (local.set $na (i32.const 0)))
      (else (local.set $na (array.len (ref.cast (ref $bytes) (local.get $a))))))
    (if (ref.is_null (local.get $b))
      (then (local.set $nb (i32.const 0)))
      (else (local.set $nb (array.len (ref.cast (ref $bytes) (local.get $b))))))
    (if (i32.ne (local.get $na) (local.get $nb))
      (then (return (i32.const 1))))
    (if (i32.eqz (local.get $na))
      (then (return (i32.const 0))))
    ;; Past here both labels are non-null and equal-length.
    (block $same
      (loop $lp
        (br_if $same (i32.ge_u (local.get $i) (local.get $na)))
        (if (i32.ne
              (array.get $bytes (ref.cast (ref $bytes) (local.get $a)) (local.get $i))
              (array.get $bytes (ref.cast (ref $bytes) (local.get $b)) (local.get $i)))
          (then (return (i32.const 1))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $lp)
      ))
    (i32.const 0)
  )

  ;; Whether f's record differs from the one with the same id in last.
  ;; Every field is compared uniformly: the fields a kind does not use
  ;; are zero (a container's color/size, a leaf's geometry and chrome
  ;; tokens, an image's direction/align), so the same test holds across
  ;; kinds; the label, and a link's href, are compared after the scalars.
  (func $fields-differ (param $f (ref $flat)) (param $r (ref $flat)) (result i32)
    (local $d i32)
    (local.set $d
      (i32.or
        (i32.or
          (i32.or
            (i32.or
              (i32.ne (struct.get $flat $kind (local.get $f)) (struct.get $flat $kind (local.get $r)))
              (i32.ne (struct.get $flat $parent (local.get $f)) (struct.get $flat $parent (local.get $r)))
            )
            (i32.or
              (i32.ne (struct.get $flat $color (local.get $f)) (struct.get $flat $color (local.get $r)))
              (i32.or
                (i32.ne (struct.get $flat $size (local.get $f)) (struct.get $flat $size (local.get $r)))
                (i32.ne (struct.get $flat $direction (local.get $f)) (struct.get $flat $direction (local.get $r)))
              )
            )
          )
          (i32.or
            (i32.or
              (i32.ne (struct.get $flat $align (local.get $f)) (struct.get $flat $align (local.get $r)))
              (i32.ne (struct.get $flat $width (local.get $f)) (struct.get $flat $width (local.get $r)))
            )
            (i32.or
              (i32.ne (struct.get $flat $height (local.get $f)) (struct.get $flat $height (local.get $r)))
              (i32.or
                (i32.ne (struct.get $flat $positioned (local.get $f)) (struct.get $flat $positioned (local.get $r)))
                (i32.ne (struct.get $flat $x (local.get $f)) (struct.get $flat $x (local.get $r)))
              )
            )
          )
        )
        (i32.or
          (i32.or
            (i32.or
              (i32.ne (struct.get $flat $y (local.get $f)) (struct.get $flat $y (local.get $r)))
              (i32.ne (struct.get $flat $z (local.get $f)) (struct.get $flat $z (local.get $r)))
            )
            (i32.or
              (i32.ne (struct.get $flat $gap (local.get $f)) (struct.get $flat $gap (local.get $r)))
              (i32.or
                (i32.ne (struct.get $flat $padding (local.get $f)) (struct.get $flat $padding (local.get $r)))
                (i32.ne (struct.get $flat $stretch (local.get $f)) (struct.get $flat $stretch (local.get $r)))
              )
            )
          )
          (i32.or
            (i32.or
              (i32.ne (struct.get $flat $overflow (local.get $f)) (struct.get $flat $overflow (local.get $r)))
              (i32.or
                (i32.ne (struct.get $flat $background (local.get $f)) (struct.get $flat $background (local.get $r)))
                (i32.ne (struct.get $flat $border (local.get $f)) (struct.get $flat $border (local.get $r)))
              )
            )
            (i32.or
              (i32.ne (struct.get $flat $radius (local.get $f)) (struct.get $flat $radius (local.get $r)))
              (i32.or
                (i32.ne (struct.get $flat $shadow (local.get $f)) (struct.get $flat $shadow (local.get $r)))
                (i32.ne (struct.get $flat $cursor (local.get $f)) (struct.get $flat $cursor (local.get $r)))
              )
            )
          )
        )
      ))
    (if (call $nez (local.get $d))
      (then (return (i32.const 1))))
    (if (call $labels-differ
          (struct.get $flat $label (local.get $f))
          (struct.get $flat $label (local.get $r)))
      (then (return (i32.const 1))))
    ;; A link's href is its record's second string; the other kinds carry null.
    (call $labels-differ
      (struct.get $flat $href (local.get $f))
      (struct.get $flat $href (local.get $r)))
  )

  ;; Whether f is reported `changed`: id'd and, in bootstrap, any;
  ;; otherwise its record differs from the last export's, or its id is new.
  (func $is-changed (param $f (ref $flat)) (param $last (ref null $flats)) (param $first i32) (result i32)
    (local $id i32)
    (local $i i32)
    (local $last-arr (ref $flats))
    (local.set $id (struct.get $flat $id (local.get $f)))
    (if (i32.eqz (local.get $id))
      (then (return (i32.const 0))))
    (if (local.get $first)
      (then (return (i32.const 1))))
    (local.set $last-arr (ref.cast (ref $flats) (local.get $last)))
    (block $found
      (loop $lp
        (br_if $found (i32.ge_u (local.get $i) (array.len (local.get $last-arr))))
        (if (i32.eq
              (struct.get $flat $id (array.get $flats (local.get $last-arr) (local.get $i)))
              (local.get $id))
          (then
            (return (call $fields-differ (local.get $f)
              (ref.cast (ref $flat) (array.get $flats (local.get $last-arr) (local.get $i))))))
          )
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $lp)
      ))
    (i32.const 1)
  )

  ;; Whether id is among the ids of arr.
  (func $present-in (param $arr (ref $flats)) (param $id i32) (result i32)
    (local $i i32)
    (block $found
      (loop $lp
        (br_if $found (i32.ge_u (local.get $i) (array.len (local.get $arr))))
        (if (i32.eq
              (struct.get $flat $id (array.get $flats (local.get $arr) (local.get $i)))
              (local.get $id))
          (then (return (i32.const 1))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $lp)
      ))
    (i32.const 0)
  )

  ;; The diff's size: the changed nodes plus the removed ids.
  (func $diff-count (param $arr (ref $flats)) (param $last (ref null $flats)) (param $first i32) (result i32)
    (local $i i32)
    (local $n i32)
    (local $last-arr (ref $flats))
    (local $id i32)
    (block $done
      (loop $lp
        (br_if $done (i32.ge_u (local.get $i) (array.len (local.get $arr))))
        (if (call $nez
              (call $is-changed
                (ref.cast (ref $flat) (array.get $flats (local.get $arr) (local.get $i)))
                (local.get $last)
                (local.get $first)))
          (then (local.set $n (i32.add (local.get $n) (i32.const 1)))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $lp)
      ))
    (if (i32.eqz (local.get $first))
      (then
        (local.set $last-arr (ref.cast (ref $flats) (local.get $last)))
        (local.set $i (i32.const 0))
        (block $done
          (loop $lp
            (br_if $done (i32.ge_u (local.get $i) (array.len (local.get $last-arr))))
            (local.set $id (struct.get $flat $id (array.get $flats (local.get $last-arr) (local.get $i))))
            (if (i32.and
                  (call $nez (local.get $id))
                  (i32.eqz (call $present-in (local.get $arr) (local.get $id))))
              (then (local.set $n (i32.add (local.get $n) (i32.const 1)))))
            (local.set $i (i32.add (local.get $i) (i32.const 1)))
            (br $lp)
          ))
      ))
    (local.get $n)
  )

  ;; ================================================================ the tree

  ;; The menu bar's window count, as a fresh GC byte array: the count in
  ;; decimal, " window", and an "s" unless the count is one.
  (func $count-label (param $n i32) (result (ref $bytes))
    (local $t i32)
    (local $count i32)
    (local $plural i32)
    (local $len i32)
    (local $a (ref $bytes))
    (local $i i32)
    (local $k i32)
    ;; Digit count as a do-while: at least one digit (0 is "0"), one per
    ;; division until the magnitude is exhausted.
    (local.set $t (local.get $n))
    (local.set $count (i32.const 0))
    (block $done
      (loop $lp
        (local.set $count (i32.add (local.get $count) (i32.const 1)))
        (local.set $t (i32.div_u (local.get $t) (i32.const 10)))
        (br_if $done (i32.eqz (local.get $t)))
        (br $lp)
      ))
    ;; 1 when the "s" is wanted (the count is not one).
    (local.set $plural (i32.ne (local.get $n) (i32.const 1)))
    (local.set $len (i32.add (i32.add (local.get $count) (i32.const 7)) (local.get $plural)))
    (local.set $a (array.new_default $bytes (local.get $len)))
    ;; The digits, most significant first.
    (local.set $i (i32.sub (local.get $count) (i32.const 1)))
    (local.set $t (local.get $n))
    (block $done
      (loop $lp
        (array.set $bytes (local.get $a) (local.get $i)
          (i32.add (i32.const 48) (i32.rem_u (local.get $t) (i32.const 10))))
        (local.set $t (i32.div_u (local.get $t) (i32.const 10)))
        (local.set $i (i32.sub (local.get $i) (i32.const 1)))
        (br_if $done (i32.eqz (local.get $t)))
        (br $lp)
      ))
    ;; " window", then the plural "s" (unless the count is one).
    (local.set $k (local.get $count))
    (block $done
      (loop $lp
        (br_if $done (i32.ge_u (local.get $k) (i32.add (local.get $count) (i32.const 7))))
        (array.set $bytes (local.get $a) (local.get $k)
          (i32.load8_u (i32.add (i32.const 65540) (i32.sub (local.get $k) (local.get $count)))))
        (local.set $k (i32.add (local.get $k) (i32.const 1)))
        (br $lp)
      ))
    (if (local.get $plural)
      (then
        (array.set $bytes (local.get $a) (i32.add (local.get $count) (i32.const 7)) (i32.const 115))))
    (local.get $a)
  )

  ;; The desktop's export size: the root, the menu bar (three), the grid
  ;; area and the grid, one node per row, one node per tile plus its
  ;; children (an icon, a label, and a caption, each when the entry's blob
  ;; supplies it), and one node per window plus its chrome (a title bar and
  ;; its children, the content slot, and the grip).
  (func $node-count (result i32)
    (local $total i32)
    (local $i i32)
    (local $e (ref $entry))
    (local $w (ref $win))
    (local $entries (ref $entries))
    (local $wins (ref $wins))
    (local.set $total (i32.const 10)) ;; root, menu bar, its six, the area, the grid
    (local.set $entries (ref.cast (ref $entries) (global.get $entries)))
    (local.set $total (i32.add (local.get $total)
      (i32.div_u (i32.add (array.len (local.get $entries)) (i32.const 2)) (i32.const 3))))
    (block $done
      (loop $lp
        (br_if $done (i32.ge_u (local.get $i) (array.len (local.get $entries))))
        (local.set $e (ref.cast (ref $entry) (array.get $entries (local.get $entries) (local.get $i))))
        (local.set $total (i32.add (local.get $total) (i32.const 2))) ;; the tile, its label
        (if (i32.eqz (ref.is_null (struct.get $entry $icon (local.get $e))))
          (then (local.set $total (i32.add (local.get $total) (i32.const 1)))))
        (if (i32.eqz (ref.is_null (struct.get $entry $lang (local.get $e))))
          (then (local.set $total (i32.add (local.get $total) (i32.const 1)))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $lp)
      ))
    ;; The windows, when any are open.
    (if (ref.is_null (global.get $wins))
      (then)
      (else
        (local.set $wins (ref.cast (ref $wins) (global.get $wins)))
        ;; The entries loop left $i at their count; restart it.
        (local.set $i (i32.const 0))
        (block $done
          (loop $lp
            (br_if $done (i32.ge_u (local.get $i) (global.get $win-len)))
            (local.set $w (ref.cast (ref $win) (array.get $wins (local.get $wins) (local.get $i))))
            ;; The window's eight fixed nodes (the icon, when any, is the ninth).
            (local.set $total (i32.add (local.get $total) (i32.const 8)))
            (if (i32.eqz (ref.is_null (struct.get $win $icon (local.get $w))))
              (then (local.set $total (i32.add (local.get $total) (i32.const 1)))))
            (local.set $i (i32.add (local.get $i) (i32.const 1)))
            (br $lp)
          ))))
    (local.get $total)
  )

  ;; The desktop's whole export, as a flat, pre-ordered list: the root
  ;; column (1), the menu bar (2), the grid area (6) and the grid (7), the
  ;; rows (8+r) and the tiles (100+i), and the open windows (their ids are
  ;; derived from the host handle).
  (func $build (result (ref $flats))
    (local $entries (ref null $entries))
    (local $en (ref $entries))
    (local $wins (ref null $wins))
    (local $wn (ref $wins))
    (local $arr (ref $flats))
    (local $n i32)
    (local $n-rows i32)
    (local $pos i32)
    (local $r i32)
    (local $c i32)
    (local $i i32)
    (local $wi i32)
    (local $e (ref $entry))
    (local $w (ref $win))
    (local $base i32)
    (local $has-icon i32)
    (local $has-lang i32)
    (local.set $entries (global.get $entries))
    (if (ref.is_null (local.get $entries))
      (then
        (global.set $entries (ref.cast (ref null $entries) (call $fetch-entries)))
        (local.set $entries (global.get $entries))))
    ;; Past the fetch, the registry is non-null; the type system needs the cast.
    (local.set $en (ref.cast (ref $entries) (local.get $entries)))
    (local.set $n (array.len (local.get $en)))
    (local.set $n-rows (i32.div_u (i32.add (local.get $n) (i32.const 2)) (i32.const 3)))
    (local.set $arr (array.new_default $flats (call $node-count)))
    ;; The root column (1): it fills the host, and the windows float in it.
    (array.set $flats (local.get $arr) (i32.const 0)
      (struct.new $flat
        (i32.const 2) (i32.const 1) (i32.const 0)
        (i32.const 0) (i32.const 0) (i32.const 1) (i32.const 0)
        (i32.const 0) (i32.const 0)
        (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
        (i32.const 0) (i32.const 0) (i32.const 3) (i32.const 0)
        (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
        (i32.const 0)
        (ref.null $bytes) (ref.null $bytes) (i32.const 0)))
    ;; The menu bar (2): the "waui" word, the tagline, a spacer, the window
    ;; count (4), a hint, and the repo link.
    (array.set $flats (local.get $arr) (i32.const 1)
      (struct.new $flat
        (i32.const 2) (i32.const 2) (i32.const 1)
        (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 1)
        (i32.const 0) (i32.const 34)
        (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 1000)
        (i32.const 12) (i32.const 16) (i32.const 1) (i32.const 0)
        (i32.const 2) (i32.const 1) (i32.const 0) (i32.const 0)
        (i32.const 0)
        (ref.null $bytes) (ref.null $bytes) (i32.const 0)))
    (array.set $flats (local.get $arr) (i32.const 2)
      (struct.new $flat
        (i32.const 0) (i32.const 0) (i32.const 2)
        (i32.const 1) (i32.const 2) (i32.const 0) (i32.const 0)
        (i32.const 0) (i32.const 0)
        (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
        (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
        (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
        (i32.const 0)
        (call $label (i32.const 65536) (i32.const 4)) (ref.null $bytes) (i32.const 0)))
    (array.set $flats (local.get $arr) (i32.const 3)
      (struct.new $flat
        (i32.const 0) (i32.const 0) (i32.const 2)
        (i32.const 5) (i32.const 0) (i32.const 0) (i32.const 0)
        (i32.const 0) (i32.const 0)
        (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
        (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
        (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
        (i32.const 0)
        (call $label (i32.const 65594) (i32.const 13)) (ref.null $bytes) (i32.const 1)))
    (array.set $flats (local.get $arr) (i32.const 4)
      (struct.new $flat
        (i32.const 2) (i32.const 3) (i32.const 2)
        (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 1)
        (i32.const 0) (i32.const 0)
        (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
        (i32.const 0) (i32.const 0) (i32.const 1) (i32.const 0)
        (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
        (i32.const 0)
        (ref.null $bytes) (ref.null $bytes) (i32.const 2)))
    (array.set $flats (local.get $arr) (i32.const 5)
      (struct.new $flat
        (i32.const 0) (i32.const 4) (i32.const 2)
        (i32.const 5) (i32.const 0) (i32.const 0) (i32.const 0)
        (i32.const 0) (i32.const 0)
        (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
        (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
        (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
        (i32.const 0)
        (call $count-label (global.get $win-len)) (ref.null $bytes) (i32.const 3)))
    (array.set $flats (local.get $arr) (i32.const 6)
      (struct.new $flat
        (i32.const 0) (i32.const 0) (i32.const 2)
        (i32.const 5) (i32.const 0) (i32.const 0) (i32.const 0)
        (i32.const 0) (i32.const 0)
        (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
        (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
        (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
        (i32.const 0)
        (call $label (i32.const 65607) (i32.const 28)) (ref.null $bytes) (i32.const 4)))
    (array.set $flats (local.get $arr) (i32.const 7)
      (struct.new $flat
        (i32.const 5) (i32.const 0) (i32.const 2)
        (i32.const 1) (i32.const 0) (i32.const 0) (i32.const 0)
        (i32.const 0) (i32.const 0)
        (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
        (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
        (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
        (i32.const 0)
        (call $label (i32.const 65635) (i32.const 6))
        (call $label (i32.const 65641) (i32.const 33))
        (i32.const 5)))
    ;; The grid area (6) and the grid (7): the rows (8+r) and the tiles.
    (array.set $flats (local.get $arr) (i32.const 8)
      (struct.new $flat
        (i32.const 2) (i32.const 6) (i32.const 1)
        (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 1)
        (i32.const 0) (i32.const 0)
        (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
        (i32.const 0) (i32.const 0) (i32.const 3) (i32.const 0)
        (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
        (i32.const 0)
        (ref.null $bytes) (ref.null $bytes) (i32.const 1)))
    (array.set $flats (local.get $arr) (i32.const 9)
      (struct.new $flat
        (i32.const 2) (i32.const 7) (i32.const 6)
        (i32.const 0) (i32.const 0) (i32.const 1) (i32.const 1)
        (i32.const 0) (i32.const 0)
        (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
        (i32.const 28) (i32.const 0) (i32.const 2) (i32.const 0)
        (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
        (i32.const 0)
        (ref.null $bytes) (ref.null $bytes) (i32.const 0)))
    (local.set $pos (i32.const 10))
    (block $rows
      (loop $lp-rows
        (br_if $rows (i32.ge_u (local.get $r) (local.get $n-rows)))
        ;; Row 8+r.
        (array.set $flats (local.get $arr) (local.get $pos)
          (struct.new $flat
            (i32.const 2) (i32.add (i32.const 8) (local.get $r)) (i32.const 7)
            (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 1)
            (i32.const 0) (i32.const 0)
            (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
            (i32.const 44) (i32.const 0) (i32.const 0) (i32.const 0)
            (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
            (i32.const 0)
            (ref.null $bytes) (ref.null $bytes) (local.get $r)))
        (local.set $pos (i32.add (local.get $pos) (i32.const 1)))
        (local.set $c (i32.const 0))
        (block $cols
          (loop $lp-cols
            (br_if $cols (i32.ge_u (local.get $c) (i32.const 3)))
            (local.set $i (i32.add (i32.mul (local.get $r) (i32.const 3)) (local.get $c)))
            (br_if $cols (i32.ge_u (local.get $i) (local.get $n)))
            (local.set $e (ref.cast (ref $entry) (array.get $entries (local.get $en) (local.get $i))))
            (local.set $has-icon (i32.eqz (ref.is_null (struct.get $entry $icon (local.get $e)))))
            (local.set $has-lang (i32.eqz (ref.is_null (struct.get $entry $lang (local.get $e)))))
            ;; Tile 100+i: a double-clickable column; the selected one rings.
            (array.set $flats (local.get $arr) (local.get $pos)
              (struct.new $flat
                (i32.const 2) (i32.add (i32.const 100) (local.get $i)) (i32.add (i32.const 8) (local.get $r))
                (i32.const 0) (i32.const 0) (i32.const 1) (i32.const 1)
                (i32.const 0) (i32.const 0)
                (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
                (i32.const 7) (i32.const 10) (i32.const 0) (i32.const 0)
                (i32.const 0)
                (if (result i32) (i32.eq (global.get $selected) (local.get $i))
                  (then (i32.const 2))
                  (else (i32.const 0)))
                (i32.const 2) (i32.const 0)
                (i32.const 1)
                (ref.null $bytes) (ref.null $bytes) (local.get $c)))
            (local.set $pos (i32.add (local.get $pos) (i32.const 1)))
            (if (local.get $has-icon)
              (then
                ;; The icon: 112px, the tile size.
                (array.set $flats (local.get $arr) (local.get $pos)
                  (struct.new $flat
                    (i32.const 4) (i32.const 0) (i32.add (i32.const 100) (local.get $i))
                    (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
                    (i32.const 112) (i32.const 112)
                    (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
                    (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
                    (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
                    (i32.const 0)
                    (struct.get $entry $icon (local.get $e)) (ref.null $bytes) (i32.const 0)))
                (local.set $pos (i32.add (local.get $pos) (i32.const 1)))))
            (array.set $flats (local.get $arr) (local.get $pos)
              (struct.new $flat
                (i32.const 0) (i32.const 0) (i32.add (i32.const 100) (local.get $i))
                (i32.const 0) (i32.const 1) (i32.const 0) (i32.const 0)
                (i32.const 0) (i32.const 0)
                (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
                (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
                (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
                (i32.const 0)
                (struct.get $entry $label (local.get $e)) (ref.null $bytes) (local.get $has-icon)))
            (local.set $pos (i32.add (local.get $pos) (i32.const 1)))
            (if (local.get $has-lang)
              (then
                (array.set $flats (local.get $arr) (local.get $pos)
                  (struct.new $flat
                    (i32.const 0) (i32.const 0) (i32.add (i32.const 100) (local.get $i))
                    (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
                    (i32.const 0) (i32.const 0)
                    (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
                    (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
                    (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
                    (i32.const 0)
                    (struct.get $entry $lang (local.get $e))
                    (ref.null $bytes) (i32.add (local.get $has-icon) (i32.const 1))))
                (local.set $pos (i32.add (local.get $pos) (i32.const 1)))))
            (local.set $c (i32.add (local.get $c) (i32.const 1)))
            (br $lp-cols)
          ))
        (local.set $r (i32.add (local.get $r) (i32.const 1)))
        (br $lp-rows)
      ))
    ;; The open windows, in the order they were opened.
    (local.set $wins (global.get $wins))
    (if (ref.is_null (local.get $wins))
      (then)
      (else
        (local.set $wn (ref.cast (ref $wins) (local.get $wins)))
        (local.set $wi (i32.const 0))
        (block $done
          (loop $lp-wins
            (br_if $done (i32.ge_u (local.get $wi) (global.get $win-len)))
            (local.set $w (ref.cast (ref $win) (array.get $wins (local.get $wn) (local.get $wi))))
            (local.set $base (struct.get $win $handle (local.get $w)))
            ;; The window (handle + 100000): placed, stacked, a panel.
            (array.set $flats (local.get $arr) (local.get $pos)
              (struct.new $flat
                (i32.const 2) (i32.add (local.get $base) (i32.const 100000)) (i32.const 1)
                (i32.const 0) (i32.const 0) (i32.const 1) (i32.const 0)
                (struct.get $win $w (local.get $w)) (struct.get $win $h (local.get $w))
                (i32.const 1) (struct.get $win $x (local.get $w)) (struct.get $win $y (local.get $w)) (struct.get $win $z (local.get $w))
                (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
                (i32.const 1) (i32.const 1) (i32.const 2) (i32.const 1)
                (i32.const 0)
                (ref.null $bytes) (ref.null $bytes) (i32.add (i32.const 2) (local.get $wi))))
            (local.set $pos (i32.add (local.get $pos) (i32.const 1)))
            ;; The title bar (handle + 200000): the drag handle.
            (array.set $flats (local.get $arr) (local.get $pos)
              (struct.new $flat
                (i32.const 2) (i32.add (local.get $base) (i32.const 200000)) (i32.add (local.get $base) (i32.const 100000))
                (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 1)
                (i32.const 0) (i32.const 36)
                (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
                (i32.const 0) (i32.const 0) (i32.const 1) (i32.const 0)
                (i32.const 2) (i32.const 1) (i32.const 0) (i32.const 0)
                (i32.const 2)
                (ref.null $bytes) (ref.null $bytes) (i32.const 0)))
            (local.set $pos (i32.add (local.get $pos) (i32.const 1)))
            (local.set $has-icon (i32.eqz (ref.is_null (struct.get $win $icon (local.get $w)))))
            (if (local.get $has-icon)
              (then
                (array.set $flats (local.get $arr) (local.get $pos)
                  (struct.new $flat
                    (i32.const 4) (i32.const 0) (i32.add (local.get $base) (i32.const 200000))
                    (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
                    (i32.const 18) (i32.const 18)
                    (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
                    (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
                    (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
                    (i32.const 0)
                    (struct.get $win $icon (local.get $w)) (ref.null $bytes) (i32.const 0)))
                (local.set $pos (i32.add (local.get $pos) (i32.const 1)))))
            (array.set $flats (local.get $arr) (local.get $pos)
              (struct.new $flat
                (i32.const 0) (i32.const 0) (i32.add (local.get $base) (i32.const 200000))
                (i32.const 0) (i32.const 1) (i32.const 0) (i32.const 0)
                (i32.const 0) (i32.const 0)
                (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
                (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
                (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
                (i32.const 0)
                (struct.get $win $label (local.get $w)) (ref.null $bytes) (local.get $has-icon)))
            (local.set $pos (i32.add (local.get $pos) (i32.const 1)))
            (array.set $flats (local.get $arr) (local.get $pos)
              (struct.new $flat
                (i32.const 2) (i32.add (local.get $base) (i32.const 300000)) (i32.add (local.get $base) (i32.const 200000))
                (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
                (i32.const 0) (i32.const 0)
                (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
                (i32.const 0) (i32.const 0) (i32.const 1) (i32.const 0)
                (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
                (i32.const 0)
                (ref.null $bytes) (ref.null $bytes) (i32.add (local.get $has-icon) (i32.const 1))))
            (local.set $pos (i32.add (local.get $pos) (i32.const 1)))
            ;; The close marker: a container (the id, the hit area) around an
            ;; id-less text. A button node would draw the host's button chrome.
            (array.set $flats (local.get $arr) (local.get $pos)
              (struct.new $flat
                (i32.const 2) (i32.add (local.get $base) (i32.const 400000)) (i32.add (local.get $base) (i32.const 200000))
                (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 1)
                (i32.const 48) (i32.const 32)
                (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
                (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
                (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
                (i32.const 0)
                (ref.null $bytes) (ref.null $bytes) (i32.add (local.get $has-icon) (i32.const 2))))
            (local.set $pos (i32.add (local.get $pos) (i32.const 1)))
            (array.set $flats (local.get $arr) (local.get $pos)
              (struct.new $flat
                (i32.const 0) (i32.const 0) (i32.add (local.get $base) (i32.const 400000))
                (i32.const 0) (i32.const 1) (i32.const 0) (i32.const 0)
                (i32.const 0) (i32.const 0)
                (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
                (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
                (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
                (i32.const 0)
                (call $label (i32.const 65591) (i32.const 3))
                (ref.null $bytes) (i32.const 0)))
            (local.set $pos (i32.add (local.get $pos) (i32.const 1)))
            ;; The content slot: its id is the handle itself (the host seats
            ;; the opened component's tree there).
            (array.set $flats (local.get $arr) (local.get $pos)
              (struct.new $flat
                (i32.const 2) (local.get $base) (i32.add (local.get $base) (i32.const 100000))
                (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 1)
                (i32.const 0) (i32.const 0)
                (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
                (i32.const 0) (i32.const 0) (i32.const 3) (i32.const 0)
                (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
                (i32.const 0)
                (ref.null $bytes) (ref.null $bytes) (i32.const 1)))
            (local.set $pos (i32.add (local.get $pos) (i32.const 1)))
            ;; The grip, at the window's bottom-right corner.
            (array.set $flats (local.get $arr) (local.get $pos)
              (struct.new $flat
                (i32.const 2) (i32.add (local.get $base) (i32.const 500000)) (i32.add (local.get $base) (i32.const 100000))
                (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
                (i32.const 14) (i32.const 14)
                (i32.const 1)
                (call $ssub (struct.get $win $w (local.get $w)) (i32.const 17))
                (call $ssub (struct.get $win $h (local.get $w)) (i32.const 17))
                (i32.const 0)
                (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
                (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
                (i32.const 4)
                (ref.null $bytes) (ref.null $bytes) (i32.const 2)))
            (local.set $pos (i32.add (local.get $pos) (i32.const 1)))
            (local.set $wi (i32.add (local.get $wi) (i32.const 1)))
            (br $lp-wins)
          ))))
    (local.get $arr)
  )

  ;; ================================================== the `data` blob (JSON)

  ;; A minimal JSON parser for the flat object the `data` blob is: an object
  ;; of string/number values. A real component would use a full parser; this
  ;; one reads the shape this host's registry emits, and anything else
  ;; (nesting, arrays, a malformed blob) stops the parse, so the caller's
  ;; fallbacks apply. The cursor is a global: the parse is synchronous, so
  ;; one is live at a time.

  ;; The byte at i of the blob, or -1 past the end.
  (func $p-peek (param $data (ref $bytes)) (param $i i32) (result i32)
    (if (i32.ge_u (local.get $i) (array.len (local.get $data)))
      (then (return (i32.const -1))))
    (array.get $bytes (local.get $data) (local.get $i))
  )

  ;; Advance the cursor over whitespace.
  (func $p-skip (param $data (ref $bytes))
    (local $c i32)
    (block $done
      (loop $lp
        (local.set $c (call $p-peek (local.get $data) (global.get $p-cur)))
        (if (i32.or
              (i32.or (i32.eq (local.get $c) (i32.const 32)) (i32.eq (local.get $c) (i32.const 9)))
              (i32.or (i32.eq (local.get $c) (i32.const 10)) (i32.eq (local.get $c) (i32.const 13))))
          (then
            (global.set $p-cur (i32.add (global.get $p-cur) (i32.const 1)))
            (br $lp)))
        (br $done)
      ))
  )

  ;; Consume one byte if it is c; report whether it was.
  (func $p-eat (param $data (ref $bytes)) (param $c i32) (result i32)
    (local $got i32)
    (local.set $got (i32.eq (call $p-peek (local.get $data) (global.get $p-cur)) (local.get $c)))
    (if (local.get $got)
      (then (global.set $p-cur (i32.add (global.get $p-cur) (i32.const 1)))))
    (local.get $got)
  )

  ;; The value of a hex digit byte, or -1.
  (func $hex-digit (param $c i32) (result i32)
    (if (i32.and (i32.ge_u (local.get $c) (i32.const 48)) (i32.le_u (local.get $c) (i32.const 57)))
      (then (return (i32.sub (local.get $c) (i32.const 48)))))
    (if (i32.and (i32.ge_u (local.get $c) (i32.const 65)) (i32.le_u (local.get $c) (i32.const 70)))
      (then (return (i32.sub (local.get $c) (i32.const 55)))))
    (if (i32.and (i32.ge_u (local.get $c) (i32.const 97)) (i32.le_u (local.get $c) (i32.const 102)))
      (then (return (i32.sub (local.get $c) (i32.const 87)))))
    (i32.const -1)
  )

  ;; The four hex digits of a `\uXXXX` escape as a code point, or -1.
  (func $hex4 (param $data (ref $bytes)) (param $i i32) (result i32)
    (local $v i32)
    (local $k i32)
    (local $d i32)
    (block $done
      (loop $lp
        (br_if $done (i32.eq (local.get $k) (i32.const 4)))
        (local.set $d (call $hex-digit (array.get $bytes (local.get $data) (i32.add (local.get $i) (local.get $k)))))
        (if (i32.lt_s (local.get $d) (i32.const 0))
          (then (return (i32.const -1))))
        (local.set $v (i32.add (i32.mul (local.get $v) (i32.const 16)) (local.get $d)))
        (local.set $k (i32.add (local.get $k) (i32.const 1)))
        (br $lp)
      ))
    (local.get $v)
  )

  ;; Whether a code point is not a character: a surrogate half, or past the
  ;; Unicode range.
  (func $cp-invalid (param $cp i32) (result i32)
    (i32.or
      (i32.and (i32.ge_u (local.get $cp) (i32.const 55296)) (i32.le_u (local.get $cp) (i32.const 57343)))
      (i32.gt_u (local.get $cp) (i32.const 1114111)))
  )

  ;; The UTF-8 length of a code point, in bytes.
  (func $utf8len (param $cp i32) (result i32)
    (if (i32.lt_u (local.get $cp) (i32.const 128))
      (then (return (i32.const 1))))
    (if (i32.lt_u (local.get $cp) (i32.const 2048))
      (then (return (i32.const 2))))
    (if (i32.lt_u (local.get $cp) (i32.const 65536))
      (then (return (i32.const 3))))
    (i32.const 4)
  )

  ;; Encode a code point as UTF-8 at w of a; the new cursor.
  (func $write-utf8 (param $a (ref $bytes)) (param $w i32) (param $cp i32) (result i32)
    (if (i32.lt_u (local.get $cp) (i32.const 128))
      (then
        (array.set $bytes (local.get $a) (local.get $w) (local.get $cp))
        (return (i32.add (local.get $w) (i32.const 1)))))
    (if (i32.lt_u (local.get $cp) (i32.const 2048))
      (then
        (array.set $bytes (local.get $a) (local.get $w)
          (i32.or (i32.const 192) (i32.shr_u (local.get $cp) (i32.const 6))))
        (array.set $bytes (local.get $a) (i32.add (local.get $w) (i32.const 1))
          (i32.or (i32.const 128) (i32.and (local.get $cp) (i32.const 63))))
        (return (i32.add (local.get $w) (i32.const 2)))))
    (if (i32.lt_u (local.get $cp) (i32.const 65536))
      (then
        (array.set $bytes (local.get $a) (local.get $w)
          (i32.or (i32.const 224) (i32.shr_u (local.get $cp) (i32.const 12))))
        (array.set $bytes (local.get $a) (i32.add (local.get $w) (i32.const 1))
          (i32.or (i32.const 128) (i32.and (i32.shr_u (local.get $cp) (i32.const 6)) (i32.const 63))))
        (array.set $bytes (local.get $a) (i32.add (local.get $w) (i32.const 2))
          (i32.or (i32.const 128) (i32.and (local.get $cp) (i32.const 63))))
        (return (i32.add (local.get $w) (i32.const 3)))))
    ;; Four bytes (a surrogate pair).
    (array.set $bytes (local.get $a) (local.get $w)
      (i32.or (i32.const 240) (i32.shr_u (local.get $cp) (i32.const 18))))
    (array.set $bytes (local.get $a) (i32.add (local.get $w) (i32.const 1))
      (i32.or (i32.const 128) (i32.and (i32.shr_u (local.get $cp) (i32.const 12)) (i32.const 63))))
    (array.set $bytes (local.get $a) (i32.add (local.get $w) (i32.const 2))
      (i32.or (i32.const 128) (i32.and (i32.shr_u (local.get $cp) (i32.const 6)) (i32.const 63))))
    (array.set $bytes (local.get $a) (i32.add (local.get $w) (i32.const 3))
      (i32.or (i32.const 128) (i32.and (local.get $cp) (i32.const 63))))
    (i32.add (local.get $w) (i32.const 4))
  )

  ;; Whether an escape's second byte is a simple one-byte escape (`\"` `\\`
  ;; `\/` `\b` `\f` `\n` `\r` `\t`); `\u` is the other case.
  (func $simple-escape (param $e i32) (result i32)
    (i32.or
      (i32.or
        (i32.or (i32.eq (local.get $e) (i32.const 34)) (i32.eq (local.get $e) (i32.const 92)))
        (i32.or (i32.eq (local.get $e) (i32.const 47)) (i32.eq (local.get $e) (i32.const 98))))
      (i32.or
        (i32.or (i32.eq (local.get $e) (i32.const 102)) (i32.eq (local.get $e) (i32.const 110)))
        (i32.or (i32.eq (local.get $e) (i32.const 114)) (i32.eq (local.get $e) (i32.const 116)))))
  )

  ;; The byte a simple escape resolves to.
  (func $resolve-escape (param $e i32) (result i32)
    (if (i32.eq (local.get $e) (i32.const 34)) (then (return (i32.const 34))))
    (if (i32.eq (local.get $e) (i32.const 92)) (then (return (i32.const 92))))
    (if (i32.eq (local.get $e) (i32.const 47)) (then (return (i32.const 47))))
    (if (i32.eq (local.get $e) (i32.const 98)) (then (return (i32.const 8))))
    (if (i32.eq (local.get $e) (i32.const 102)) (then (return (i32.const 12))))
    (if (i32.eq (local.get $e) (i32.const 110)) (then (return (i32.const 10))))
    (if (i32.eq (local.get $e) (i32.const 114)) (then (return (i32.const 13))))
    (i32.const 9)
  )

  ;; A JSON string, escapes resolved, in a fresh GC byte array; the cursor
  ;; must be at the opening quote. Null on a malformed string (the cursor is
  ;; left where the parse stopped; the caller's parse ends either way).
  ;; Two passes: the first measures (a `\uXXXX` is 1-4 bytes), the second
  ;; fills the exact-sized array.
  (func $parse-string (param $data (ref $bytes)) (result (ref null $bytes))
    (local $ok i32)
    (local $start i32)
    (local $i i32)
    (local $c i32)
    (local $e i32)
    (local $cp i32)
    (local $len i32)
    (local $w i32)
    (local $a (ref $bytes))
    (if (i32.ne (call $p-peek (local.get $data) (global.get $p-cur)) (i32.const 34))
      (then (return (ref.null $bytes))))
    (local.set $ok (i32.const 1))
    (local.set $start (i32.add (global.get $p-cur) (i32.const 1)))
    ;; Pass 1: measure.
    (local.set $i (local.get $start))
    (block $done
      (loop $walk
        (if (i32.ge_u (local.get $i) (array.len (local.get $data)))
          (then
            (local.set $ok (i32.const 0))
            (br $done)))
        (local.set $c (array.get $bytes (local.get $data) (local.get $i)))
        (if (i32.eq (local.get $c) (i32.const 34))
          (then
            (local.set $i (i32.add (local.get $i) (i32.const 1)))
            (br $done)))
        (if (i32.eq (local.get $c) (i32.const 92))
          (then
            (if (i32.ge_u (i32.add (local.get $i) (i32.const 1)) (array.len (local.get $data)))
              (then
                (local.set $ok (i32.const 0))
                (br $done)))
            (local.set $e (array.get $bytes (local.get $data) (i32.add (local.get $i) (i32.const 1))))
            (if (call $simple-escape (local.get $e))
              (then
                (local.set $len (i32.add (local.get $len) (i32.const 1)))
                (local.set $i (i32.add (local.get $i) (i32.const 2))))
              (else
                (if (i32.ne (local.get $e) (i32.const 117))
                  (then
                    (local.set $ok (i32.const 0))
                    (br $done)))
                (if (i32.gt_u (i32.add (local.get $i) (i32.const 5)) (array.len (local.get $data)))
                  (then
                    (local.set $ok (i32.const 0))
                    (br $done)))
                (local.set $cp (call $hex4 (local.get $data) (i32.add (local.get $i) (i32.const 2))))
                (if (call $cp-invalid (local.get $cp))
                  (then
                    (local.set $ok (i32.const 0))
                    (br $done)))
                (local.set $len (i32.add (local.get $len) (call $utf8len (local.get $cp))))
                (local.set $i (i32.add (local.get $i) (i32.const 6)))))
            (br $walk))
          (else
            (local.set $len (i32.add (local.get $len) (i32.const 1)))
            (local.set $i (i32.add (local.get $i) (i32.const 1)))
            (br $walk)))
        (br $walk)
      ))
    (if (i32.eqz (local.get $ok))
      (then (return (ref.null $bytes))))
    ;; The cursor moves past the closing quote.
    (global.set $p-cur (local.get $i))
    ;; Pass 2: fill (pass 1 validated the bounds, so no checks).
    (local.set $a (array.new_default $bytes (local.get $len)))
    (local.set $i (local.get $start))
    (block $done
      (loop $walk
        (local.set $c (array.get $bytes (local.get $data) (local.get $i)))
        (if (i32.eq (local.get $c) (i32.const 34))
          (then (br $done)))
        (if (i32.eq (local.get $c) (i32.const 92))
          (then
            (local.set $e (array.get $bytes (local.get $data) (i32.add (local.get $i) (i32.const 1))))
            (if (call $simple-escape (local.get $e))
              (then
                (array.set $bytes (local.get $a) (local.get $w) (call $resolve-escape (local.get $e)))
                (local.set $w (i32.add (local.get $w) (i32.const 1)))
                (local.set $i (i32.add (local.get $i) (i32.const 2))))
              (else
                (local.set $cp (call $hex4 (local.get $data) (i32.add (local.get $i) (i32.const 2))))
                (local.set $w (call $write-utf8 (local.get $a) (local.get $w) (local.get $cp)))
                (local.set $i (i32.add (local.get $i) (i32.const 6)))))
            (br $walk))
          (else
            (array.set $bytes (local.get $a) (local.get $w) (local.get $c))
            (local.set $w (i32.add (local.get $w) (i32.const 1)))
            (local.set $i (i32.add (local.get $i) (i32.const 1)))
            (br $walk)))
        (br $walk)
      ))
    (local.get $a)
  )

  ;; A JSON number: (ok, value). The cursor advances over the whole token;
  ;; the value is its integer part (the registry's sizes are small integers;
  ;; a fractional part or exponent is not parsed).
  (func $parse-number (param $data (ref $bytes)) (result i32 i32)
    (local $start i32)
    (local $c i32)
    (local $i i32)
    (local $dstart i32)
    (local $minus i32)
    (local $v i32)
    (local.set $start (global.get $p-cur))
    (block $scan
      (loop $lp
        (local.set $c (call $p-peek (local.get $data) (global.get $p-cur)))
        (if (i32.or
              (i32.and (i32.ge_u (local.get $c) (i32.const 48)) (i32.le_u (local.get $c) (i32.const 57)))
              (i32.or
                (i32.or (i32.eq (local.get $c) (i32.const 45)) (i32.eq (local.get $c) (i32.const 43)))
                (i32.or (i32.eq (local.get $c) (i32.const 46)) (i32.or (i32.eq (local.get $c) (i32.const 101)) (i32.eq (local.get $c) (i32.const 69))))))
          (then
            (global.set $p-cur (i32.add (global.get $p-cur) (i32.const 1)))
            (br $lp)))
        (br $scan)
      ))
    (if (i32.eq (global.get $p-cur) (local.get $start))
      (then (return (i32.const 0) (i32.const 0))))
    (local.set $i (local.get $start))
    (if (i32.eq (array.get $bytes (local.get $data) (local.get $i)) (i32.const 45))
      (then
        (local.set $minus (i32.const 1))
        (local.set $i (i32.add (local.get $i) (i32.const 1))))
      (else (local.set $minus (i32.const 0))))
    (local.set $dstart (local.get $i))
    (block $dig
      (loop $lp
        (local.set $c (call $p-peek (local.get $data) (local.get $i)))
        (if (i32.and (i32.ge_u (local.get $c) (i32.const 48)) (i32.le_u (local.get $c) (i32.const 57)))
          (then
            (local.set $v (i32.add (i32.mul (local.get $v) (i32.const 10)) (i32.sub (local.get $c) (i32.const 48))))
            (local.set $i (i32.add (local.get $i) (i32.const 1)))
            (br $lp)))
        (br $dig)
      ))
    ;; A sign with no digits is not a number.
    (if (i32.eq (local.get $i) (local.get $dstart))
      (then (return (i32.const 0) (i32.const 0))))
    (if (local.get $minus)
      (then (local.set $v (i32.sub (i32.const 0) (local.get $v)))))
    (i32.const 1)
    (local.get $v)
  )

  ;; Whether a key byte array equals the literal at (off, len) in the data
  ;; segment.
  (func $lit-eq (param $key (ref $bytes)) (param $off i32) (param $len i32) (result i32)
    (local $i i32)
    (if (i32.ne (array.len (local.get $key)) (local.get $len))
      (then (return (i32.const 0))))
    (block $same
      (loop $lp
        (br_if $same (i32.ge_u (local.get $i) (local.get $len)))
        (if (i32.ne
              (array.get $bytes (local.get $key) (local.get $i))
              (i32.load8_u (i32.add (local.get $off) (local.get $i))))
          (then (return (i32.const 0))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $lp)
      ))
    (i32.const 1)
  )

  ;; Parse an entry's `data` blob into its metadata. The fields this host
  ;; supplies are `label` (the tile's display label), `lang` (its caption),
  ;; `icon` (its icon), and `width`/`height` (the window's default size). A
  ;; field the blob lacks keeps its fallback; a blob that will not parse
  ;; keeps the fallbacks parsed so far.
  (func $parse-meta (param $e (ref $entry)) (param $data (ref null $bytes))
    (local $a (ref $bytes))
    (local $key (ref null $bytes))
    (local $v (ref null $bytes))
    (local $ok i32)
    (local $val i32)
    (if (ref.is_null (local.get $data))
      (then (return)))
    (local.set $a (ref.cast (ref $bytes) (local.get $data)))
    (if (i32.eqz (array.len (local.get $a)))
      (then (return)))
    (global.set $p-cur (i32.const 0))
    (call $p-skip (local.get $a))
    (if (i32.eqz (call $p-eat (local.get $a) (i32.const 123))) ;; '{'
      (then (return)))
    (block $done
      (loop $lp
        (call $p-skip (local.get $a))
        (if (call $p-eat (local.get $a) (i32.const 125)) (then (br $done))) ;; '}'
        (local.set $key (call $parse-string (local.get $a)))
        (if (ref.is_null (local.get $key))
          (then (return)))
        (call $p-skip (local.get $a))
        (if (i32.eqz (call $p-eat (local.get $a) (i32.const 58))) (then (return))) ;; ':'
        (call $p-skip (local.get $a))
        (if (i32.eq (call $p-peek (local.get $a) (global.get $p-cur)) (i32.const 34))
          (then
            (local.set $v (call $parse-string (local.get $a)))
            (if (ref.is_null (local.get $v))
              (then (return)))
            (if (call $lit-eq (ref.cast (ref $bytes) (local.get $key)) (i32.const 65548) (i32.const 5)) (then (struct.set $entry $label (local.get $e) (local.get $v)))) ;; "label"
            (if (call $lit-eq (ref.cast (ref $bytes) (local.get $key)) (i32.const 65553) (i32.const 4)) (then (struct.set $entry $lang (local.get $e) (local.get $v)))) ;; "lang"
            (if (call $lit-eq (ref.cast (ref $bytes) (local.get $key)) (i32.const 65557) (i32.const 4)) (then (struct.set $entry $icon (local.get $e) (local.get $v)))) ;; "icon"
          )
          (else
            ;; The number returns (ok, value); a result-less block pops both.
            (block
              (call $parse-number (local.get $a))
              (local.set $val)
              (local.set $ok)
            )
            (if (local.get $ok)
              (then
                (if (call $lit-eq (ref.cast (ref $bytes) (local.get $key)) (i32.const 65561) (i32.const 5)) (then (struct.set $entry $width (local.get $e) (local.get $val)))) ;; "width"
                (if (call $lit-eq (ref.cast (ref $bytes) (local.get $key)) (i32.const 65566) (i32.const 6)) (then (struct.set $entry $height (local.get $e) (local.get $val)))) ;; "height"
              ))))
        (call $p-skip (local.get $a))
        (if (call $p-eat (local.get $a) (i32.const 44)) (then (br $lp))) ;; ','
        (if (i32.eqz (call $p-eat (local.get $a) (i32.const 125))) (then (return))) ;; '}'
        (br $lp)
      ))
  )

  ;; The registry, fetched once: each entry's name plus its parsed `data`.
  ;; The host writes the records and strings into the module's scratch region
  ;; (through `cabi_realloc`), so the component copies them into GC byte
  ;; arrays and frees the scratch.
  (func $fetch-entries (result (ref $entries))
    (local $desc i32)
    (local $ptr i32)
    (local $len i32)
    (local $arr (ref $entries))
    (local $i i32)
    (local $o i32)
    (local $name (ref $bytes))
    (local $data (ref $bytes))
    (local $e (ref $entry))
    (local.set $desc (call $arena-alloc (i32.const 8)))
    (call $list-entries (local.get $desc))
    (local.set $ptr (i32.load (local.get $desc)))
    (local.set $len (i32.load (i32.add (local.get $desc) (i32.const 4))))
    (local.set $arr (array.new_default $entries (local.get $len)))
    (block $done
      (loop $lp
        (br_if $done (i32.ge_u (local.get $i) (local.get $len)))
        (local.set $o (i32.add (local.get $ptr) (i32.mul (local.get $i) (i32.const 16))))
        (local.set $name (call $scratch-to-bytes (i32.load (local.get $o)) (i32.load (i32.add (local.get $o) (i32.const 4)))))
        (local.set $data (call $scratch-to-bytes (i32.load (i32.add (local.get $o) (i32.const 8))) (i32.load (i32.add (local.get $o) (i32.const 12)))))
        ;; The label starts as the name (the fallback display label).
        (local.set $e (struct.new $entry
          (ref.cast (ref null $bytes) (local.get $name))
          (ref.cast (ref null $bytes) (local.get $name))
          (ref.null $bytes)
          (ref.null $bytes)
          (i32.const 0)
          (i32.const 0)))
        (call $parse-meta (local.get $e) (ref.cast (ref null $bytes) (local.get $data)))
        (array.set $entries (local.get $arr) (local.get $i) (local.get $e))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $lp)
      ))
    ;; The scratch is a bump region the host filled; a free returns it to base.
    (drop (call $cabi-realloc (i32.const 0) (i32.const 0) (i32.const 1) (i32.const 0)))
    (local.get $arr)
  )

  ;; ======================================================= window management

  ;; a - b, saturating at 0 (the host's units never go negative).
  (func $ssub (param $a i32) (param $b i32) (result i32)
    (if (i32.lt_s (local.get $a) (local.get $b))
      (then (return (i32.const 0))))
    (i32.sub (local.get $a) (local.get $b))
  )

  (func $imax (param $a i32) (param $b i32) (result i32)
    (if (i32.lt_s (local.get $a) (local.get $b))
      (then (return (local.get $b))))
    (local.get $a)
  )

  (func $imin (param $a i32) (param $b i32) (result i32)
    (if (i32.gt_s (local.get $a) (local.get $b))
      (then (return (local.get $b))))
    (local.get $a)
  )

  ;; Clamp v to [lo, hi], tolerating a degenerate range (a window larger
  ;; than the viewport) by returning the lower bound.
  (func $clamp (param $v i32) (param $lo i32) (param $hi i32) (result i32)
    (if (i32.gt_s (local.get $lo) (local.get $hi))
      (then (return (local.get $lo))))
    (if (i32.lt_s (local.get $v) (local.get $lo))
      (then (return (local.get $lo))))
    (if (i32.gt_s (local.get $v) (local.get $hi))
      (then (return (local.get $hi))))
    (local.get $v)
  )

  ;; The index of the window with `handle`, or -1.
  (func $win-find (param $handle i32) (result i32)
    (local $arr (ref null $wins))
    (local $i i32)
    (local.set $arr (global.get $wins))
    (if (ref.is_null (local.get $arr))
      (then (return (i32.const -1))))
    (block $done
      (loop $lp
        (br_if $done (i32.ge_u (local.get $i) (global.get $win-len)))
        (if (i32.eq
              (struct.get $win $handle (ref.cast (ref $win) (array.get $wins (ref.cast (ref $wins) (local.get $arr)) (local.get $i))))
              (local.get $handle))
          (then (return (local.get $i))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $lp)
      ))
    (i32.const -1)
  )

  ;; Push a window (doubling the array when it is full).
  (func $win-push (param $w (ref $win))
    (local $arr (ref null $wins))
    (local $n2 (ref $wins))
    (local $len i32)
    (local $cap i32)
    (local $i i32)
    (local.set $arr (global.get $wins))
    (local.set $len (global.get $win-len))
    (if (ref.is_null (local.get $arr))
      (then
        (local.set $arr (ref.cast (ref null $wins) (array.new_default $wins (i32.const 2))))
        (local.set $len (i32.const 0))))
    (local.set $cap (array.len (ref.cast (ref $wins) (local.get $arr))))
    (if (i32.eq (local.get $len) (local.get $cap))
      (then
        (local.set $n2 (array.new_default $wins (i32.mul (local.get $cap) (i32.const 2))))
        (block $done
          (loop $lp
            (br_if $done (i32.ge_u (local.get $i) (local.get $len)))
            (array.set $wins (local.get $n2) (local.get $i)
              (array.get $wins (ref.cast (ref $wins) (local.get $arr)) (local.get $i)))
            (local.set $i (i32.add (local.get $i) (i32.const 1)))
            (br $lp)
          ))
        (local.set $arr (ref.cast (ref null $wins) (local.get $n2)))))
    (array.set $wins (ref.cast (ref $wins) (local.get $arr)) (local.get $len) (local.get $w))
    (global.set $wins (local.get $arr))
    (global.set $win-len (i32.add (local.get $len) (i32.const 1)))
  )

  ;; Drop every window with the handle (the host assigns unique handles, so
  ;; at most one): swap with the last, shorten, and re-check the same slot.
  (func $win-remove (param $handle i32)
    (local $arr (ref null $wins))
    (local $len i32)
    (local $i i32)
    (local.set $arr (global.get $wins))
    (if (ref.is_null (local.get $arr))
      (then (return)))
    (local.set $len (global.get $win-len))
    (block $done
      (loop $lp
        (br_if $done (i32.ge_u (local.get $i) (local.get $len)))
        (if (i32.eq
              (struct.get $win $handle (ref.cast (ref $win) (array.get $wins (ref.cast (ref $wins) (local.get $arr)) (local.get $i))))
              (local.get $handle))
          (then
            (array.set $wins (ref.cast (ref $wins) (local.get $arr)) (local.get $i)
              (array.get $wins (ref.cast (ref $wins) (local.get $arr)) (i32.sub (local.get $len) (i32.const 1))))
            (local.set $len (i32.sub (local.get $len) (i32.const 1)))
          )
          (else (local.set $i (i32.add (local.get $i) (i32.const 1)))))
        (br $lp)
      ))
    (global.set $win-len (local.get $len))
  )

  ;; The index of the window with the highest z, or -1.
  (func $win-top (result i32)
    (local $arr (ref null $wins))
    (local $i i32)
    (local $top i32)
    (local $topz i32)
    (local $z i32)
    (local.set $arr (global.get $wins))
    (if (ref.is_null (local.get $arr))
      (then (return (i32.const -1))))
    (local.set $top (i32.const -1))
    (local.set $topz (i32.const 0))
    (block $done
      (loop $lp
        (br_if $done (i32.ge_u (local.get $i) (global.get $win-len)))
        (local.set $z (struct.get $win $z (ref.cast (ref $win) (array.get $wins (ref.cast (ref $wins) (local.get $arr)) (local.get $i)))))
        (if (i32.or
              (i32.eq (local.get $top) (i32.const -1))
              (i32.gt_s (local.get $z) (local.get $topz)))
          (then
            (local.set $top (local.get $i))
            (local.set $topz (local.get $z))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $lp)
      ))
    (local.get $top)
  )

  ;; The lowest z among the open windows (10 when there are none).
  (func $win-min-z (result i32)
    (local $arr (ref null $wins))
    (local $i i32)
    (local $m i32)
    (local $z i32)
    (local.set $arr (global.get $wins))
    (if (ref.is_null (local.get $arr))
      (then (return (i32.const 10))))
    (local.set $m (i32.const -1))
    (block $done
      (loop $lp
        (br_if $done (i32.ge_u (local.get $i) (global.get $win-len)))
        (local.set $z (struct.get $win $z (ref.cast (ref $win) (array.get $wins (ref.cast (ref $wins) (local.get $arr)) (local.get $i)))))
        (if (i32.or
              (i32.lt_s (local.get $m) (i32.const 0))
              (i32.lt_s (local.get $z) (local.get $m)))
          (then (local.set $m (local.get $z))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $lp)
      ))
    (local.get $m)
  )

  ;; The window (its index) and the kind of node with `id`, if it is a
  ;; window node: kind 0 window, 1 title bar, 2 close, 3 content, 4 grip.
  ;; -1 when the id is not a window node.
  (func $window-for-id (param $id i32) (result i32 i32)
    (local $arr (ref null $wins))
    (local $i i32)
    (local $h i32)
    (local.set $arr (global.get $wins))
    (if (ref.is_null (local.get $arr))
      (then (return (i32.const -1) (i32.const 0))))
    (block $done
      (loop $lp
        (br_if $done (i32.ge_u (local.get $i) (global.get $win-len)))
        (local.set $h (struct.get $win $handle (ref.cast (ref $win) (array.get $wins (ref.cast (ref $wins) (local.get $arr)) (local.get $i)))))
        (if (i32.eq (local.get $id) (local.get $h))
          (then (return (local.get $i) (i32.const 3))))
        (if (i32.eq (local.get $id) (i32.add (local.get $h) (i32.const 100000)))
          (then (return (local.get $i) (i32.const 0))))
        (if (i32.or
              (i32.eq (local.get $id) (i32.add (local.get $h) (i32.const 200000)))
              (i32.eq (local.get $id) (i32.add (local.get $h) (i32.const 300000))))
          (then (return (local.get $i) (i32.const 1))))
        (if (i32.eq (local.get $id) (i32.add (local.get $h) (i32.const 400000)))
          (then (return (local.get $i) (i32.const 2))))
        (if (i32.eq (local.get $id) (i32.add (local.get $h) (i32.const 500000)))
          (then (return (local.get $i) (i32.const 4))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $lp)
      ))
    (i32.const -1)
    (i32.const 0)
  )

  ;; A new window's placement: cascade down-right, wrapping after 9,
  ;; clamped to the viewport. Skipped when the viewport is unknown.
  (func $cascade-placement (param $w i32) (param $h i32) (result i32 i32)
    (local $n i32)
    (local $left i32)
    (local $top i32)
    (local.set $n (i32.rem_u (global.get $cascade) (i32.const 9)))
    (global.set $cascade (i32.add (global.get $cascade) (i32.const 1)))
    (call $refresh-viewport)
    (if (i32.and (i32.eqz (global.get $vp-w)) (i32.eqz (global.get $vp-h)))
      (then
        ;; Unknown viewport: the raw cascade.
        (local.set $left (i32.add (i32.const 24) (i32.mul (local.get $n) (i32.const 28))))
        (local.set $top (i32.add (i32.const 48) (i32.mul (local.get $n) (i32.const 28))))
      )
      (else
        (local.set $left (call $clamp
          (i32.add (i32.const 24) (i32.mul (local.get $n) (i32.const 28)))
          (i32.const 8)
          (call $ssub (call $ssub (global.get $vp-w) (local.get $w)) (i32.const 8))))
        (local.set $top (call $clamp
          (i32.add (i32.const 48) (i32.mul (local.get $n) (i32.const 28)))
          (i32.const 38)
          (call $ssub (call $ssub (global.get $vp-h) (local.get $h)) (i32.const 8))))))
    (local.get $left)
    (local.get $top)
  )

  ;; Open the entry with index i: request a window from the host (a cascade
  ;; placement, the entry's default size), and record it. A window the host
  ;; cannot open is dropped.
  (func $open-entry-at (param $i i32)
    (local $entries (ref null $entries))
    (local $en (ref $entries))
    (local $e (ref $entry))
    (local $name (ref $bytes))
    (local $len i32)
    (local $buf i32)
    (local $i2 i32)
    (local $ret i32)
    (local $case i32)
    (local $w i32)
    (local $h i32)
    (local $x i32)
    (local $y i32)
    (local $win (ref $win))
    (local.set $entries (global.get $entries))
    (if (ref.is_null (local.get $entries))
      (then (return)))
    (local.set $en (ref.cast (ref $entries) (local.get $entries)))
    (if (i32.or
          (i32.lt_s (local.get $i) (i32.const 0))
          (i32.ge_u (local.get $i) (array.len (local.get $en))))
      (then (return)))
    (local.set $e (ref.cast (ref $entry) (array.get $entries (local.get $en) (local.get $i))))
    ;; The default size, for a dimension the entry's blob does not set.
    (local.set $w (if (result i32)
      (i32.gt_s (struct.get $entry $width (local.get $e)) (i32.const 0))
      (then (struct.get $entry $width (local.get $e)))
      (else (i32.const 300))))
    (local.set $h (if (result i32)
      (i32.gt_s (struct.get $entry $height (local.get $e)) (i32.const 0))
      (then (struct.get $entry $height (local.get $e)))
      (else (i32.const 380))))
    (block $place
      (call $cascade-placement (local.get $w) (local.get $h))
      (local.set $y)
      (local.set $x)
    )
    ;; The name buffer is in the module's own memory (the host reads it); it
    ;; is arena-allocated and dies with the next post.
    (local.set $name (ref.cast (ref $bytes) (struct.get $entry $name (local.get $e))))
    (local.set $len (array.len (local.get $name)))
    (local.set $buf (call $arena-alloc (local.get $len)))
    (block $done
      (loop $lp
        (br_if $done (i32.ge_u (local.get $i2) (local.get $len)))
        (i32.store8 (i32.add (local.get $buf) (local.get $i2))
          (array.get $bytes (local.get $name) (local.get $i2)))
        (local.set $i2 (i32.add (local.get $i2) (i32.const 1)))
        (br $lp)
      ))
    (local.set $ret (call $arena-alloc (i32.const 8)))
    (call $open-entry
      (local.get $buf) (local.get $len)
      (local.get $x) (local.get $y)
      (local.get $w) (local.get $h)
      (local.get $ret))
    ;; Ok: the host's window handle. Err: the window was not opened.
    (local.set $case (i32.load8_u (local.get $ret)))
    (if (i32.eqz (local.get $case))
      (then
        (local.set $win (struct.new $win
          (i32.load (i32.add (local.get $ret) (i32.const 4)))
          (struct.get $entry $label (local.get $e))
          (struct.get $entry $icon (local.get $e))
          (local.get $x)
          (local.get $y)
          (local.get $w)
          (local.get $h)
          (global.get $next-z)))
        (global.set $next-z (i32.add (global.get $next-z) (i32.const 1)))
        (call $win-push (local.get $win))))
  )

  ;; Close the window with `handle`: drop it and tell the host to free the
  ;; sandbox.
  (func $close-window (param $handle i32)
    (call $win-remove (local.get $handle))
    (call $close-entry (local.get $handle))
  )

  ;; Whether the (ptr, len) string in memory equals the literal at (off,
  ;; litlen) in the data segment.
  (func $key-eq (param $ptr i32) (param $len i32) (param $off i32) (param $litlen i32) (result i32)
    (local $i i32)
    (if (i32.ne (local.get $len) (local.get $litlen))
      (then (return (i32.const 0))))
    (block $same
      (loop $lp
        (br_if $same (i32.ge_u (local.get $i) (local.get $len)))
        (if (i32.ne
              (i32.load8_u (i32.add (local.get $ptr) (local.get $i)))
              (i32.load8_u (i32.add (local.get $off) (local.get $i))))
          (then (return (i32.const 0))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $lp)
      ))
    (i32.const 1)
  )

  ;; ================================================================ the world

  (func (export "view") (result i32)
    (local $arr (ref $flats))
    (local $n i32)
    (local $buf i32)
    (local $i i32)
    (local.set $arr (call $build))
    (global.set $last (ref.cast (ref null $flats) (local.get $arr)))
    (local.set $n (array.len (local.get $arr)))
    (local.set $buf (call $arena-alloc (i32.mul (local.get $n) (i32.const 60))))
    (block $done
      (loop $lp
        (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
        (call $emit-node
          (i32.add (local.get $buf) (i32.mul (local.get $i) (i32.const 60)))
          (ref.cast (ref $flat) (array.get $flats (local.get $arr) (local.get $i))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $lp)
      ))
    (call $make-list (local.get $buf) (local.get $n))
  )

  (func (export "changes") (result i32)
    (local $arr (ref $flats))
    (local $last (ref null $flats))
    (local $last-arr (ref $flats))
    (local $first i32)
    (local $n i32)
    (local $buf i32)
    (local $off i32)
    (local $i i32)
    (local $id i32)
    (local $f (ref $flat))
    (local.set $arr (call $build))
    (local.set $last (global.get $last))
    (local.set $first (call $nez (ref.is_null (local.get $last))))
    (global.set $last (ref.cast (ref null $flats) (local.get $arr)))
    (local.set $n (call $diff-count (local.get $arr) (local.get $last) (local.get $first)))
    (if (i32.eqz (local.get $n))
      (then (return (call $make-list (i32.const 0) (i32.const 0)))))
    (local.set $buf (call $arena-alloc (i32.mul (local.get $n) (i32.const 68))))
    ;; The changed nodes, in the new tree's pre-order.
    (block $done
      (loop $lp
        (br_if $done (i32.ge_u (local.get $i) (array.len (local.get $arr))))
        (local.set $f (ref.cast (ref $flat) (array.get $flats (local.get $arr) (local.get $i))))
        (if (call $nez (call $is-changed (local.get $f) (local.get $last) (local.get $first)))
          (then
            (call $emit-changed
              (i32.add (local.get $buf) (i32.mul (local.get $off) (i32.const 68)))
              (local.get $f))
            (local.set $off (i32.add (local.get $off) (i32.const 1)))
          ))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $lp)
      ))
    ;; The removed ids, in the last export's pre-order.
    (if (i32.eqz (local.get $first))
      (then
        (local.set $last-arr (ref.cast (ref $flats) (local.get $last)))
        (block $done
          (loop $lp
            (br_if $done (i32.ge_u (local.get $i) (array.len (local.get $last-arr))))
            (local.set $id (struct.get $flat $id (array.get $flats (local.get $last-arr) (local.get $i))))
            (if (i32.and
                  (call $nez (local.get $id))
                  (i32.eqz (call $present-in (local.get $arr) (local.get $id))))
              (then
                (call $emit-removed
                  (i32.add (local.get $buf) (i32.mul (local.get $off) (i32.const 68)))
                  (local.get $id))
                (local.set $off (i32.add (local.get $off) (i32.const 1)))
              ))
            (local.set $i (i32.add (local.get $i) (i32.const 1)))
            (br $lp)
          ))
      ))
    (call $make-list (local.get $buf) (local.get $n))
  )

  ;; A single click: the close marker closes its window; a tile selects it
  ;; (its ring).
  (func (export "on-click") (param $id i32)
    (local $idx i32)
    (local $kind i32)
    (local $handle i32)
    (local $i i32)
    (local $entries (ref null $entries))
    (block $r (call $window-for-id (local.get $id))
      (local.set $kind)
      (local.set $idx))
    (if (i32.and
          (i32.ne (local.get $idx) (i32.const -1))
          (i32.eq (local.get $kind) (i32.const 2)))
      (then
        (local.set $handle (struct.get $win $handle
          (ref.cast (ref $win) (array.get $wins (ref.cast (ref $wins) (global.get $wins)) (local.get $idx)))))
        (call $close-window (local.get $handle))
        (return)))
    (local.set $i (i32.sub (local.get $id) (i32.const 100)))
    (local.set $entries (global.get $entries))
    (if (ref.is_null (local.get $entries))
      (then (return)))
    (if (i32.and
          (i32.ge_s (local.get $i) (i32.const 0))
          (i32.lt_u (local.get $i) (array.len (ref.cast (ref $entries) (local.get $entries)))))
      (then (global.set $selected (local.get $i))))
  )

  ;; The desktop has no input; the buffer the host allocated is freed.
  (func (export "on-text") (param $id i32) (param $ptr i32) (param $len i32)
    (drop (call $cabi-realloc (local.get $ptr) (local.get $len) (i32.const 1) (i32.const 0)))
  )

  (func (export "on-tick"))

  ;; A double-clicked tile opens its entry (and takes the selection's ring).
  (func (export "on-double-click") (param $id i32)
    (local $i i32)
    (local $entries (ref null $entries))
    (local.set $i (i32.sub (local.get $id) (i32.const 100)))
    (local.set $entries (global.get $entries))
    (if (ref.is_null (local.get $entries))
      (then (return)))
    (if (i32.and
          (i32.ge_s (local.get $i) (i32.const 0))
          (i32.lt_u (local.get $i) (array.len (ref.cast (ref $entries) (local.get $entries)))))
      (then
        (global.set $selected (local.get $i))
        (call $open-entry-at (local.get $i))))
  )

  ;; The pointer was pressed over a window node: raise it, and start a drag
  ;; (title bar) or resize (grip).
  (func (export "on-pointer-down") (param $id i32) (param $x i32) (param $y i32)
    (local $idx i32)
    (local $kind i32)
    (local $w (ref $win))
    (block $r (call $window-for-id (local.get $id))
      (local.set $kind)
      (local.set $idx))
    (if (i32.eq (local.get $idx) (i32.const -1))
      (then (return)))
    (local.set $w (ref.cast (ref $win)
      (array.get $wins (ref.cast (ref $wins) (global.get $wins)) (local.get $idx))))
    (struct.set $win $z (local.get $w) (global.get $next-z))
    (global.set $next-z (i32.add (global.get $next-z) (i32.const 1)))
    (if (i32.eq (local.get $kind) (i32.const 1))
      (then
        ;; A title-bar drag: move, keeping the grab offset.
        (global.set $drag-active (i32.const 1))
        (global.set $drag-kind (i32.const 0))
        (global.set $drag-handle (struct.get $win $handle (local.get $w)))
        (global.set $drag-dx (i32.sub (local.get $x) (struct.get $win $x (local.get $w))))
        (global.set $drag-dy (i32.sub (local.get $y) (struct.get $win $y (local.get $w))))
        (global.set $drag-sx (i32.const 0))
        (global.set $drag-sy (i32.const 0))
        (global.set $drag-sw (i32.const 0))
        (global.set $drag-sh (i32.const 0))
      )
      (else
        (if (i32.eq (local.get $kind) (i32.const 4))
          (then
            ;; A grip drag: grow from the origin.
            (global.set $drag-active (i32.const 1))
            (global.set $drag-kind (i32.const 1))
            (global.set $drag-handle (struct.get $win $handle (local.get $w)))
            (global.set $drag-dx (i32.const 0))
            (global.set $drag-dy (i32.const 0))
            (global.set $drag-sx (local.get $x))
            (global.set $drag-sy (local.get $y))
            (global.set $drag-sw (struct.get $win $w (local.get $w)))
            (global.set $drag-sh (struct.get $win $h (local.get $w)))
          )
          (else
            ;; A press on the window, its content, or the close marker only
            ;; raises (the close is a click, not a drag).
            (global.set $drag-active (i32.const 0))))))
  )

  ;; The pointer moved while a button is held: continue the drag or resize.
  (func (export "on-pointer-move") (param $x i32) (param $y i32)
    (local $kind i32)
    (local $handle i32)
    (local $idx i32)
    (local $w (ref $win))
    (local $dx i32)
    (local $dy i32)
    (local $sx i32)
    (local $sy i32)
    (local $sw i32)
    (local $sh i32)
    (if (i32.eqz (global.get $drag-active))
      (then (return)))
    (local.set $kind (global.get $drag-kind))
    (local.set $handle (global.get $drag-handle))
    (local.set $dx (global.get $drag-dx))
    (local.set $dy (global.get $drag-dy))
    (local.set $sx (global.get $drag-sx))
    (local.set $sy (global.get $drag-sy))
    (local.set $sw (global.get $drag-sw))
    (local.set $sh (global.get $drag-sh))
    (call $refresh-viewport)
    (local.set $idx (call $win-find (local.get $handle)))
    (if (i32.eq (local.get $idx) (i32.const -1))
      (then (return)))
    (local.set $w (ref.cast (ref $win)
      (array.get $wins (ref.cast (ref $wins) (global.get $wins)) (local.get $idx))))
    (if (i32.eq (local.get $kind) (i32.const 0))
      (then
        ;; Move: the window under the grab, clamped to the viewport.
        (if (i32.and (i32.eqz (global.get $vp-w)) (i32.eqz (global.get $vp-h)))
          (then
            ;; Unknown viewport: no clamping.
            (struct.set $win $x (local.get $w)
              (call $imax (i32.sub (local.get $x) (local.get $dx)) (i32.const 0)))
            (struct.set $win $y (local.get $w)
              (call $imax (i32.sub (local.get $y) (local.get $dy)) (i32.const 0)))
          )
          (else
            (struct.set $win $x (local.get $w)
              (call $imax (i32.const 0)
                (call $clamp
                  (i32.sub (local.get $x) (local.get $dx))
                  (i32.sub (i32.const 72) (struct.get $win $w (local.get $w)))
                  (i32.sub (global.get $vp-w) (i32.const 64)))))
            (struct.set $win $y (local.get $w)
              (call $imax (i32.const 0)
                (call $clamp
                  (i32.sub (local.get $y) (local.get $dy))
                  (i32.const 34)
                  (i32.sub (global.get $vp-h) (i32.const 36)))))))
      )
      (else
        ;; Resize: grow from the origin, clamped to the viewport.
        (if (i32.or (i32.eqz (global.get $vp-w)) (i32.eqz (global.get $vp-h)))
          (then
            (struct.set $win $w (local.get $w)
              (call $imax (i32.const 220)
                (i32.add (local.get $sw)
                  (i32.sub (local.get $x) (local.get $sx)))))
            (struct.set $win $h (local.get $w)
              (call $imax (i32.const 260)
                (i32.add (local.get $sh)
                  (i32.sub (local.get $y) (local.get $sy)))))
          )
          (else
            (struct.set $win $w (local.get $w)
              (call $imin
                (call $imax (i32.const 220)
                  (i32.add (local.get $sw)
                    (i32.sub (local.get $x) (local.get $sx))))
                (call $imax (i32.const 220)
                  (call $ssub (call $ssub (global.get $vp-w) (struct.get $win $x (local.get $w))) (i32.const 8)))))
            (struct.set $win $h (local.get $w)
              (call $imin
                (call $imax (i32.const 260)
                  (i32.add (local.get $sh)
                    (i32.sub (local.get $y) (local.get $sy))))
                (call $imax (i32.const 260)
                  (call $ssub (call $ssub (global.get $vp-h) (struct.get $win $y (local.get $w))) (i32.const 8)))))))))
  )

  ;; The pointer was released: end the drag or resize.
  (func (export "on-pointer-up") (param $x i32) (param $y i32)
    (global.set $drag-active (i32.const 0))
  )

  ;; A key: Escape closes the frontmost window (the highest z), tab raises
  ;; it, shift+tab lowers it below the rest. The key name is a string in the
  ;; module's memory (the host allocates it), compared against the host's
  ;; names, then freed.
  (func (export "on-key") (param $ptr i32) (param $len i32)
    (local $top i32)
    (local $w (ref $win))
    (local $arr (ref $wins))
    (local $what i32)
    (local.set $what (i32.const 0))
    (local.set $top (call $win-top))
    (if (i32.ne (local.get $top) (i32.const -1))
      (then
        (local.set $arr (ref.cast (ref $wins) (global.get $wins)))
        (local.set $w (ref.cast (ref $win) (array.get $wins (local.get $arr) (local.get $top))))
        (if (call $key-eq (local.get $ptr) (local.get $len) (i32.const 65572) (i32.const 6))
          (then (local.set $what (i32.const 1)))) ;; Escape: close
        (if (i32.and
              (i32.eqz (local.get $what))
              (call $key-eq (local.get $ptr) (local.get $len) (i32.const 65579) (i32.const 3)))
          (then
            ;; Tab: raise the frontmost.
            (local.set $what (i32.const 2))
            (struct.set $win $z (local.get $w) (global.get $next-z))
            (global.set $next-z (i32.add (global.get $next-z) (i32.const 1)))))
        (if (i32.and
              (i32.eqz (local.get $what))
              (call $key-eq (local.get $ptr) (local.get $len) (i32.const 65582) (i32.const 9)))
          (then
            ;; Shift+tab: lower it below the rest.
            (local.set $what (i32.const 3))
            (struct.set $win $z (local.get $w)
              (call $ssub (call $win-min-z) (i32.const 1)))))
        ;; The close acts on the frontmost (the window above); it stays
        ;; inside the guard, where the window local is initialized.
        (if (i32.eq (local.get $what) (i32.const 1))
          (then (call $close-window (struct.get $win $handle (local.get $w)))))
      )
    )
    (drop (call $cabi-realloc (local.get $ptr) (local.get $len) (i32.const 1) (i32.const 0)))
  )

  ;; A host-side close (a host-drawn chrome): drop the window. The
  ;; just-an-app desktop closes via its own button, so this is a no-op for
  ;; it, but a host-chrome component would use it.
  (func (export "on-window-close") (param $window i32)
    (call $win-remove (local.get $window))
  )

)
