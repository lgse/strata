# Large directory open (99,998 images)

Measured on 2026-11-21 (baseline) and 2026-11-22 (post-optimization) on the Rune interface.

## Environment and fixture

- **Hardware / OS:** Intel Core Ultra 9 285K (24 cores), Linux `7.2.15-1-omarchy`.
- **Fixture:** One local directory containing **99,998** image files (`.jpg`, `.JPG`, `.jpeg`,
  `.png`, `.webp`) and **0** folders, warm filesystem page cache, empty thumbnail cache.
- **Settings:** `sort_key = "name"`, `sort_direction = "ascending"`, `folders_first = true`,
  `show_hidden = false`, `columns_show_sizes = true`, `columns_mirror_selection = true`,
  `show_thumbnails = true`, `icons_thumbnail_size = 62`, `browser_density = "compact"`,
  `tenxer_mode = true`, `thumbnail_workers = 4`, `restore_tabs = false`.

## 1. Windowed launch timings (`--release`, LTO)

Measured in a private headless `sway` (`1200x760`, scale 1, `WLR_RENDERER=gles2`) and private
session D-Bus with the release `strata` binary (`lto = true`, `codegen-units = 1`).
Values are median `[min, max]` over 3 runs per mode.

- **Mapped:** process launch until the compositor registers the window.
- **Settled:** process launch until process-tree CPU ticks (`utime + stime + cutime + cstime`
  across the process and its descendants) remain unchanged for a continuous 500 ms.
- **Peak PSS:** peak process-tree `Pss` from `/proc/<pid>/smaps_rollup`.
- **Total CPU:** total process-tree user + system CPU seconds at settle time.

| View mode | Mapped (ms) | Settled (ms) | Peak PSS (MiB) | Total CPU (s) |
| --- | ---: | ---: | ---: | ---: |
| Columns | 616 `[561, 631]` | 1,633 `[1,629, 1,785]` | 315.3 `[314.5, 316.9]` | 1.37 `[1.35, 1.49]` |
| List (ungrouped) | 566 `[548, 572]` | 1,665 `[1,573, 1,669]` | 315.2 `[314.7, 317.0]` | 1.33 `[1.32, 1.45]` |
| List (grouped by type) | 566 `[543, 566]` | 1,922 `[1,867, 1,962]` | 323.7 `[323.3, 325.7]` | 1.82 `[1.79, 1.85]` |
| Icons (62 pt tiles) | 566 `[547, 580]` | 1,887 `[1,835, 2,464]` | 319.6 `[319.0, 319.7]` | 2.10 `[1.99, 2.12]` |

## 2. Offscreen phase breakdown (`--profile perf`)

Measured through `strata_app::host::offscreen` (`1200x760`, scale 1, `opt-level = 3`) from
`App::open` (or `Browser::show_child` for the child column case) until `LoadFinished` is
rendered. This covers the Rust, GLib, layout and text-shaping pipeline; it excludes GPU
submission, Wayland swapchain pacing and sandboxed thumbnail worker rendering.

| Mode | Total to `LoadFinished` | To `EntriesReplaced` (enum + sort) | To first 128 visible rows | GLib dispatch | `shell::poll` | `shell::view` | Rune commit + layout + keys | Max turn (frame max) | Post-load 60-frame scroll (p50 / p95 / max) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| Columns (root) | 340.3 ms | 158.2 ms | 169.9 ms | 93.0 ms | 27.9 ms | 4.0 ms | 82.0 ms | 31.1 ms (17.1 ms) | 0.20 / 0.24 / 0.35 ms (`0` views) |
| Columns (child of parent) | 256.7 ms | 113.5 ms | — | 65.0 ms | 36.8 ms | 3.7 ms | 150.7 ms | 21.6 ms (13.2 ms) | 0.41 / 1.34 / 1.59 ms (`4` views) |
| Icons (62 pt tiles) | 266.7 ms | 133.8 ms | 141.2 ms | 88.0 ms | 55.0 ms | 2.7 ms | 118.9 ms | 26.5 ms (14.2 ms) | 0.53 / 2.13 / 2.44 ms (`0` views) |
| List (ungrouped) | 286.2 ms | 137.8 ms | 144.0 ms | 85.2 ms | 36.9 ms | 3.4 ms | 158.7 ms | 24.3 ms (13.4 ms) | 0.65 / 1.85 / 2.14 ms (`0` views) |
| List (grouped by type) | 617.9 ms | 183.4 ms | 192.8 ms | 132.9 ms | 255.5 ms | 4.3 ms | 223.1 ms | 85.2 ms (76.4 ms) | 0.67 / 1.92 / 2.03 ms (`0` views) |

### Slowest turns during staged load

Each entry shows `total_ms | rows_before -> rows_after | glib_ms | frame_ms`.

