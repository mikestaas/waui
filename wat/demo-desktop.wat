;; The desktop, in hand-rolled WAT on the WASM GC — the same component as
;; `crates/demo-desktop` and `assembly/demo-desktop`, same `waui:ui/app`
;; world. See the sibling `demo-counter.wat` for the shared scaffolding:
;; the $flat struct and $flats array (the contract's flat, pre-ordered
;; node list, on the GC heap), the two-page memory (result arena, host
;; scratch, static literals), and the canonical-ABI records.
;;
;; The multi-component host's component: it owns the *logical* state — the
;; grid (built from the host's `list-entries` registry), the menu bar's
;; window count, and which windows are open — and exports it via
;; `view`/`changes`, as any component does. It *requests* a window through
;; the `open-entry` import when a tile is double-clicked, and drops a window
;; from its open set when the host reports a close. The windows themselves
;; (their chrome, geometry, and the embedded demo running in each) are
;; host-drawn, not part of this tree.
;;
;;   view:     a centered column: the menu bar (id 1: "waui" accent-large,
;;             and the window count id 2, neutral-small) and the grid (id 3:
;;             a column of rows of three tiles). Each tile (id 100+i) is a
;;             column of an icon (112px, id-less), a label (neutral-medium,
;;             id-less), and a caption (neutral-small, id-less).
;;   on-double-click: a tile opens its entry (the host places it; the size
;;             is the entry's default)
;;   on-window-close: a closed window is dropped from the open set
;;   on-click / on-text / on-tick: nothing (no input, no clock; a single
;;             click on a tile does nothing)
;;   changes:  the diff (the window count; the whole tree on first contact)
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

  (memory (export "memory") 2)

  ;; A label, as UTF-8 bytes on the GC heap. Each byte is held as an i32
  ;; (zero-extended): array.get returns an i32 for i32.store8, and the
  ;; element is mutable so array.set can fill it.
  (type $bytes (array (mut i32)))

  ;; One node of the export, as the contract's flat record plus its label:
  ;; the scalars mirror the ABI record's fields, the label is a GC byte
  ;; array (null = none; for an image it is the `src`). The contract's tree
  ;; is a flat, pre-ordered list, so the list is the tree.
  (type $flat (struct
    (field $kind i32)          ;; 0 text, 1 button, 2 container, 3 input, 4 image
    (field $id i32)            ;; 0 = none
    (field $parent i32)        ;; 0 = none
    (field $color i32)         ;; the WIT color token
    (field $size i32)          ;; the WIT size token
    (field $direction i32)     ;; containers: 0 row, 1 column
    (field $align i32)         ;; containers: 0 start, 1 center, 2 end
    (field $width i32)         ;; inputs: characters; images: host units
    (field $height i32)        ;; images: host units (0 = the host's default)
    (field $label (ref null $bytes))
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

  ;; The open windows' handles, as the host assigns them.
  (type $handles (array (mut i32)))

  ;; State: the registry (null = not fetched yet), the open windows, the
  ;; last export (the diff's reference).
  (global $entries (mut (ref null $entries)) (ref.null $entries))
  (global $open (mut (ref null $handles)) (ref.null $handles))
  (global $open-len (mut i32) (i32.const 0))
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

  ;; =========================================================== the records

  ;; The 36-byte node record for f at rec. Every byte a host reads is
  ;; written: the tag, both option flags (0 = none), the id and parent words
  ;; (when flagged), and the per-kind payload. An image carries no style:
  ;; its [28..36) is the width and the height. The canonical ABI's padding
  ;; is never written and never read (see web/glue.js), so no zero-fill.
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
  )

  ;; A 44-byte `changed` mutation carrying f, at rec: the tag byte, the
  ;; 36-byte node one word in, and the index after it. The tag and the index
  ;; are the only bytes of this record a host reads besides the node's.
  (func $emit-changed (param $rec i32) (param $f (ref $flat))
    (i32.store8 (local.get $rec) (i32.const 0))
    (call $emit-node (i32.add (local.get $rec) (i32.const 4)) (local.get $f))
    (i32.store (i32.add (local.get $rec) (i32.const 40))
      (struct.get $flat $index (local.get $f)))
  )

  ;; A 44-byte `removed` mutation for id, at rec: the tag byte and the id.
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
  ;; Every field is compared uniformly: a container's color/size/label, a
  ;; leaf's direction/align, and an image's width/height are zero where
  ;; unused, so the comparison holds across kinds.
  (func $fields-differ (param $f (ref $flat)) (param $r (ref $flat)) (result i32)
    (local $d i32)
    (local.set $d
      (i32.or
        (i32.ne (struct.get $flat $kind (local.get $f)) (struct.get $flat $kind (local.get $r)))
        (i32.ne (struct.get $flat $parent (local.get $f)) (struct.get $flat $parent (local.get $r)))))
    (if (call $nez (local.get $d))
      (then (return (i32.const 1))))
    (local.set $d
      (i32.or
        (i32.or
          (i32.or
            (i32.or
              (i32.ne (struct.get $flat $direction (local.get $f)) (struct.get $flat $direction (local.get $r)))
              (i32.ne (struct.get $flat $align (local.get $f)) (struct.get $flat $align (local.get $r))))
            (i32.ne (struct.get $flat $width (local.get $f)) (struct.get $flat $width (local.get $r))))
          (i32.ne (struct.get $flat $height (local.get $f)) (struct.get $flat $height (local.get $r))))
        (i32.or
          (i32.ne (struct.get $flat $color (local.get $f)) (struct.get $flat $color (local.get $r)))
          (i32.ne (struct.get $flat $size (local.get $f)) (struct.get $flat $size (local.get $r))))))
    (if (call $nez (local.get $d))
      (then (return (i32.const 1))))
    (call $labels-differ
      (struct.get $flat $label (local.get $f))
      (struct.get $flat $label (local.get $r)))
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

  ;; The desktop's export size: the root, the menu bar (three), the grid,
  ;; one node per row, and one node per tile plus its children (an icon, a
  ;; label, and a caption, each when the entry's blob supplies it).
  (func $node-count (param $entries (ref $entries)) (result i32)
    (local $n i32)
    (local $i i32)
    (local $total i32)
    (local $kids i32)
    (local.set $n (array.len (local.get $entries)))
    (local.set $total (i32.const 5)) ;; root, menu, "waui", the count, grid
    (local.set $total (i32.add (local.get $total) (i32.div_u (i32.add (local.get $n) (i32.const 2)) (i32.const 3))))
    (block $done
      (loop $lp
        (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
        (local.set $kids (i32.const 1)) ;; the label, always
        (if (i32.eqz (ref.is_null (struct.get $entry $icon (ref.cast (ref $entry) (array.get $entries (local.get $entries) (local.get $i))))))
          (then (local.set $kids (i32.add (local.get $kids) (i32.const 1)))))
        (if (i32.eqz (ref.is_null (struct.get $entry $lang (ref.cast (ref $entry) (array.get $entries (local.get $entries) (local.get $i))))))
          (then (local.set $kids (i32.add (local.get $kids) (i32.const 1)))))
        (local.set $total (i32.add (local.get $total) (i32.add (local.get $kids) (i32.const 1))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $lp)
      ))
    (local.get $total)
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

  ;; Add a handle to the open set (doubling the array when it is full).
  (func $open-push (param $handle i32)
    (local $arr (ref null $handles))
    (local $n2 (ref $handles))
    (local $len i32)
    (local $cap i32)
    (local $i i32)
    (local.set $arr (global.get $open))
    (local.set $len (global.get $open-len))
    (if (ref.is_null (local.get $arr))
      (then
        (local.set $arr (ref.cast (ref null $handles) (array.new_default $handles (i32.const 2))))
        (local.set $len (i32.const 0))))
    (local.set $cap (array.len (ref.cast (ref $handles) (local.get $arr))))
    (if (i32.eq (local.get $len) (local.get $cap))
      (then
        (local.set $n2 (array.new_default $handles (i32.mul (local.get $cap) (i32.const 2))))
        (block $done
          (loop $lp
            (br_if $done (i32.ge_u (local.get $i) (local.get $len)))
            (array.set $handles (local.get $n2) (local.get $i)
              (array.get $handles (ref.cast (ref $handles) (local.get $arr)) (local.get $i)))
            (local.set $i (i32.add (local.get $i) (i32.const 1)))
            (br $lp)
          ))
        (local.set $arr (ref.cast (ref null $handles) (local.get $n2)))))
    (array.set $handles (ref.cast (ref $handles) (local.get $arr)) (local.get $len) (local.get $handle))
    (global.set $open (local.get $arr))
    (global.set $open-len (i32.add (local.get $len) (i32.const 1)))
  )

  ;; Drop every window with the handle (the host assigns unique handles, so
  ;; at most one): swap with the last, shorten, and re-check the same slot.
  (func $open-remove (param $window i32)
    (local $arr (ref null $handles))
    (local $len i32)
    (local $i i32)
    (local.set $arr (global.get $open))
    (if (ref.is_null (local.get $arr))
      (then (return)))
    (local.set $len (global.get $open-len))
    (block $done
      (loop $lp
        (br_if $done (i32.ge_u (local.get $i) (local.get $len)))
        (if (i32.eq
              (array.get $handles (ref.cast (ref $handles) (local.get $arr)) (local.get $i))
              (local.get $window))
          (then
            (array.set $handles (ref.cast (ref $handles) (local.get $arr)) (local.get $i)
              (array.get $handles (ref.cast (ref $handles) (local.get $arr)) (i32.sub (local.get $len) (i32.const 1))))
            (local.set $len (i32.sub (local.get $len) (i32.const 1)))
          )
          (else (local.set $i (i32.add (local.get $i) (i32.const 1)))))
        (br $lp)
      ))
    (global.set $open-len (local.get $len))
  )

  ;; The desktop's whole export, as a flat, pre-ordered list: the root
  ;; column, the menu bar (the "waui" word and the window count), and the
  ;; grid (a column of rows of three tiles; each tile an icon, a label, and
  ;; a caption, each when the entry's blob supplies it).
  (func $build (result (ref $flats))
    (local $entries (ref null $entries))
    (local $en (ref $entries))
    (local $arr (ref $flats))
    (local $n i32)
    (local $n-rows i32)
    (local $pos i32)
    (local $r i32)
    (local $c i32)
    (local $i i32)
    (local $e (ref $entry))
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
    (local.set $arr (array.new_default $flats (call $node-count (local.get $en))))
    ;; The root column (id-less; it has children only through its two id'd
    ;; children, so it keeps no id of its own).
    (array.set $flats (local.get $arr) (i32.const 0)
      (struct.new $flat
        (i32.const 2) (i32.const 0) (i32.const 0)
        (i32.const 0) (i32.const 0) (i32.const 1) (i32.const 1)
        (i32.const 0) (i32.const 0)
        (ref.null $bytes) (i32.const 0)))
    ;; The menu bar (1): the "waui" word, and the window count (2).
    (array.set $flats (local.get $arr) (i32.const 1)
      (struct.new $flat
        (i32.const 2) (i32.const 1) (i32.const 0)
        (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 1)
        (i32.const 0) (i32.const 0)
        (ref.null $bytes) (i32.const 0)))
    (array.set $flats (local.get $arr) (i32.const 2)
      (struct.new $flat
        (i32.const 0) (i32.const 0) (i32.const 1)
        (i32.const 1) (i32.const 2) (i32.const 0) (i32.const 0)
        (i32.const 0) (i32.const 0)
        (call $label (i32.const 65536) (i32.const 4)) (i32.const 0)))
    (array.set $flats (local.get $arr) (i32.const 3)
      (struct.new $flat
        (i32.const 0) (i32.const 2) (i32.const 1)
        (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
        (i32.const 0) (i32.const 0)
        (call $count-label (global.get $open-len)) (i32.const 1)))
    ;; The grid (3): a column of the rows.
    (array.set $flats (local.get $arr) (i32.const 4)
      (struct.new $flat
        (i32.const 2) (i32.const 3) (i32.const 0)
        (i32.const 0) (i32.const 0) (i32.const 1) (i32.const 1)
        (i32.const 0) (i32.const 0)
        (ref.null $bytes) (i32.const 1)))
    (local.set $pos (i32.const 5))
    (block $rows
      (loop $lp-rows
        (br_if $rows (i32.ge_u (local.get $r) (local.get $n-rows)))
        ;; Row 10+r.
        (array.set $flats (local.get $arr) (local.get $pos)
          (struct.new $flat
            (i32.const 2) (i32.add (i32.const 10) (local.get $r)) (i32.const 3)
            (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 1)
            (i32.const 0) (i32.const 0)
            (ref.null $bytes) (local.get $r)))
        (local.set $pos (i32.add (local.get $pos) (i32.const 1)))
        (local.set $c (i32.const 0))
        (block $cols
          (loop $lp-cols
            (br_if $cols (i32.ge_u (local.get $c) (i32.const 3)))
            (local.set $i (i32.add (i32.mul (local.get $r) (i32.const 3)) (local.get $c)))
            (br_if $cols (i32.ge_u (local.get $i) (local.get $n)))
            (local.set $e (ref.cast (ref $entry) (array.get $entries (local.get $en) (local.get $i))))
            ;; Tile 100+i: a double-clickable column.
            (array.set $flats (local.get $arr) (local.get $pos)
              (struct.new $flat
                (i32.const 2) (i32.add (i32.const 100) (local.get $i)) (i32.add (i32.const 10) (local.get $r))
                (i32.const 0) (i32.const 0) (i32.const 1) (i32.const 1)
                (i32.const 0) (i32.const 0)
                (ref.null $bytes) (local.get $c)))
            (local.set $pos (i32.add (local.get $pos) (i32.const 1)))
            (local.set $has-icon (i32.eqz (ref.is_null (struct.get $entry $icon (local.get $e)))))
            (local.set $has-lang (i32.eqz (ref.is_null (struct.get $entry $lang (local.get $e)))))
            (if (local.get $has-icon)
              (then
                ;; The icon: 112px, the web desktop's tile size.
                (array.set $flats (local.get $arr) (local.get $pos)
                  (struct.new $flat
                    (i32.const 4) (i32.const 0) (i32.add (i32.const 100) (local.get $i))
                    (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
                    (i32.const 112) (i32.const 112)
                    (struct.get $entry $icon (local.get $e)) (i32.const 0)))
                (local.set $pos (i32.add (local.get $pos) (i32.const 1)))))
            (array.set $flats (local.get $arr) (local.get $pos)
              (struct.new $flat
                (i32.const 0) (i32.const 0) (i32.add (i32.const 100) (local.get $i))
                (i32.const 0) (i32.const 1) (i32.const 0) (i32.const 0)
                (i32.const 0) (i32.const 0)
                (struct.get $entry $label (local.get $e)) (local.get $has-icon)))
            (local.set $pos (i32.add (local.get $pos) (i32.const 1)))
            (if (local.get $has-lang)
              (then
                (array.set $flats (local.get $arr) (local.get $pos)
                  (struct.new $flat
                    (i32.const 0) (i32.const 0) (i32.add (i32.const 100) (local.get $i))
                    (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0)
                    (i32.const 0) (i32.const 0)
                    (struct.get $entry $lang (local.get $e))
                    (i32.add (local.get $has-icon) (i32.const 1))))
                (local.set $pos (i32.add (local.get $pos) (i32.const 1)))))
            (local.set $c (i32.add (local.get $c) (i32.const 1)))
            (br $lp-cols)
          ))
        (local.set $r (i32.add (local.get $r) (i32.const 1)))
        (br $lp-rows)
      ))
    (local.get $arr)
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
    (local.set $buf (call $arena-alloc (i32.mul (local.get $n) (i32.const 36))))
    (block $done
      (loop $lp
        (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
        (call $emit-node
          (i32.add (local.get $buf) (i32.mul (local.get $i) (i32.const 36)))
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
    (local.set $buf (call $arena-alloc (i32.mul (local.get $n) (i32.const 44))))
    ;; The changed nodes, in the new tree's pre-order.
    (block $done
      (loop $lp
        (br_if $done (i32.ge_u (local.get $i) (array.len (local.get $arr))))
        (local.set $f (ref.cast (ref $flat) (array.get $flats (local.get $arr) (local.get $i))))
        (if (call $nez (call $is-changed (local.get $f) (local.get $last) (local.get $first)))
          (then
            (call $emit-changed
              (i32.add (local.get $buf) (i32.mul (local.get $off) (i32.const 44)))
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
                  (i32.add (local.get $buf) (i32.mul (local.get $off) (i32.const 44)))
                  (local.get $id))
                (local.set $off (i32.add (local.get $off) (i32.const 1)))
              ))
            (local.set $i (i32.add (local.get $i) (i32.const 1)))
            (br $lp)
          ))
      ))
    (call $make-list (local.get $buf) (local.get $n))
  )

  ;; A single click on a tile does nothing (the double-click opens), and the
  ;; desktop has no other activatable widget.
  (func (export "on-click") (param $id i32))

  ;; The desktop has no input; the buffer the host allocated is freed.
  (func (export "on-text") (param $id i32) (param $ptr i32) (param $len i32)
    (drop (call $cabi-realloc (local.get $ptr) (local.get $len) (i32.const 1) (i32.const 0)))
  )

  (func (export "on-tick"))

  ;; A tile was double-clicked: open the entry it represents. The placement
  ;; is left to the host (0, 0 = its default cascade); the size is the
  ;; entry's default. A window the host cannot open is dropped.
  (func (export "on-double-click") (param $id i32)
    (local $i i32)
    (local $entries (ref null $entries))
    (local $en (ref $entries))
    (local $e (ref $entry))
    (local $name (ref $bytes))
    (local $len i32)
    (local $buf i32)
    (local $ret i32)
    (local $case i32)
    (local $i2 i32)
    (local.set $i (i32.sub (local.get $id) (i32.const 100)))
    (local.set $entries (global.get $entries))
    (if (ref.is_null (local.get $entries))
      (then (return)))
    ;; Past the guard the registry is non-null; the type system needs the cast.
    (local.set $en (ref.cast (ref $entries) (local.get $entries)))
    (if (i32.or
          (i32.lt_s (local.get $i) (i32.const 0))
          (i32.ge_u (local.get $i) (array.len (local.get $en))))
      (then (return)))
    (local.set $e (ref.cast (ref $entry) (array.get $entries (local.get $en) (local.get $i))))
    (local.set $name (ref.cast (ref $bytes) (struct.get $entry $name (local.get $e))))
    (local.set $len (array.len (local.get $name)))
    ;; The name buffer is in the module's own memory (the host reads it); it
    ;; is arena-allocated and dies with the next post.
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
      (i32.const 0) (i32.const 0)
      (struct.get $entry $width (local.get $e))
      (struct.get $entry $height (local.get $e))
      (local.get $ret))
    ;; Ok: the host's window handle. Err: the window was not opened.
    (local.set $case (i32.load8_u (local.get $ret)))
    (if (i32.eqz (local.get $case))
      (then
        (call $open-push (i32.load (i32.add (local.get $ret) (i32.const 4))))))
  )

  ;; A window was closed by the user: drop it from the open set.
  (func (export "on-window-close") (param $window i32)
    (call $open-remove (local.get $window))
  )
)
