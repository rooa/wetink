#!/usr/bin/env bash
# Record the README demo, demo/wetink.gif: asciinema records a terminal that
# tmux drives, and agg renders the GIF.
#   cargo build --release && demo/record.sh
set -euo pipefail
cd "$(dirname "$0")/.."
for t in tmux asciinema agg python3; do
  command -v "$t" > /dev/null || { echo "record.sh: needs $t" >&2; exit 1; }
done

root=$PWD
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cast=$work/demo.cast
theme=dracula
bg="#282a36" # the theme's background; wetink detects it and fades flashes into it
tm() { tmux -L wetink-demo "$@"; } # a private tmux server
keys() { tm send-keys -t demo "$@"; }
type_slowly() {
  local s=$1 i
  for ((i = 0; i < ${#s}; i++)); do keys -l "${s:i:1}"; sleep 0.07; done
}

tm kill-server 2> /dev/null || true
tm -f /dev/null new-session -d -s demo -x 100 -y 30 -c "$work" \
  "PATH='$root/target/release':\$PATH asciinema rec -q --overwrite -c 'env PS1=\"\$ \" BASH_SILENCE_DEPRECATION_WARNING=1 bash --norc --noprofile' '$cast'"
tm set -g status off
tm set -g window-style "bg=$bg"
sleep 1

# Open the viewer first; the simulated agent then creates and edits demo.md.
type_slowly "wetink demo.md"
sleep 0.4
keys Enter
sleep 1.5
python3 scripts/simulate.py "$work/demo.md" -d 1 > /dev/null
sleep 2

# Select changes with n and open their diffs with d: the status line, the
# first checked task, and (skipping the second) the removed task.
keys g
sleep 0.8
for steps in n n "n n"; do
  for k in $steps; do keys "$k"; sleep 0.8; done
  keys d
  sleep 2
done
sleep 1
keys q
sleep 0.5
keys -l exit
keys Enter
sleep 1
tm kill-server 2> /dev/null || true

# End on the viewer rather than the shell it returns to: drop everything from
# the moment wetink leaves the alternate screen.
python3 - "$cast" << 'PY'
import json, sys
lines = open(sys.argv[1]).read().splitlines()
out = lines[:1]
for line in lines[1:]:
    event = json.loads(line)
    if event[1] == "o" and "\x1b[?1049l" in event[2]:
        break
    out.append(line)
open(sys.argv[1], "w").write("\n".join(out) + "\n")
PY

agg -q --theme "$theme" --font-size 16 --line-height 1.2 --idle-time-limit 2 --last-frame-duration 3 "$cast" demo/wetink.gif
echo "wrote demo/wetink.gif ($(du -h demo/wetink.gif | cut -f1))"
