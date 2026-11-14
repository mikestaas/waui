;; A stopwatch, in hand-rolled WAT on the WASM GC — the same component as
;; `crates/demo-stopwatch` and `assembly/demo-stopwatch`, same `waui:ui/app`
;; world. See the sibling `demo-counter.wat` for the shared scaffolding:
;; the $flat struct and $flats array (the contract's flat, pre-ordered
;; node list, on the GC heap), the two-page memory (result arena, host
;; scratch, static literals), and the canonical-ABI records.
;;
;; Exercises the world's import side: the component polls `now()` (a host
;; clock) and renders elapsed time; it requests its own refresh —
;; `schedule` at 250ms while running, 0 while paused — so the host wakes it
;; only while running.
;;
;; Its diff is state-dependent: while running, the readout changes with the
;; clock, so each `changes` reports it; while paused, it reports nothing.
;;
;;   view:     a column (id 1) with the readout (id 2, "N ms", large;
;;             accent while running) and the button (id 3, "start"/pause)
;;   on-click: 3 starts or pauses
;;   on-text:  nothing (a stopwatch has no input)
;;   on-tick:  nothing (the reading is recomputed from the clock on demand)
;;   changes:  the changed readout and/or button (the whole tree on first
;;             contact)
;;
;; The elapsed reading is sampled once per export, so one exported list
;; never mixes two clock readings.
(module
  (import "$root" "now" (func $now (result i64)))
  (import "$root" "schedule" (func $schedule (param i64)))
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
    (field $kind i32)          ;; 0 text, 1 button, 2 container, 3 input
    (field $id i32)            ;; 0 = none
    (field $parent i32)        ;; 0 = none
    (field $color i32)         ;; the WIT color token
    (field $size i32)          ;; the WIT size token
    (field $direction i32)     ;; containers: 0 row, 1 column
    (field $align i32)         ;; containers: 0 start, 1 center, 2 end
    (field $width i32)         ;; inputs: characters (0 = the host's default)
    (field $label (ref null $bytes))
    (field $index i32)         ;; position among the parent's children
  ))
  (type $flats (array (mut (ref null $flat))))

  ;; State: the clock reading when the run started (0 = never started), the
  ;; reading held while paused, and the run flag; the last export (the diff's
  ;; reference).
  (global $start (mut i64) (i64.const 0))
  (global $frozen (mut i64) (i64.const 0))
  (global $running (mut i32) (i32.const 0))
  (global $last (mut (ref null $flats)) (ref.null $flats))

  ;; The result arena's cursor (8-aligned by construction).
  (global $arena (mut i32) (i32.const 0))
  ;; The host scratch region's cursor (the top 4 KiB of page 0).
  (global $scratch (mut i32) (i32.const 61440))

  ;; The string literals, from page 1.
  (data $ms (i32.const 65536) " ms")
  (data $start-label (i32.const 65539) "start")
  (data $pause-label (i32.const 65544) "pause")

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
    (i32.ne (local.get $x) (i32.const 0)))

  ;; The canonical ABI's cabi_realloc: a buffer in the module's own memory,
  ;; for a string parameter. A bump over the scratch region: one buffer is
  ;; live at a time (the host allocates, the callee frees), so a free
  ;; returns the region to its base. A buffer past the region traps.
  (func $cabi-realloc (export "cabi_realloc")
    (param $old-ptr i32) (param $old-len i32) (param $new-len i32) (param $align i32)
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

  ;; "N ms" for the elapsed milliseconds, in a fresh GC byte array. The
  ;; magnitude is an unsigned bit pattern; the digit count is a do-while
  ;; (at least one digit: 0 is "0").
  (func $ms-label (param $v i64) (result (ref $bytes))
    (local $t i64)
    (local $count i32)
    (local $arr (ref $bytes))
    (local $i i32)
    (local.set $t (local.get $v))
    (local.set $count (i32.const 0))
    (block $done
      (loop $lp
        (local.set $count (i32.add (local.get $count) (i32.const 1)))
        (local.set $t (i64.div_u (local.get $t) (i64.const 10)))
        (br_if $done (i64.eqz (local.get $t)))
        (br $lp)
      ))
    (local.set $arr (array.new_default $bytes (i32.add (local.get $count) (i32.const 3))))
    ;; The digits, from the tail. Each is 0-9, so the i64 remainder wraps
    ;; back to an i32 exactly.
    (local.set $t (local.get $v))
    (local.set $i (i32.sub (local.get $count) (i32.const 1)))
    (block $done
      (loop $lp
        (br_if $done (i32.lt_s (local.get $i) (i32.const 0)))
        (array.set $bytes (local.get $arr) (local.get $i)
          (i32.add (i32.const 48)
            (i32.wrap_i64 (i64.rem_u (local.get $t) (i64.const 10)))))
        (local.set $t (i64.div_u (local.get $t) (i64.const 10)))
        (local.set $i (i32.sub (local.get $i) (i32.const 1)))
        (br $lp)
      ))
    ;; The " ms" suffix, from the literal.
    (local.set $i (i32.const 0))
    (block $done
      (loop $lp
        (br_if $done (i32.ge_u (local.get $i) (i32.const 3)))
        (array.set $bytes (local.get $arr)
          (i32.add (local.get $count) (local.get $i))
          (i32.load8_u (i32.add (i32.const 65536) (local.get $i))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $lp)
      ))
    (local.get $arr)
  )

  ;; Copy the node's label bytes into the arena; store the (ptr, len) at the
  ;; record's string slot (+20/+24). A null or empty label stores (0, 0).
  (func $emit-label (param $rec i32) (param $label (ref null $bytes))
    (local $a (ref $bytes))
    (local $len i32)
    (local $i i32)
    (local $dst i32)
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
  ;; (when flagged), the per-kind payload, and — for an input — the width
  ;; word. The canonical ABI's padding is never written and never read (see
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
      )
      (else
        (call $emit-label (local.get $rec) (struct.get $flat $label (local.get $f)))
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
  ;; leaf's direction/align, and a non-input's width are zero, so the
  ;; comparison holds across kinds.
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
            (i32.ne (struct.get $flat $direction (local.get $f)) (struct.get $flat $direction (local.get $r)))
            (i32.ne (struct.get $flat $align (local.get $f)) (struct.get $flat $align (local.get $r))))
          (i32.ne (struct.get $flat $width (local.get $f)) (struct.get $flat $width (local.get $r))))
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

  ;; The elapsed milliseconds, as the component currently sees it: while
  ;; running, the clock minus the start (saturating); while paused, the held
  ;; reading. The clock is sampled once.
  (func $elapsed (result i64)
    (local $t i64)
    (if (global.get $running)
      (then
        (if (i64.eqz (global.get $start))
          (then (return (i64.const 0))))
        (local.set $t (call $now))
        (if (i64.lt_s (local.get $t) (global.get $start))
          (then (return (i64.const 0))))
        (return (i64.sub (local.get $t) (global.get $start)))
      )
      (else (return (global.get $frozen)))
    )
    (unreachable)
  )

  ;; The stopwatch's whole export, as a flat, pre-ordered list: the column
  ;; (1), the readout (2), the button (3).
  (func $build (result (ref $flats))
    (local $arr (ref $flats))
    (local $running i32)
    (local.set $running (global.get $running))
    (local.set $arr (array.new_default $flats (i32.const 3)))
    (array.set $flats (local.get $arr) (i32.const 0)
      (struct.new $flat
        (i32.const 2) (i32.const 1) (i32.const 0)
        (i32.const 0) (i32.const 0) (i32.const 1) (i32.const 1)
        (i32.const 0)
        (ref.null $bytes) (i32.const 0)))
    ;; The readout takes the accent color while running; the button's label
    ;; and color name the action.
    (array.set $flats (local.get $arr) (i32.const 1)
      (struct.new $flat
        (i32.const 0) (i32.const 2) (i32.const 1)
        (if (result i32) (local.get $running) (then (i32.const 1)) (else (i32.const 0)))
        (i32.const 2) (i32.const 0) (i32.const 1)
        (i32.const 0)
        (call $ms-label (call $elapsed)) (i32.const 0)))
    (array.set $flats (local.get $arr) (i32.const 2)
      (struct.new $flat
        (i32.const 1) (i32.const 3) (i32.const 1)
        (if (result i32) (local.get $running) (then (i32.const 4)) (else (i32.const 2)))
        (i32.const 1) (i32.const 0) (i32.const 1)
        (i32.const 0)
        (if (result (ref $bytes)) (local.get $running)
          (then (call $label (i32.const 65544) (i32.const 5)))
          (else (call $label (i32.const 65539) (i32.const 5))))
        (i32.const 1)))
    (local.get $arr)
  )

  ;; ================================================================ the world

  (func (export "view") (result i32)
    (local $arr (ref $flats))
    (local $n i32)
    (local $buf i32)
    (local $i i32)
    (local.set $arr (call $build))
    (global.set $last (local.get $arr))
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
    (global.set $last (local.get $arr))
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

  ;; 3 starts or pauses. The clock is read once and used for both the new
  ;; start and the frozen reading.
  (func (export "on-click") (param $id i32)
    (local $t i64)
    (if (i32.eq (local.get $id) (i32.const 3))
      (then
        (local.set $t (call $now))
        (if (global.get $running)
          (then
            (if (i64.ne (global.get $start) (i64.const 0))
              (then
                (if (i64.lt_s (local.get $t) (global.get $start))
                  (then (global.set $frozen (i64.const 0)))
                  (else (global.set $frozen (i64.sub (local.get $t) (global.get $start)))))
              ))
            (global.set $running (i32.const 0))
          )
          (else
            (global.set $start (local.get $t))
            (global.set $running (i32.const 1))
          ))
        (call $schedule
          (if (result i64) (global.get $running)
            (then (i64.const 250))
            (else (i64.const 0))))
      ))
  )

  ;; The host wakes a running stopwatch; the reading is recomputed from the
  ;; clock on the next export, so there is nothing to do.
  (func (export "on-tick"))

  ;; No input, so the buffer is unused; it is freed.
  (func (export "on-text") (param $id i32) (param $ptr i32) (param $len i32)
    (drop (call $cabi-realloc (local.get $ptr) (local.get $len) (i32.const 0) (i32.const 1)))
  )
)
