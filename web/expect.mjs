// The demos' expected node strings, shared by the two browser-side and
// Node-side suites that assert on them:
//
//   web/selftest.mjs  the Node ABI selftest — serializes the ABI node list
//                     (its `node()`), against these strings.
//   e2e/              the Playwright suite — serializes the rendered DOM
//                     (e2e/helpers.mjs), against the same strings.
//
// The DOM carries the same facts as the ABI node: the widget id in
// data-id, the kind and the style tokens as waui-* classes, the content as
// text or value, and the input's width in data-width. Both serializers
// produce the selftest's string format, so the expectations live here once:
// the component's output is the source of truth, and the two suites compare
// against the same copy of it.

// The counter (crates/demo-counter, assembly/demo-counter).
export const counter = {
  // The initial view: the readout at zero.
  zero: [
    'container{id:1,parent:null,dir:column,align:center}',
    'button{id:2,parent:1,label:"+",style:positive:medium}',
    'text{id:3,parent:1,content:"0",style:neutral:large}',
    'button{id:4,parent:1,label:"-",style:negative:medium}',
  ],
  // After two decrements: the readout's content and color both change.
  minusOne: [
    'container{id:1,parent:null,dir:column,align:center}',
    'button{id:2,parent:1,label:"+",style:positive:medium}',
    'text{id:3,parent:1,content:"-1",style:negative:large}',
    'button{id:4,parent:1,label:"-",style:negative:medium}',
  ],
};

// The stopwatch (crates/demo-stopwatch, assembly/demo-stopwatch).
export const stopwatch = {
  // The initial view: stopped at zero.
  idle: [
    'container{id:1,parent:null,dir:column,align:center}',
    'text{id:2,parent:1,content:"0 ms",style:neutral:large}',
    'button{id:3,parent:1,label:"start",style:positive:medium}',
  ],
};

// The todo list (crates/demo-todo, assembly/demo-todo). The states follow
// the selftest's sequence: seed, type, add, toggle, remove, clear done.

const todoSeeded = [
  'container{id:1,parent:null,dir:column,align:center}',
  'text{id:2,parent:1,content:"todos",style:accent:large}',
  'container{id:3,parent:1,dir:column,align:start}',
  'container{id:4,parent:3,dir:column,align:start}',
  'container{id:104,parent:4,dir:row,align:center}',
  'button{id:105,parent:104,label:"○",style:neutral:medium}',
  'text{id:106,parent:104,content:"walk the dog",style:neutral:medium}',
  'button{id:107,parent:104,label:"✕",style:negative:medium}',
  'container{id:100,parent:4,dir:row,align:center}',
  'button{id:101,parent:100,label:"○",style:neutral:medium}',
  'text{id:102,parent:100,content:"buy milk",style:neutral:medium}',
  'button{id:103,parent:100,label:"✕",style:negative:medium}',
  'container{id:5,parent:3,dir:column,align:start}',
  'container{id:508,parent:5,dir:row,align:center}',
  'button{id:109,parent:508,label:"✓",style:positive:medium}',
  'text{id:110,parent:508,content:"water plants",style:warning:medium}',
  'button{id:111,parent:508,label:"✕",style:negative:medium}',
  'container{id:6,parent:1,dir:row,align:center}',
  'text{id:7,parent:6,content:"2 left",style:neutral:small}',
  'input{id:8,parent:6,content:"",style:neutral:medium,width:12}',
  'button{id:9,parent:6,label:"add",style:positive:medium}',
  'button{id:10,parent:6,label:"clear done",style:negative:medium}',
];

