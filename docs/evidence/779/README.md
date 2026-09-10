# Real-playback memory comparison: #764 / #765 / #779

## Conclusion

[#765](https://github.com/lgse/strata/pull/765) addresses the dominant roughly
half-GiB-per-open VRAM retention in this reproduction. All fixed-build media
hierarchies finalize and large playback allocations drop on close.

Residual RAM/file-descriptor and small VRAM growth remains; it is tracked in
[#779](https://github.com/lgse/strata/issues/779). These nine fixed-build media
cycles do not prove whether the remainder is an unbounded leak or eventually
reaches a plateau. Identify surviving resources before selecting another fix.

## Reproduction and provenance

The owner manually opened/played/closed previews on an NVIDIA GPU, using native
debug binaries and an external graphics-inclusive `nvidia-smi -q -x` + `/proc`
sampler. The application used a private D-Bus session and private settings/cache
profile, on the desktop display operated by the owner. Automated checks were
separately isolated on Xvfb; they are not the source of these GPU measurements.

| Property | Both captures |
| --- | --- |
| Application | Strata 0.14.0, native debug |
| Host | Omarchy/Arch Linux, Hyprland |
| GTK | 4.22.4 |
| GPU / driver | NVIDIA GeForce RTX 5090 / 610.57.04 |
| Actual media backend | `GtkGstMediaFileBuiltin` |
| Renderer | `GskVulkanRenderer` |
| Conversion policy | **Software**; media helpers received **zero GPU devices** |
| Encoded GIF / MP4 output | 490,175 / 4,634,021 bytes |
| Prepared dimensions | GIF 1280×542; MP4 1280×1280 |

Baseline source: upstream **`63f1063589b4a523612ef606a847be05fa6ffce3`** plus opt-in
instrumentation. Comparison source: that **same baseline plus #765 at
`51c8a16b5154887caec3b4e29206c653df123cd1`**, retaining the traces. This is not a
pristine executable of the PR's older tree. The diagnostic PR containing this
report is stacked on #765; the measurements predate its resource-category/PSS
extension and do not claim those new fields were already collected.

- Baseline SHA256:
  `2d0fa0888fd5ee714e2059fd17d035d7925ba91610388688bab47f43bf727c4e`
- Comparison SHA256:
  `112c6f940d3b6372903efa12798210f7896c8439c2fce2364678d99db8ad0585`

The baseline contained **5 GIF + 5 MP4 + 25 image opens**. The fixed run contained
**4 GIF + 5 MP4 + 29 image opens**, including images after the video. Timing and
order differ: final totals are descriptive, not an identical-work benchmark.
The five-MP4 phase is the clearest equal-count comparison. File tokens are random
per process and cannot establish cross-run identity; the owner repeated the media,
and encoded output sizes and prepared dimensions match.

## Main-process results

| Measurement | Unfixed | With #765 |
| --- | ---: | ---: |
| Pre-preview VRAM | 60 MiB | 60 MiB |
| Added retained VRAM across five MP4 opens | +2,832 MiB | **+26 MiB** |
| Final closed-preview VRAM | 5,577 MiB | **114 MiB** |
| Sampled playback peak | 5,577 MiB | 680 MiB |
| Media finalized / created | 0 / 10 | **9 / 9** |
| Pictures finalized / created | 0 / 10 | **9 / 9** |
| Overlays finalized / created | 0 / 10 | **9 / 9** |
| Pre-preview → final RSS | 252 → 1,560 MiB | 184 → 682 MiB |

For the five-MP4 phase, unfixed VRAM goes 2,745 → 5,577 MiB; fixed VRAM goes
88 → 114 MiB: approximately **99.1% less retained increase in that phase**, not a
universal memory-reduction claim. Fixed hierarchies finalize 58–65 ms after cleanup
begins. VRAM stays at 114 MiB for approximately 20 seconds after the final video
close, including subsequent image previews. The final reported point precedes
process teardown, about seven seconds after the final image-preview close.

Each run launches only two media conversions (initial GIF and MP4); repeated
media openings are cache hits. The encoded cache ends around 6 MiB. Image-only
phases are flat at 2,745 MiB before the fix, and at 88/114 MiB in the fixed run.
The acceleration preference controls conversion, not all GTK playback/display GPU
usage. Neither repeated conversion nor cache capacity explains the large staircase.

## Remaining resource growth

| Fixed GIF close | RSS MiB | FDs | VRAM MiB |
| --- | ---: | ---: | ---: |
| 1 | 338.4 | 111 | 75 |
| 2 | 374.9 | 120 | 79 |
| 3 | 417.1 | 129 | 84 |
| 4 | 451.4 | 138 | 88 |

MP4 post-close VRAM observations are 97, **unavailable between actions**, 106, 110,
114 MiB. The second MP4's close/reopen gap contains no fully isolated sample; it is
not filled with the next player's startup measurement.

After video cycling, RSS reaches about 675 MiB, then 682 MiB at the final closed
sample. FDs end at 180 versus 71 before previews. Several successive media cycles
leave roughly nine FDs. Thread counts also stay elevated, but shared worker pools
complicate attribution. The fixed encoded cache ends at 6,385,054 bytes (~6.1 MiB).

Counts and RSS are not allocation backtraces. The next experiment should extend
same-file cycles to 30–50 and inspect path-free FD/thread categories and PSS at
closed checkpoints. If GTK objects finalize but resource counts continue to grow,
follow the surviving backend/driver owners. Split CPU/GPU fix issues only if
measurements establish independent causes; do not infer one from timing alone.

## Included evidence and limitations

- [`baseline-memory.csv`](baseline-memory.csv): all 202 main-PID samples.
- [`pr765-memory.csv`](pr765-memory.csv): all 163 main-PID samples.
- [`baseline-events.jsonl`](baseline-events.jsonl): 522 structured trace records.
- [`pr765-events.jsonl`](pr765-events.jsonl): 568 structured trace records.
- [`baseline-provenance.json`](baseline-provenance.json) and
  [`pr765-provenance.json`](pr765-provenance.json): sanitized environment/build data.

Only structured traces and numeric main-PID samples are included. String values
were reviewed: event/backend/type names and opaque tokens, not filenames. Raw
application logs, user profiles/settings, stack dumps, and core dumps are excluded.
GPU totals in provenance include the desktop; results above use only the main PID.

Both captures had zero trace-parse/GPU-query errors. Memory samples begin roughly
500 ms apart; query windows last approximately 90–300 ms. Align event `unix_ms`
with CSV `unix_ms` / `sample_end_unix_ms`; exclude samples overlapping the next open
for post-close comparisons. Missing NVIDIA records/values are not zero. Final
process-teardown samples with unavailable memory are not post-close evidence.

CSV schemas retain the original extraction: `main_pid`/`main_present` in baseline,
`pid`/`main_present` in the fixed run; RSS is in MiB, FDs/threads are counts. Each
JSONL record includes an event, timestamp, PID, optional sandbox job, and structured
fields. Request IDs are drawer-local; both sessions use one drawer. Helper PIDs
with `sandbox_job` set are namespace-local, not the main host PID.

See [capture instructions](../../preview-memory-tracing.md) for the next run.
These historical captures validate the large improvement in #765, not a fix for
#779, and are not a substitute for portable regression tests or required CI.
