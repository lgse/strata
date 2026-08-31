#!/usr/bin/env bash
set -euo pipefail

# Build the README's GitHub-friendly animation from the current high-resolution
# product screenshots. The first frame doubles as the reduced-motion fallback.
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
assets="$root/docs/assets"

command -v ffmpeg >/dev/null 2>&1 || {
  echo "ffmpeg is required to generate the README demo" >&2
  exit 1
}

ffmpeg -hide_banner -loglevel error -y \
  -loop 1 -t 10.0 -i "$assets/strata-columns.png" \
  -loop 1 -t 10.0 -i "$assets/strata-search.png" \
  -loop 1 -t 10.0 -i "$assets/strata-settings.png" \
  -loop 1 -t 10.0 -i "$assets/strata-themes.png" \
  -loop 1 -t 10.0 -i "$assets/strata-columns.png" \
  -filter_complex \
  "[0:v]scale=1280:696,format=rgba[a];\
[1:v]scale=1280:696,format=rgba[b];\
[2:v]scale=1280:696,format=rgba[c];\
[3:v]scale=1280:696,format=rgba[d];\
[4:v]scale=1280:696,format=rgba[e];\
[a][b]xfade=transition=fade:duration=0.5:offset=2.0[ab];\
[ab][c]xfade=transition=fade:duration=0.5:offset=4.4[abc];\
[abc][d]xfade=transition=fade:duration=0.5:offset=6.8[abcd];\
[abcd][e]xfade=transition=fade:duration=0.5:offset=9.2,fps=10,split[x][p];\
[p]palettegen=max_colors=128:stats_mode=diff[pal];\
[x][pal]paletteuse=dither=bayer:bayer_scale=3:diff_mode=rectangle" \
  -t 9.7 -loop 0 "$assets/strata-demo.gif"

printf 'Generated %s\n' "$assets/strata-demo.gif"
