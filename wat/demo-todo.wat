;; A todo list, in hand-rolled WAT on the WASM GC — the same component as
;; `crates/demo-todo` and `assembly/demo-todo`, same `waui:ui/app` world.
;; See the sibling `demo-counter.wat` for the shared scaffolding: the $flat
;; struct and $flats array (the contract's flat, pre-ordered node list, on
;; the GC heap), the two-page memory (result arena, host scratch, static
;; literals), and the canonical-ABI records. The structural demo: the
;; contract's diff expresses add, remove, and re-parent.
;;
;; The footer's "add" button uses the text input: it adds a todo with the
;; input's text and clears the field, or — when the field is empty — cycles
;; a preset list of labels. The input's text is component state, stored on
;; each edit (`on-text`).
;;
;; **The ids.** A todo created i-th gets its row 100+4i while active and
;; 500+4i while done; its checkbox, label, and delete keep 101+4i, 102+4i,
;; and 103+4i in both columns. The row's id changes with the column, because
;; a container move is a subtree removal + re-add; the children's ids do not,
;; so a move re-parents them. The static structure is 1 (the outer column),
;; 2 (the header), 3 (the left-align wrapper), 4 and 5 (the lists), and 6
;; (the footer); the footer's count, input, add, and clear are 7, 8, 9, and
;; 10.
;;
;; **The order.** Both columns are newest-first, so `view()` and the
;; retained list agree.
;;
;;   on-click: 9 adds, 10 clears the done column, 101+4i toggles, 103+4i
;;             deletes
;;   on-text:  8 stores the input's text
;;   on-tick:  nothing (it changes only on a click or a text edit)
(module
  (memory (export "memory") 2)

  ;; A label, as UTF-8 bytes on the GC heap. Each byte is held as an i32
  ;; (zero-extended): array.get returns an i32 for i32.store8, and the element
  ;; is mutable so array.set can fill it.
  (type $bytes (array (mut i32)))

  ;; One todo: its creation index (the source of its node ids, never reused),
  ;; its label, and the column it sits in.
  (type $todo (struct
    (field $idx i32)
    (field $label (ref $bytes))
    (field $done (mut i32))
  ))
  (type $todos (array (mut (ref null $todo))))

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

  ;; State: the two lists (newest first), the input's text (null = empty),
  ;; the next creation index and preset (the seed consumed the first three),
  ;; the seed flag, and the last export (the diff's reference).
  (global $active (mut (ref null $todos)) (ref.null $todos))
  (global $done (mut (ref null $todos)) (ref.null $todos))
  (global $input (mut (ref null $bytes)) (ref.null $bytes))
  (global $next-idx (mut i32) (i32.const 3))
  (global $preset-idx (mut i32) (i32.const 3))
  (global $inited (mut i32) (i32.const 0))
  (global $last (mut (ref null $flats)) (ref.null $flats))

  ;; The result arena's cursor (8-aligned by construction).
  (global $arena (mut i32) (i32.const 0))
  ;; The host scratch region's cursor (the top 4 KiB of page 0).
  (global $scratch (mut i32) (i32.const 61440))

  ;; The string literals, from page 1.
  (data $header (i32.const 65536) "todos")
  (data $circle (i32.const 65541) "○")
  (data $check (i32.const 65544) "✓")
  (data $cross (i32.const 65547) "✕")
  (data $left (i32.const 65550) " left")
  (data $add (i32.const 65555) "add")
  (data $clear (i32.const 65558) "clear done")
  ;; The presets the "add" button cycles when the input is empty.
  (data $p0 (i32.const 65568) "buy milk")
  (data $p1 (i32.const 65576) "walk the dog")
  (data $p2 (i32.const 65588) "water plants")
  (data $p3 (i32.const 65600) "write code")
  (data $p4 (i32.const 65610) "ship the demo")
  (data $p5 (i32.const 65623) "take a break")

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

  ;; The preset with index i (0-5), as a fresh GC byte array.
  (func $preset (param $i i32) (result (ref $bytes))
    (local $o i32)
    (local $l i32)
    (if (i32.eq (local.get $i) (i32.const 0))
      (then
        (local.set $o (i32.const 65568))
        (local.set $l (i32.const 8))
      ))
    (if (i32.eq (local.get $i) (i32.const 1))
      (then
        (local.set $o (i32.const 65576))
        (local.set $l (i32.const 12))
      ))
    (if (i32.eq (local.get $i) (i32.const 2))
      (then
        (local.set $o (i32.const 65588))
        (local.set $l (i32.const 12))
      ))
    (if (i32.eq (local.get $i) (i32.const 3))
      (then
        (local.set $o (i32.const 65600))
        (local.set $l (i32.const 10))
      ))
    (if (i32.eq (local.get $i) (i32.const 4))
      (then
        (local.set $o (i32.const 65610))
        (local.set $l (i32.const 13))
      ))
    (if (i32.eq (local.get $i) (i32.const 5))
      (then
        (local.set $o (i32.const 65623))
        (local.set $l (i32.const 12))
      ))
    (call $label (local.get $o) (local.get $l))
  )

  ;; v as decimal ASCII, in a fresh GC byte array. The magnitude is an
  ;; unsigned bit pattern: for the most negative value the negation wraps to
  ;; its correct magnitude, so no special case is needed.
  (func $itoa (param $v i32) (result (ref $bytes))
    (local $mag i32)
    (local $t i32)
    (local $minus i32)
    (local $count i32)
    (local $arr (ref $bytes))
    (local $i i32)
    (if (i32.lt_s (local.get $v) (i32.const 0))
      (then
        (local.set $minus (i32.const 1))
        (local.set $mag (i32.sub (i32.const 0) (local.get $v)))
      )
      (else
        (local.set $minus (i32.const 0))
        (local.set $mag (local.get $v))
      ))
    ;; Digit count as a do-while: at least one digit (v = 0 is "0"), one per
    ;; division until the magnitude is exhausted.
    (local.set $t (local.get $mag))
    (local.set $count (i32.const 0))
    (block $done
      (loop $lp
        (local.set $count (i32.add (local.get $count) (i32.const 1)))
        (local.set $t (i32.div_u (local.get $t) (i32.const 10)))
        (br_if $done (i32.eqz (local.get $t)))
        (br $lp)
      ))
    (local.set $arr (array.new_default $bytes (i32.add (local.get $minus) (local.get $count))))
    (if (local.get $minus)
      (then (array.set $bytes (local.get $arr) (i32.const 0) (i32.const 45))))
    (local.set $i (i32.sub (i32.add (local.get $minus) (local.get $count)) (i32.const 1)))
    (block $done
      (loop $lp
        (br_if $done (i32.lt_s (local.get $i) (local.get $minus)))
        (array.set $bytes (local.get $arr) (local.get $i)
          (i32.add (i32.const 48) (i32.rem_u (local.get $mag) (i32.const 10))))
        (local.set $mag (i32.div_u (local.get $mag) (i32.const 10)))
        (local.set $i (i32.sub (local.get $i) (i32.const 1)))
        (br $lp)
      ))
    (local.get $arr)
  )

  ;; "N left", for the active count: the digits, then the suffix.
  (func $count-label (param $n i32) (result (ref $bytes))
    (local $d (ref $bytes))
    (local $dl i32)
    (local $arr (ref $bytes))
    (local $i i32)
    (local.set $d (call $itoa (local.get $n)))
    (local.set $dl (array.len (local.get $d)))
    (local.set $arr (array.new_default $bytes (i32.add (local.get $dl) (i32.const 5))))
    (block $done
      (loop $lp
        (br_if $done (i32.ge_u (local.get $i) (local.get $dl)))
        (array.set $bytes (local.get $arr) (local.get $i)
          (array.get $bytes (local.get $d) (local.get $i)))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $lp)
      ))
    ;; The " left" suffix, from the literal.
    (local.set $i (i32.const 0))
    (block $done
      (loop $lp
        (br_if $done (i32.ge_u (local.get $i) (i32.const 5)))
        (array.set $bytes (local.get $arr)
          (i32.add (local.get $dl) (local.get $i))
          (i32.load8_u (i32.add (i32.const 65550) (local.get $i))))
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

  ;; ================================================================ the lists

  ;; The position of the todo with idx in arr, or -1.
  (func $find (param $arr (ref $todos)) (param $idx i32) (result i32)
    (local $i i32)
    (block $done
      (loop $lp
        (br_if $done (i32.ge_u (local.get $i) (array.len (local.get $arr))))
        (if (i32.eq
              (struct.get $todo $idx (array.get $todos (local.get $arr) (local.get $i)))
              (local.get $idx))
          (then (return (local.get $i))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $lp)
      ))
    (i32.const -1)
  )

  ;; Copy len todos from src at src-off into dst at dst-off. A loop: this
  ;; wasm-tools WAT parser has no array.copy.
  (func $copy-range (param $dst (ref $todos)) (param $dst-off i32)
    (param $src (ref $todos)) (param $src-off i32) (param $len i32)
    (local $i i32)
    (block $done
      (loop $lp
        (br_if $done (i32.ge_u (local.get $i) (local.get $len)))
        (array.set $todos (local.get $dst) (i32.add (local.get $dst-off) (local.get $i))
          (array.get $todos (local.get $src) (i32.add (local.get $src-off) (local.get $i))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $lp)
      ))
  )

  ;; arr with t at the front: a fresh array (the GC heap owns the old one).
  (func $push-front (param $arr (ref $todos)) (param $t (ref $todo)) (result (ref $todos))
    (local $n i32)
    (local $out (ref $todos))
    (local.set $n (array.len (local.get $arr)))
    (local.set $out (array.new_default $todos (i32.add (local.get $n) (i32.const 1))))
    (array.set $todos (local.get $out) (i32.const 0) (local.get $t))
    (if (i32.gt_s (local.get $n) (i32.const 0))
      (then
        (call $copy-range (local.get $out) (i32.const 1)
          (local.get $arr) (i32.const 0) (local.get $n))
      ))
    (local.get $out)
  )

  ;; arr without its element at pos (a fresh array). pos is in range.
  (func $remove-at (param $arr (ref $todos)) (param $pos i32) (result (ref $todos))
    (local $n i32)
    (local $out (ref $todos))
    (local.set $n (array.len (local.get $arr)))
    (local.set $out (array.new_default $todos (i32.sub (local.get $n) (i32.const 1))))
    (if (i32.gt_s (local.get $pos) (i32.const 0))
      (then
        (call $copy-range (local.get $out) (i32.const 0)
          (local.get $arr) (i32.const 0) (local.get $pos))
      ))
    (call $copy-range (local.get $out) (local.get $pos)
      (local.get $arr)
      (i32.add (local.get $pos) (i32.const 1))
      (i32.sub (local.get $n) (i32.add (local.get $pos) (i32.const 1))))
    (local.get $out)
  )

  ;; The todo with idx, in whichever list holds it: toggled to the other
  ;; column's front, or removed. A stale idx (gone or deleted) is a no-op.
  (func $move (param $idx i32) (param $remove i32)
    (local $active (ref $todos))
    (local $done (ref $todos))
    (local $pos i32)
    (local $t (ref $todo))
    (local.set $active (ref.cast (ref $todos) (global.get $active)))
    (local.set $pos (call $find (local.get $active) (local.get $idx)))
    (if (call $nez (i32.add (local.get $pos) (i32.const 1)))
      (then
        (local.set $t (ref.cast (ref $todo) (array.get $todos (local.get $active) (local.get $pos))))
        (global.set $active (call $remove-at (local.get $active) (local.get $pos)))
        (if (i32.eqz (local.get $remove))
          (then
            (struct.set $todo $done (local.get $t) (i32.const 1))
            (global.set $done
              (call $push-front (ref.cast (ref $todos) (global.get $done)) (local.get $t)))
          )
        )
      )
      (else
        (local.set $done (ref.cast (ref $todos) (global.get $done)))
        (local.set $pos (call $find (local.get $done) (local.get $idx)))
        (if (call $nez (i32.add (local.get $pos) (i32.const 1)))
          (then
            (local.set $t (ref.cast (ref $todo) (array.get $todos (local.get $done) (local.get $pos))))
            (global.set $done (call $remove-at (local.get $done) (local.get $pos)))
            (if (i32.eqz (local.get $remove))
              (then
                (struct.set $todo $done (local.get $t) (i32.const 0))
                (global.set $active
                  (call $push-front (ref.cast (ref $todos) (global.get $active)) (local.get $t)))
              )
            )
          )
        )
      )
    )
  )

  ;; Seed the lists once: the first three presets, the last one done, in
  ;; index order (so the newest, 1, sits at the front of the active list).
  (func $seed
    (local $a (ref $todos))
    (local $d (ref $todos))
    (if (global.get $inited)
      (then (return)))
    (global.set $inited (i32.const 1))
    (local.set $a (array.new_default $todos (i32.const 2)))
    (array.set $todos (local.get $a) (i32.const 0)
      (struct.new $todo (i32.const 1) (call $preset (i32.const 1)) (i32.const 0)))
    (array.set $todos (local.get $a) (i32.const 1)
      (struct.new $todo (i32.const 0) (call $preset (i32.const 0)) (i32.const 0)))
    (local.set $d (array.new_default $todos (i32.const 1)))
    (array.set $todos (local.get $d) (i32.const 0)
      (struct.new $todo (i32.const 2) (call $preset (i32.const 2)) (i32.const 1)))
    (global.set $active (local.get $a))
    (global.set $done (local.get $d))
  )

  ;; One todo's four nodes: the row (its id changes with the column) and the
  ;; checkbox, label, and delete (their ids do not), at c in arr.
  (func $emit-row (param $arr (ref $flats)) (param $c i32) (param $index i32)
    (param $t (ref $todo)) (param $col i32) (param $row-base i32)
    (local $base i32)
    (local $row i32)
    (local.set $base (i32.mul (struct.get $todo $idx (local.get $t)) (i32.const 4)))
    (local.set $row (i32.add (local.get $row-base) (local.get $base)))
    (array.set $flats (local.get $arr) (local.get $c)
      (struct.new $flat
        (i32.const 2) (local.get $row) (local.get $col)
        (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 1)
        (i32.const 0)
        (ref.null $bytes) (local.get $index)))
    (array.set $flats (local.get $arr) (i32.add (local.get $c) (i32.const 1))
      (struct.new $flat
        (i32.const 1) (i32.add (i32.const 101) (local.get $base)) (local.get $row)
        (if (result i32) (struct.get $todo $done (local.get $t))
          (then (i32.const 2)) (else (i32.const 0)))
        (i32.const 1) (i32.const 0) (i32.const 1)
        (i32.const 0)
        (if (result (ref $bytes)) (struct.get $todo $done (local.get $t))
          (then (call $label (i32.const 65544) (i32.const 3)))
          (else (call $label (i32.const 65541) (i32.const 3))))
        (i32.const 0)))
    (array.set $flats (local.get $arr) (i32.add (local.get $c) (i32.const 2))
      (struct.new $flat
        (i32.const 0) (i32.add (i32.const 102) (local.get $base)) (local.get $row)
        (if (result i32) (struct.get $todo $done (local.get $t))
          (then (i32.const 4)) (else (i32.const 0)))
        (i32.const 1) (i32.const 0) (i32.const 1)
        (i32.const 0)
        (struct.get $todo $label (local.get $t)) (i32.const 1)))
    (array.set $flats (local.get $arr) (i32.add (local.get $c) (i32.const 3))
      (struct.new $flat
        (i32.const 1) (i32.add (i32.const 103) (local.get $base)) (local.get $row)
        (i32.const 3) (i32.const 1) (i32.const 0) (i32.const 1)
        (i32.const 0)
        (call $label (i32.const 65547) (i32.const 3)) (i32.const 2)))
  )

  ;; ================================================================ the tree

  ;; The todo list's whole export, as a flat, pre-ordered list: the column
  ;; (1), the header (2), the wrapper (3), the active column (4) and its
  ;; rows, the done column (5) and its rows, and the footer (6) with the
  ;; count, the input, the add button, and the clear button.
  (func $build (result (ref $flats))
    (local $arr (ref $flats))
    (local $active (ref $todos))
    (local $done (ref $todos))
    (local $input (ref null $bytes))
    (local $c i32)
    (local $i i32)
    (call $seed)
    (local.set $active (ref.cast (ref $todos) (global.get $active)))
    (local.set $done (ref.cast (ref $todos) (global.get $done)))
    (local.set $input (global.get $input))
    (local.set $arr (array.new_default $flats
      (i32.add (i32.const 10)
        (i32.mul (i32.const 4)
          (i32.add (array.len (local.get $active)) (array.len (local.get $done)))))))
    ;; The static structure, at the front of the list.
    (array.set $flats (local.get $arr) (i32.const 0)
      (struct.new $flat
        (i32.const 2) (i32.const 1) (i32.const 0)
        (i32.const 0) (i32.const 0) (i32.const 1) (i32.const 1)
        (i32.const 0)
        (ref.null $bytes) (i32.const 0)))
    (array.set $flats (local.get $arr) (i32.const 1)
      (struct.new $flat
        (i32.const 0) (i32.const 2) (i32.const 1)
        (i32.const 1) (i32.const 2) (i32.const 0) (i32.const 1)
        (i32.const 0)
        (call $label (i32.const 65536) (i32.const 5)) (i32.const 0)))
    (array.set $flats (local.get $arr) (i32.const 2)
      (struct.new $flat
        (i32.const 2) (i32.const 3) (i32.const 1)
        (i32.const 0) (i32.const 0) (i32.const 1) (i32.const 0)
        (i32.const 0)
        (ref.null $bytes) (i32.const 1)))
    (array.set $flats (local.get $arr) (i32.const 3)
      (struct.new $flat
        (i32.const 2) (i32.const 4) (i32.const 3)
        (i32.const 0) (i32.const 0) (i32.const 1) (i32.const 0)
        (i32.const 0)
        (ref.null $bytes) (i32.const 0)))
    (local.set $c (i32.const 4))
    ;; The active rows, newest first.
    (block $done
      (loop $lp
        (br_if $done (i32.ge_u (local.get $i) (array.len (local.get $active))))
        (call $emit-row (local.get $arr) (local.get $c) (local.get $i)
          (ref.cast (ref $todo) (array.get $todos (local.get $active) (local.get $i)))
          (i32.const 4) (i32.const 100))
        (local.set $c (i32.add (local.get $c) (i32.const 4)))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $lp)
      ))
    (array.set $flats (local.get $arr) (local.get $c)
      (struct.new $flat
        (i32.const 2) (i32.const 5) (i32.const 3)
        (i32.const 0) (i32.const 0) (i32.const 1) (i32.const 0)
        (i32.const 0)
        (ref.null $bytes) (i32.const 1)))
    (local.set $c (i32.add (local.get $c) (i32.const 1)))
    ;; The done rows, newest first.
    (local.set $i (i32.const 0))
    (block $done
      (loop $lp
        (br_if $done (i32.ge_u (local.get $i) (array.len (local.get $done))))
        (call $emit-row (local.get $arr) (local.get $c) (local.get $i)
          (ref.cast (ref $todo) (array.get $todos (local.get $done) (local.get $i)))
          (i32.const 5) (i32.const 500))
        (local.set $c (i32.add (local.get $c) (i32.const 4)))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $lp)
      ))
    ;; The footer and its four children.
    (array.set $flats (local.get $arr) (local.get $c)
      (struct.new $flat
        (i32.const 2) (i32.const 6) (i32.const 1)
        (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 1)
        (i32.const 0)
        (ref.null $bytes) (i32.const 2)))
    (local.set $c (i32.add (local.get $c) (i32.const 1)))
    (array.set $flats (local.get $arr) (local.get $c)
      (struct.new $flat
        (i32.const 0) (i32.const 7) (i32.const 6)
        (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 1)
        (i32.const 0)
        (call $count-label (array.len (local.get $active))) (i32.const 0)))
    (local.set $c (i32.add (local.get $c) (i32.const 1)))
    (array.set $flats (local.get $arr) (local.get $c)
      (struct.new $flat
        (i32.const 3) (i32.const 8) (i32.const 6)
        (i32.const 0) (i32.const 1) (i32.const 0) (i32.const 1)
        (i32.const 12)
        (local.get $input) (i32.const 1)))
    (local.set $c (i32.add (local.get $c) (i32.const 1)))
    (array.set $flats (local.get $arr) (local.get $c)
      (struct.new $flat
        (i32.const 1) (i32.const 9) (i32.const 6)
        (i32.const 2) (i32.const 1) (i32.const 0) (i32.const 1)
        (i32.const 0)
        (call $label (i32.const 65555) (i32.const 3)) (i32.const 2)))
    (local.set $c (i32.add (local.get $c) (i32.const 1)))
    (array.set $flats (local.get $arr) (local.get $c)
      (struct.new $flat
        (i32.const 1) (i32.const 10) (i32.const 6)
        (i32.const 3) (i32.const 1) (i32.const 0) (i32.const 1)
        (i32.const 0)
        (call $label (i32.const 65558) (i32.const 10)) (i32.const 3)))
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
    ;; The removed ids, in the last export's pre-order. The changed loop
    ;; left $i at the new tree's length; the last tree can be a different
    ;; size, so start over.
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

  ;; The add button: the input's text if the user typed any, else the next
  ;; preset; newest first.
  (func $add
    (local $input (ref null $bytes))
    (local $label (ref $bytes))
    (local $t (ref $todo))
    (local $len i32)
    (local $empty i32)
    (local.set $input (global.get $input))
    ;; The input's length, with null as 0: a cast of a null input would
    ;; trap, so the length comes from an if-as-expression.
    (local.set $len
      (if (result i32) (ref.is_null (local.get $input))
        (then (i32.const 0))
        (else (array.len (ref.cast (ref $bytes) (local.get $input))))))
    (local.set $empty (i32.eqz (local.get $len)))
    ;; The label, from an if-as-expression: the validator rejects a
    ;; non-nullable ref local set in both branches of a statement if.
    (local.set $label
      (if (result (ref $bytes)) (local.get $empty)
        (then (call $preset (i32.rem_u (global.get $preset-idx) (i32.const 6))))
        (else (ref.cast (ref $bytes) (local.get $input)))))
    (if (local.get $empty)
      (then
        (global.set $preset-idx (i32.add (global.get $preset-idx) (i32.const 1)))
      )
      (else
        (global.set $input (ref.null $bytes))
      ))
    (local.set $t (struct.new $todo
      (global.get $next-idx) (local.get $label) (i32.const 0)))
    (global.set $next-idx (i32.add (global.get $next-idx) (i32.const 1)))
    (global.set $active
      (call $push-front (ref.cast (ref $todos) (global.get $active)) (local.get $t)))
  )

  (func (export "on-click") (param $id i32)
    (call $seed)
    (if (i32.lt_u (local.get $id) (i32.const 101))
      (then
        (if (i32.eq (local.get $id) (i32.const 9))
          (then (call $add)))
        (if (i32.eq (local.get $id) (i32.const 10))
          (then (global.set $done (array.new_default $todos (i32.const 0)))))
      )
      (else
        ;; A row's checkbox (101+4i) or delete (103+4i); the todo's idx is
        ;; the id's high bits. Other ids in range are no-ops, like in Rust.
        (if (i32.eq (i32.rem_u (local.get $id) (i32.const 4)) (i32.const 1))
          (then
            (call $move
              (i32.div_u (i32.sub (local.get $id) (i32.const 101)) (i32.const 4))
              (i32.const 0))))
        (if (i32.eq (i32.rem_u (local.get $id) (i32.const 4)) (i32.const 3))
          (then
            (call $move
              (i32.div_u (i32.sub (local.get $id) (i32.const 103)) (i32.const 4))
              (i32.const 1))))
      ))
  )

  ;; The input (8): the component stores the buffer; the input node's content
  ;; re-derives from it. The buffer is the host's to give and the callee's to
  ;; free.
  (func (export "on-text") (param $id i32) (param $ptr i32) (param $len i32)
    (local $a (ref $bytes))
    (local $i i32)
    (if (i32.eq (local.get $id) (i32.const 8))
      (then
        (local.set $a (array.new_default $bytes (local.get $len)))
        (block $done
          (loop $lp
            (br_if $done (i32.ge_u (local.get $i) (local.get $len)))
            (array.set $bytes (local.get $a) (local.get $i)
              (i32.load8_u (i32.add (local.get $ptr) (local.get $i))))
            (local.set $i (i32.add (local.get $i) (i32.const 1)))
            (br $lp)
          ))
        (global.set $input (local.get $a))
      ))
    (drop (call $cabi-realloc (local.get $ptr) (local.get $len) (i32.const 0) (i32.const 1)))
  )

  (func (export "on-tick"))
)
