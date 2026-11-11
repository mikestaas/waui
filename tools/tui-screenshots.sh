#!/usr/bin/env bash
# Render the host-tui goldens (ANSI text) to PNGs for the README, via freeze.
#
# A tui golden is a fixed-width grid of SGR-colored cells; GitHub READMEs
# can't render it inline, so each becomes an image. freeze renders the ANSI
# with a real monospace font. The terminal is sized to the frame (its widest
# line and its line count) so the hint bar and the content don't wrap.
#
# Usage: tools/tui-screenshots.sh   (writes docs/screenshots/*-tui*.png)

set -euo pipefail
cd "$(dirname "$0")/.." # the repo root

GOLD=crates/host-tui/tests/goldens
OUT=docs/screenshots
mkdir -p "$OUT"

command -v freeze >/dev/null 2>&1 || {
  echo "tui-screenshots needs freeze — brew install charmbracelet/tap/freeze" >&2
  exit 1
}

# <golden-stem> <output-stem> pairs: the three demos, each in its three source
# languages. The tui goldens are shared, so the languages are byte-identical;
# each still renders to its own file.
pairs=(
  "counter-1 counter-tui" "counter-1-as counter-tui-as" "counter-1-wat counter-tui-wat"
  "stopwatch-running stopwatch-tui" "stopwatch-running-as stopwatch-tui-as" "stopwatch-running-wat stopwatch-tui-wat"
  "todo-added todo-tui" "todo-added-as todo-tui-as" "todo-added-wat todo-tui-wat"
)

for pair in "${pairs[@]}"; do
  stem=${pair%% *}
  out=${pair##* }
  file="$GOLD/$stem.txt"
  printf "%s" "$(cat $file)" | freeze  --font.size 7 -p 0 -o "$OUT/$out.png"
done

echo "wrote $OUT/*-tui*.png"