- **Columns (root):**
  - `31.08 ms | 0 -> 128 | glib=20.87 ms | frame=10.21 ms`
  - `25.56 ms | 57472 -> 73856 | glib=8.43 ms | frame=17.14 ms`
  - `22.50 ms | 47232 -> 57472 | glib=12.07 ms | frame=10.43 ms`
  - `22.12 ms | 73856 -> 90240 | glib=8.73 ms | frame=13.39 ms`
  - `19.26 ms | 90240 -> 99998 | glib=5.64 ms | frame=13.62 ms`
- **List (grouped by type):**
  - `85.18 ms | 73856 -> 90240 | glib=8.74 ms | frame=76.43 ms`
  - `72.78 ms | 0 -> 128 | glib=64.56 ms | frame=8.21 ms`
  - `61.36 ms | 90240 -> 99998 | glib=5.53 ms | frame=55.82 ms`
  - `56.66 ms | 57472 -> 73856 | glib=8.37 ms | frame=48.28 ms`
  - `45.72 ms | 45184 -> 57472 | glib=12.28 ms | frame=33.43 ms`

### Live view switching (99,998 entries already loaded)

| Transition | `show_view` | Next frame | Total |
| --- | ---: | ---: | ---: |
| Columns → List (grouped) | 116.4 ms | 25.5 ms | 141.9 ms |
| List (grouped) → Icons | 55.5 ms | 14.3 ms | 69.9 ms |
| Icons → Columns | 67.6 ms | 18.1 ms | 85.7 ms |

## 3. Isolated micro-benchmarks (`99,998` entries, `--profile perf`)

| Stage | Time | Notes |
| --- | ---: | --- |
| Native directory enumeration (`strata_fs::adapters::local_files::native`) | 34–44 ms | Observed in live release logs; runs off the UI thread |
| Enumeration + background `sort_entries` (to `EntriesReplaced`) | 135.3 ms | Implies ~90–100 ms for single-threaded natural-name sort |
| `Browser` total including 51 staged publish chunks | 202.0 ms | Empty observer |
| `Row::of` × 99,998 (cold `Ids`) | 63.4 ms | Runs on UI thread inside `events::read` |
| ↳ `Ids::of(&entry.location)` (cold / warm) | 25.2 / 12.8 ms | Clones and inserts 100k `Location`s into `HashMap` |
| ↳ `entry_icon::entry_icon` | 17.0 ms | Allocates `name.to_ascii_lowercase()` + table scans |
| ↳ `thumbnails::key_of` | 11.2 ms | Calls `kind::of(path)` (lowercased extension alloc) + SipHash |
| ↳ `thumbnails::is_rendered_model` | 8.9 ms | Calls `kind::of(path)` a second time per entry |
| `Column::apply(Edit::Replace)` ungrouped (100k at once) | 15.1 ms | Single pass |
| `Column::apply(Edit::Insert)` ungrouped (50 staged chunks) | 28.6 ms | Uses append fast path |
| `TypeGroup::of_row` × 99,998 (GIO MIME lookup) | 56.4 ms | 0% hit rate in `TYPE_CACHE` on unique filenames |
| `Column::apply(Edit::Replace)` grouped (100k at once) | 69.2 ms | Includes one 100k `TypeGroup::of_row` pass |
| `Column::apply(Edit::Insert)` grouped (50 staged chunks) | 175.9 ms | Re-indexes and re-buckets all accumulated rows per chunk |
| `row_key(id_at(i))` × 99,998 (1 virtual-list key scan) | 2.2 ms | Allocates one `String` per row |
| `Browser::column_entry_counts` (first call) | 0.42 ms | Cached thereafter |
| `Browser::filled_entries` | 0.00 ms | Negligible |

## 4. Observed bottlenecks

### 1. Quadratic re-indexing and regrouping in grouped List mode

In `crates/strata-app/src/listing.rs` (`Column::insert`), whenever `self.grouping.is_some()`,
every staged `EntriesPublished` chunk takes the full `splice` + `self.index()` + `self.group()`
path instead of incremental insertion. Across 51 chunks (`MAX_PUBLISH_CHUNK = 2048` in
`crates/strata-fs/src/app/browser/publication.rs`), `Column::index` rebuilds `positions`
(`HashMap<EntryId, usize>`) and `Column::group` re-buckets all rows published so far
(`2k + 4k + … + 100k` ≈ 2.5M entries processed). `Pane::sync_by` (`crates/strata-app/src/pane.rs`)
also calls `groups_of(column)` across all rows on each chunk.

- **Measured impact:** `shell::poll` rises from `36.9 ms` (ungrouped) to `255.5 ms` (grouped),
  with single-frame stalls up to `76.4 ms` (`85.2 ms` turn).

### 2. Zero-hit-rate MIME cache and duplicate full-directory MIME scan

