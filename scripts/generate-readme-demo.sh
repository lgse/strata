#!/usr/bin/env bash
set -euo pipefail

# Rebuild the README's high-resolution demo video from the three browser-mode
# captures, then derive the optimized GitHub-friendly GIF from that video.
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
assets="$root/docs/assets"

command -v ffmpeg >/dev/null 2>&1 || {
  echo "ffmpeg is required to generate the README demo" >&2
  exit 1
}

ffmpeg -hide_banner -loglevel error -y \
  -loop 1 -t 8.4 -i "$assets/strata-screenshot.png" \
  -loop 1 -t 8.4 -i "$assets/strata-grid.png" \
  -loop 1 -t 8.4 -i "$assets/strata-explorer.png" \
  -loop 1 -t 8.4 -i "$assets/strata-screenshot.png" \
  -filter_complex \
  "[0:v]scale=1920:1056,format=yuv420p[a];\
[1:v]scale=1920:1056,format=yuv420p[b];\
[2:v]scale=1920:1056,format=yuv420p[c];\
[3:v]scale=1920:1056,format=yuv420p[d];\
[a][b]xfade=transition=fade:duration=0.6:offset=2.2[ab];\
[ab][c]xfade=transition=fade:duration=0.6:offset=4.8[abc];\
[abc][d]xfade=transition=fade:duration=0.6:offset=7.4,fps=30[out]" \
  -map '[out]' -t 8 -an -c:v libx264 -crf 20 -preset slow \
  -movflags +faststart "$assets/strata-demo.mp4"

ffmpeg -hide_banner -loglevel error -y -i "$assets/strata-demo.mp4" \
  -filter_complex \
  "fps=10,scale=1280:704:flags=lanczos,split[x][p];\
[p]palettegen=max_colors=128:stats_mode=diff[pal];\
[x][pal]paletteuse=dither=bayer:bayer_scale=3:diff_mode=rectangle" \
  -loop 0 "$assets/strata-demo.gif"

printf 'Generated %s and %s\n' \
  "$assets/strata-demo.mp4" "$assets/strata-demo.gif"
