#!/bin/bash
# The README banner and demo GIFs are rendered from banner.html and demo.html with agent-browser (headless Chromium).
#   banner: agent-browser open file://$PWD/banner.html[?theme=dark]; set viewport 2560 800 1; screenshot ../banner-light.png
#   demo:   ./frames.sh light; ./frames.sh dark; then
#     ffmpeg -framerate 12 -i frames-light/%04d.png -vf "split[a][b];[a]palettegen=max_colors=128:stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=5:diff_mode=rectangle" -loop 0 ../demo-light.gif
# demo.html is the landing page's pad with its draw(t) timeline exposed, so every frame is a function of t.
# frames.sh <light|dark>: one PNG per 1/12 s of the demo timeline, from draw(t)
set -e
R=$(dirname "$0"); T=$1; S=demo-$T; D=$R/frames-$T; rm -rf "$D"; mkdir -p "$D"
agent-browser --session $S open "file://$R/demo.html?theme=$T" >/dev/null
agent-browser --session $S set viewport 800 540 2 >/dev/null
agent-browser --session $S eval "document.fonts.ready.then(() => 1)" >/dev/null
total=$(agent-browser --session $S eval "window.total" | tr -dc 0-9)
step=83; n=$((total/step))
for ((i=0;i<=n;i++)); do
  agent-browser --session $S eval "draw($((i*step)))" >/dev/null
  agent-browser --session $S screenshot "$D/$(printf %04d $i).png" >/dev/null
done
agent-browser --session $S close >/dev/null
echo "$T: $((n+1)) frames of $total ms"