In `crates/strata-fs/src/services/mime_type.rs` (`mime_description_for_name`), `TYPE_CACHE`
keys by the full filename with `TYPE_CACHE_LIMIT = 2048`. In a directory of 99,998 unique
filenames, every lookup misses and clears the cache ~49 times, invoking
`gio::content_type_guess` and `gio::content_type_get_description` 99,998 times (`56.4 ms` per
pass).

Additionally, in `crates/strata-fs/src/app/navigation/type_groups.rs`
(`ColumnState::first_displayed_entry`), when a grouped folder contains no subdirectories,
`first_visible_folder()` returns `None` and `first_in_file_groups()` runs `entry_type()` across
all 99,998 entries synchronously on the UI thread inside `install_snapshot` before the first
128 rows are published. `Column::group` in `strata-app` then performs a second full 99,998-row
MIME pass.

- **Measured impact:** `+45 ms` UI-thread stall on the `0 -> 128` turn (`64.6 ms` vs `19.3 ms`
  ungrouped) plus `56.4 ms` in `Column::group` (`~100 ms` total in grouped List).

### 3. UI-thread 100k `HashSet` uniqueness scan before the first row shows

In `crates/strata-fs/src/app/navigation.rs` (`lists_each_location_once`), called via
`selected_positions` during `publish_sorted_snapshot`, a `HashSet<&Location>` of capacity
99,998 is allocated and populated on the UI thread to check whether every entry has a unique
location, even for native directory listings where the filesystem guarantees uniqueness.

- **Measured impact:** `~19–21 ms` UI-thread stall on the `0 -> 128` turn in every view mode.

### 4. Repeated Rune virtual-list key scans across 51 staged publish chunks

`Browser::publish_staged` (`crates/strata-fs/src/app/browser/publication.rs`) publishes 99,998
entries in 51 chunks of up to 2,048 rows. Each rendered frame that receives new chunks changes
`len` and `.virtual_rows_version` (`crates/strata-components/src/collection/element/mod.rs`),
causing Rune to re-scan all `k` row keys published so far (Rune #203), allocating a `String`
per line in `row_key`.

- **Measured impact:** `82–159 ms` cumulative Rune commit/layout/key time across the load
  (`223 ms` in grouped List), with the final 4 chunk frames (`57k -> 74k -> 90k -> 100k`) taking
  `12–17 ms` each even in ungrouped modes.

### 5. Per-row allocations and duplicate work in `Row::of`

`crates/strata-app/src/listing.rs` (`Row::of`) runs on the UI thread for every entry copied
from `Browser`:

- `thumbnails::key_of(entry)` (`11.2 ms`) and `thumbnails::is_rendered_model(entry)` (`8.9 ms`)
  each call `thumbnails::kind::of(path)` separately, parsing and lowercasing the file extension
  twice per entry.
- `entry_icon::entry_icon(entry)` (`17.0 ms`, `crates/strata-app/src/entry_icon/mod.rs`)
  allocates `name.to_ascii_lowercase()` for every file and linearly scans `FILENAME_ICONS`,
  `FILENAME_AFFIX_PATTERNS` and `EXTENSION_GROUPS`.
- `Ids::of(&entry.location)` (`25.2 ms` cold, `crates/strata-app/src/slots/ids.rs`) clones and
  hashes 99,998 `Location`s into an unreserved `HashMap`.

- **Measured impact:** `63.4 ms` total UI-thread time per 99,998 entries.

### 6. Single-threaded natural-name sort

`crates/strata-fs/src/app/navigation/ordering.rs` (`sort_entries`) sorts all 99,998 entries on
one background thread (`gio::spawn_blocking`), taking `~90–100 ms` after the `34–44 ms` native
`readdir` enumeration. This dominates time-to-first-visible-row (`141–193 ms`).

### 7. Continuous animation frames while waiting for background load

In the offscreen runs for `icons`, `list` (ungrouped and grouped) and `columns` (child),
`runtime.frame()` returned `animating = true` on every turn while waiting on background
enumeration and sorting (`936` to `2,460` turns with `wait = 0 ms`, compared to `22` turns and
`132 ms` of idle wait in `columns` root), adding unnecessary CPU work during the load wait.

### 8. Startup latency and memory footprint (`315–324 MiB` Peak PSS)

- **Window map latency:** `566–616 ms` from process launch to Wayland window registration
  (Vulkan/`wgpu`, font loading, theme setup, single-instance D-Bus claim and GLib loop
  initialization).
- **Duplicate 100k entry storage:** Opening 99,998 entries retains three concurrent copies of
  the directory's names/paths and two large hash tables in memory:
  1. `strata_fs::app::Browser`'s `Vec<FileEntry>` (`Location`, `display_name`, `native_name`,
     metadata fields).
  2. `strata_app::slots::Ids`'s `HashMap<Location, EntryId>`, which retains every issued
     `Location` for the lifetime of the process (`crates/strata-app/src/slots/ids.rs`).
  3. `strata_app::listing::Column`'s `Vec<Row>`, `HashMap<EntryId, usize>`, `shown: Vec<usize>`
     and (when grouped) `back: Vec<usize>` (`crates/strata-app/src/listing.rs`).