export const todo = {
  // Seeded: two active (newest first), one done, "2 left".
  seeded: todoSeeded,
  // The typed buffer, echoed into the field.
  typed: todoSeeded.map((s) =>
    s.startsWith('input{id:8,') ? s.replace('content:""', 'content:"buy coffee"') : s,
  ),
  // "add" with the typed text: the new row at the top of the active column,
  // the count, the cleared field.
  added: [
    'container{id:1,parent:null,dir:column,align:center}',
    'text{id:2,parent:1,content:"todos",style:accent:large}',
    'container{id:3,parent:1,dir:column,align:start}',
    'container{id:4,parent:3,dir:column,align:start}',
    'container{id:112,parent:4,dir:row,align:center}',
    'button{id:113,parent:112,label:"○",style:neutral:medium}',
    'text{id:114,parent:112,content:"buy coffee",style:neutral:medium}',
    'button{id:115,parent:112,label:"✕",style:negative:medium}',
    'container{id:104,parent:4,dir:row,align:center}',
    'button{id:105,parent:104,label:"○",style:neutral:medium}',
    'text{id:106,parent:104,content:"walk the dog",style:neutral:medium}',
    'button{id:107,parent:104,label:"✕",style:negative:medium}',
    'container{id:100,parent:4,dir:row,align:center}',
    'button{id:101,parent:100,label:"○",style:neutral:medium}',
    'text{id:102,parent:100,content:"buy milk",style:neutral:medium}',
    'button{id:103,parent:100,label:"✕",style:negative:medium}',
    'container{id:5,parent:3,dir:column,align:start}',
    'container{id:508,parent:5,dir:row,align:center}',
    'button{id:109,parent:508,label:"✓",style:positive:medium}',
    'text{id:110,parent:508,content:"water plants",style:warning:medium}',
    'button{id:111,parent:508,label:"✕",style:negative:medium}',
    'container{id:6,parent:1,dir:row,align:center}',
    'text{id:7,parent:6,content:"3 left",style:neutral:small}',
    'input{id:8,parent:6,content:"",style:neutral:medium,width:12}',
    'button{id:9,parent:6,label:"add",style:positive:medium}',
    'button{id:10,parent:6,label:"clear done",style:negative:medium}',
  ],
  // "walk the dog" toggled: re-added under a new id (504) in the done
  // column, its children re-parented, the count, the old row removed.
  afterToggle: [
    'container{id:1,parent:null,dir:column,align:center}',
    'text{id:2,parent:1,content:"todos",style:accent:large}',
    'container{id:3,parent:1,dir:column,align:start}',
    'container{id:4,parent:3,dir:column,align:start}',
    'container{id:112,parent:4,dir:row,align:center}',
    'button{id:113,parent:112,label:"○",style:neutral:medium}',
    'text{id:114,parent:112,content:"buy coffee",style:neutral:medium}',
    'button{id:115,parent:112,label:"✕",style:negative:medium}',
    'container{id:100,parent:4,dir:row,align:center}',
    'button{id:101,parent:100,label:"○",style:neutral:medium}',
    'text{id:102,parent:100,content:"buy milk",style:neutral:medium}',
    'button{id:103,parent:100,label:"✕",style:negative:medium}',
    'container{id:5,parent:3,dir:column,align:start}',
    'container{id:504,parent:5,dir:row,align:center}',
    'button{id:105,parent:504,label:"✓",style:positive:medium}',
    'text{id:106,parent:504,content:"walk the dog",style:warning:medium}',
    'button{id:107,parent:504,label:"✕",style:negative:medium}',
    'container{id:508,parent:5,dir:row,align:center}',
    'button{id:109,parent:508,label:"✓",style:positive:medium}',
    'text{id:110,parent:508,content:"water plants",style:warning:medium}',
    'button{id:111,parent:508,label:"✕",style:negative:medium}',
    'container{id:6,parent:1,dir:row,align:center}',
    'text{id:7,parent:6,content:"2 left",style:neutral:small}',
    'input{id:8,parent:6,content:"",style:neutral:medium,width:12}',
    'button{id:9,parent:6,label:"add",style:positive:medium}',
    'button{id:10,parent:6,label:"clear done",style:negative:medium}',
  ],
  // "buy milk" removed: the count, then the row and its children.
  afterRemove: [
    'container{id:1,parent:null,dir:column,align:center}',
    'text{id:2,parent:1,content:"todos",style:accent:large}',
    'container{id:3,parent:1,dir:column,align:start}',
    'container{id:4,parent:3,dir:column,align:start}',
    'container{id:112,parent:4,dir:row,align:center}',
    'button{id:113,parent:112,label:"○",style:neutral:medium}',
    'text{id:114,parent:112,content:"buy coffee",style:neutral:medium}',
    'button{id:115,parent:112,label:"✕",style:negative:medium}',
    'container{id:5,parent:3,dir:column,align:start}',
    'container{id:504,parent:5,dir:row,align:center}',
    'button{id:105,parent:504,label:"✓",style:positive:medium}',
    'text{id:106,parent:504,content:"walk the dog",style:warning:medium}',
    'button{id:107,parent:504,label:"✕",style:negative:medium}',
    'container{id:508,parent:5,dir:row,align:center}',
    'button{id:109,parent:508,label:"✓",style:positive:medium}',
    'text{id:110,parent:508,content:"water plants",style:warning:medium}',
    'button{id:111,parent:508,label:"✕",style:negative:medium}',
    'container{id:6,parent:1,dir:row,align:center}',
    'text{id:7,parent:6,content:"1 left",style:neutral:small}',
    'input{id:8,parent:6,content:"",style:neutral:medium,width:12}',
    'button{id:9,parent:6,label:"add",style:positive:medium}',
    'button{id:10,parent:6,label:"clear done",style:negative:medium}',
  ],
  // "clear done": only the done rows — the count (the active list) is
  // unchanged, so it is not reported.
  afterClear: [
    'container{id:1,parent:null,dir:column,align:center}',
    'text{id:2,parent:1,content:"todos",style:accent:large}',
    'container{id:3,parent:1,dir:column,align:start}',
    'container{id:4,parent:3,dir:column,align:start}',
    'container{id:112,parent:4,dir:row,align:center}',
    'button{id:113,parent:112,label:"○",style:neutral:medium}',
    'text{id:114,parent:112,content:"buy coffee",style:neutral:medium}',
    'button{id:115,parent:112,label:"✕",style:negative:medium}',
    'container{id:5,parent:3,dir:column,align:start}',
    'container{id:6,parent:1,dir:row,align:center}',
    'text{id:7,parent:6,content:"1 left",style:neutral:small}',
    'input{id:8,parent:6,content:"",style:neutral:medium,width:12}',
    'button{id:9,parent:6,label:"add",style:positive:medium}',
    'button{id:10,parent:6,label:"clear done",style:negative:medium}',
  ],
};
