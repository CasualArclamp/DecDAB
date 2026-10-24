#!/usr/bin/env bash
# made by CasualArclamp
# Smoke test of the DecDAB executables, for CI and the release workflow (Linux, or Git
# Bash on Windows). After DecDRM's.
#
#   scripts/smoke-test.sh BIN_DIR OUT_DIR
#
# BIN_DIR holds decdab and decdab-gui. The CLI prints its version and the Band III plan;
# the GUI starts with no source and saves a screenshot: headless under xvfb-run on Linux;
# on Windows it needs an OpenGL driver (Mesa on GitHub's runners). OUT_DIR receives the
# screenshot.
#
# Nothing here decodes a signal yet: the IQ captures are never committed, and the
# modulator (M2) that will make a test signal does not exist yet. When it does, add:
# modulate an ensemble, `decdab decode` a service, and check its audio frames.
set -euo pipefail

if [ $# -ne 2 ]; then
    echo "usage: $0 BIN_DIR OUT_DIR" >&2
    exit 2
fi
bin=$(cd "$1" && pwd)
mkdir -p "$2"
out=$(cd "$2" && pwd)
work=$(mktemp -d)
cd "$work"

"$bin/decdab" --version
"$bin/decdab-gui" --version

echo "== the Band III plan"
"$bin/decdab" blocks | tee "$out/blocks.txt"
grep -E "^ +12B +225\.648 MHz" "$out/blocks.txt"

echo "== GUI: start with no source, screenshot"
gui=("$bin/decdab-gui")
if [ "$(uname -s)" = Linux ]; then
    gui=(xvfb-run -a -s "-screen 0 1400x860x24" "$bin/decdab-gui")
fi
# A screenshot run keeps nothing in the prefs and plays no sound.
"${gui[@]}" --screenshot "$out/gui.png" --after 3
test -s "$out/gui.png"
echo "smoke test passed"