- **Process tree overhead:** Sandboxed thumbnail worker processes (`--browser-worker` under
  bubblewrap, up to `thumbnail_workers = 4`) plus shared GLib/GIO and graphics driver mappings
  contribute to the `315–324 MiB` peak process-tree PSS.

## 5. Follow-up measurements (2026-11-22)

After addressing bottlenecks #1–#8, re-measured on the same 24-core machine and 99,998-image fixture:

### Windowed launch timings (`--release`, LTO, private headless `sway`, median `[min, max]` over 3 runs)

| View mode | Mapped (ms) | Settled (ms) | Peak PSS (MiB) | Total CPU (s) |
| --- | ---: | ---: | ---: | ---: |
| Columns | 623 `[588, 739]` | **954** `[927, 1,083]` (was `1,633`, `-42%`) | **252.6** `[251.4, 252.7]` (was `315.3`, `-62.7 MiB`) | **0.96** `[0.92, 1.03]` (was `1.37`, `-30%`) |
| List (ungrouped) | 581 `[560, 602]` | **938** `[884, 1,075]` (was `1,665`, `-44%`) | **253.0** `[252.5, 253.5]` (was `315.2`, `-62.2 MiB`) | **0.86** `[0.85, 0.96]` (was `1.33`, `-35%`) |
| List (grouped by type) | 557 `[528, 586]` | **877** `[874, 935]` (was `1,922`, `-54%`) | **253.6** `[253.6, 253.9]` (was `323.7`, `-70.1 MiB`) | **0.89** `[0.84, 0.93]` (was `1.82`, `-51%`) |
| Icons (62 pt tiles) | 578 `[571, 604]` | **888** `[770, 906]` (was `1,887`, `-53%`) | **254.7** `[230.1, 254.9]` (was `319.6`, `-64.9 MiB`) | **0.84** `[0.74, 0.84]` (was `2.10`, `-60%`) |

### Offscreen phase breakdown (`--profile perf`, 99,998 images)

| Mode | Total to `LoadFinished` | To `EntriesReplaced` (enum + sort) | To first 128 visible rows | GLib dispatch | Rune frames | Turns before `EntriesReplaced` (`anim`) | Idle wait |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Columns (root) | **170.5 ms** (was `340.3 ms`) | **99.8 ms** (was `158.2 ms`) | **105.7 ms** (was `169.9 ms`) | **51.1 ms** (was `93.0 ms`) | **30.8 ms** | `21` (`1`) | `81.8 ms` |
| Icons (62 pt tiles) | **170.4 ms** (was `266.7 ms`) | **105.8 ms** (was `133.8 ms`) | **112.7 ms** (was `141.2 ms`) | **51.0 ms** (was `88.0 ms`) | **23.5 ms** | `23` (`1`) | `88.1 ms` |
| List (ungrouped) | **170.5 ms** (was `286.2 ms`) | **98.4 ms** (was `137.8 ms`) | **105.3 ms** (was `144.0 ms`) | **51.3 ms** (was `85.2 ms`) | **30.6 ms** | `19` (`1`) | `82.1 ms` |
| List (grouped by type) | **183.3 ms** (was `617.9 ms`) | **105.4 ms** (was `183.4 ms`) | **110.6 ms** (was `192.8 ms`) | **51.9 ms** (was `132.9 ms`) | **41.5 ms** | `17` (`1`) | `81.3 ms` |

### Startup and memory baseline (empty folder, `0` files, `--release`)

- **Mapped / Settled:** `560–573 ms` / `588–600 ms` (`0.52–0.54 s` CPU), `145.2–147.0 MiB` PSS.
- **Breakdown:** Strata's own process/state/font setup (`UiThread::claim` + `Saved::load` +
  `ThemeEngine::load` + `App::new` + `TextSystem::new`) accounts for `~25 ms`; the remaining
  `~540 ms` map time and `~146 MiB` base PSS are `winit` / `wgpu` / graphics driver / Wayland
  compositor initialization inside Rune.
- **99,998-image delta over empty baseline:** `+~290–350 ms` settled time, `+~0.31–0.42 s` CPU,
  `+~78 MiB` main-process PSS (`~224–227 MiB` main + `4 × 6.8 MiB` sandboxed `--browser-worker`
  processes). `strata_app::slots::Ids` now prunes unreferenced entries above `131,072` mappings
  between folder/tab transitions so long sessions visiting multiple large directories stay
  bounded.
