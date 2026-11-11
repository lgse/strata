# Testing

How the gallery, its goldens and its scripts fit together, and how a work package adds to
them without touching another's files. The plan's testing rules are in
[`plan.md` §6](plan.md#6-testing). The repository rules are in [`AGENTS.md`](../AGENTS.md).

## Gallery pages

The gallery (`crates/strata-gallery`) has an index and one page per component, plus a page
for each contract or look work package that shows something (W1, W2, K1–K4, C10a). The
registry in `src/scenes/mod.rs` declares every page. A work package edits only its own
`src/scenes/<id>.rs`, which provides six items:

| Item | What it is |
| --- | --- |
| `Demo` | The page's demo state: a `Default` type, held in `Gallery::<id>` |
| `SCENES` | Golden scenes: each gets an accessibility golden and a PNG golden |
| `REVIEW` | Scenes for review with `--scene` only, kept out of the goldens |
| `view` | The page's view, `fn(&Gallery, &mut Cx<Gallery>) -> El` |
| `on_key` | `AppSpec::on_key` for the page: keys no focused element took. True consumes the key. |
| `poll` | `AppSpec::poll` for the page: runs every frame while it is open. True when state changed. |

Page ids are the component ids (`c1`, `c14a`, `c20b`) or the work package ids (`w2`, `k3`,
`c10a`). The index's id is `gallery`.

Scene rules, which `tests/goldens/registry.rs` enforces:

- Every page in the registry has golden scenes (`Page::implemented`): every work package has
  landed, so a page added without them fails.
- A scene name starts with its page id and a dash, and ends in `-light` or `-dark`:
  `c1-ten-rows-light`.
- Every `-light` scene has a `-dark` twin.
- Names are unique across the gallery.
- A scene opens its own page in its own theme: it calls `g.open("c1", dark)`, then sets its
  demo state.

Fake data is generated in the page file or in the component's own test fixtures. Pages never
wait on Strata's workers or on platform shims.

### Keys, polls and async results

The app's hooks (`strata_gallery::spec`) route to the open page, in a window and in headless
`--snap` and `--script` runs alike:

- **Keys.** An element's own `on_key` hears keys only while it has focus. Everything else
  (shortcuts such as Ctrl+L, and keys with nothing focused) reaches the page's `on_key`
  first. Only a key it leaves falls back to the gallery: Escape on a page goes back to the
  index. So a page that uses Escape (cancel a marquee, stop an autoscroll) returns true for
  it.
- **Polls.** The open page's `poll` runs at the start of every frame, with the clock in
  seconds: virtual in tests and headless runs. Use it to deliver fake async results
  (completions, highlighted lines, thumbnails, progress), and return true when state changed.
- **Waker.** `Gallery::waker` wakes the app loop from another thread so the page's `poll` runs.
  In tests it is a no-op.

Drive fake async from `poll` on the virtual clock: a fake worker is a queue of results, each
due at a clock time, that `poll` hands over once the clock reaches it. Goldens and scripts then
see the same results every run. A real thread (with `Gallery::waker`) is for windowed demos
only; its results land at times no golden or script can rely on.

Golden scenes are captured at 1 s on the virtual clock (`AT` in `tests/goldens/support.rs`),
late enough for entrance motion to come to rest. Both goldens run the same frames: the PNG
check passes `--snap --at 1`, and the in-process mount (`strata_gallery::runtime`) runs a
first frame at 0 s, then one every 1/60 s up to 1 s, as `--snap` does. So the page's `poll` runs
61 times at the same clock times for both, and
`png::the_png_render_ends_on_the_mounts_clock` checks the two clocks agree. A golden scene
either pre-fills its state or schedules results that come due before 1 s.

Test a page's behavior in process in `crates/strata-gallery/tests/<id>.rs`, with the tests
inside `mod <id> { … }`, so `mise run test <id>::` runs them with the page's goldens. Mount a
scene with `strata_gallery::runtime`, send keys with `Runtime::key` or pointer input, step
`Runtime::frame`, and read the state; or write a script. `tests/goldens/keys.rs` shows the
key routing itself: a page hook that takes Escape keeps the page open.

### The Look

W2 replaces `Gallery::kit` with the Look contract. Its PR is the one sanctioned edit to
strata-gallery's `src/lib.rs`: it adds a `look: Look` field with the accessor components take
(`fn(&Gallery) -> &Look`), and makes `Gallery::open` rebuild it for `dark`. From then on, K1, K2
and K4 change `g.look` (a theme, density, text size or override) only from their own
`scenes/<id>.rs`.

W2's token sheet goldens (`w2-tokens-*`) show the core tokens only. Slot tokens appear only on
the `w2-slots-light` and `w2-slots-dark` review scenes, which have no goldens, so adding a slot
token never moves a golden outside the component's own page.

The gallery draws in the bundled JetBrains Mono (`strata_kit::fonts`, K6), in both font slots,
as Strata does. `strata_gallery::spec` passes it to `AppSpec::fonts` and `families`, and
`strata_gallery::runtime` builds its text system from the same fonts, so in-process mounts (the
accessibility goldens, page tests) shape text as `--snap` and `--script` runs do. A page test
that builds a runtime of its own with `Runtime::new` shapes in Rune's built-in Inter instead;
use `strata_gallery::runtime`. strata-components' own unit tests use `Runtime::new` and stay
in Inter.

`rune::text::measure` (and `advance`, `ink`, `font_epoch`) reads process-wide state: any text
system built elsewhere (`TextSystem::new`, `Runtime::new`) moves its fonts, and a runtime's
`set_font_families` (K2's picker) moves its families. A `rune::text::with_families` guard taken
first, with `TextSystem::new(fonts::FONTS, fonts::FAMILIES)` (or the runtime) built after it on
the same thread, narrows that hazard, since the guard copies whatever fonts are process-wide
when it's made and the text system built under it lands the bundled ones there. It doesn't
close it (R-K6-1 below). So a test that needs the measure path's fonts pinned lives alone in
its own test binary, with nothing beside it that builds a text system: strata-kit's
`tests/fonts_weight.rs`, and the gallery's `tests/k6.rs`, which holds one test. A test that
only needs a font's measurements measures on a `TextSystem` of its own
(`TextSystem::measure`), which no other test can move.

**Rune draft R-K6-1 (draft, not filed): a measurer can keep the wrong fonts.** At the pinned
rev, `with_text` (`crates/rune/src/text/measure.rs`) builds a thread's measurer from the fonts
it read, then reads the state's `fonts_generation` again (line 226) to tag it, with no lock held
across the build. A `TextSystem::new` with other fonts on another thread in between leaves the
measurer holding the old fonts under the new fonts' generation. Later, the families-only branch
(line 216) sees that generation match and keeps the loaded fonts, changing only the families:
text measures in the old fonts under the new names. In strata-kit this showed as JetBrains
Mono's name over Inter's fonts (258 pt where 600 was due, about one run in six). There, a
`with_families` guard taken while the process had no measure state yet built the measurer
(Inter, from `TextSystem::default()`), while a parallel test's `TextSystem::new` with
JetBrains Mono moved the state. Repro sketch, headless, run in a loop of fresh processes:
thread A takes `with_families("F", "F")` on a process with no text system yet, while thread B
calls `TextSystem::new(&[FONT_F], ("F", "F"))`. A then calls `TextSystem::new(&[FONT_F],
("F", "F"))` and `rune::text::advance` on a run whose width tells F from Inter: some runs
measure Inter. A fix would tag the measurer with the generation it read the fonts at, or hold
the state's lock from the read through the tag.

Open the gallery with `mise run gallery`. A button on the index opens its page (click, Space
or Return), and Escape returns to the index. `mise run gallery -- --scene c1-ten-rows-dark`
opens one scene, and `-- --list-scenes` lists them all.

### The index's markers

The index groups the pages by their registry `lane` and marks each one built or not built.
`Page::implemented` decides it: a page is built once it has golden scenes. The marker is a dot
on the button plus its accessible description, and a count and legend sit above the lanes. The
app and a `--snap` without `--scene` show them, but the golden scenes `gallery-index-light` and
`-dark` turn them off (`Gallery::index.markers`). So landing a page's first scenes never
changes the index goldens, and parallel branches don't conflict on them. Review the markers with
`--scene gallery-index-status-light` or `-dark`.

## Goldens

```
tests/golden/a11y/<id>/<scene>.json      the accessibility tree, stripped (below)
tests/golden/png/<id>/<scene>.png        the scene rendered headless at --scale 1, 960×640
tests/golden/scripts/<id>/<script>.json  the accessibility tree a script ends on, stripped
tests/scripts/<id>/<script>.json         the scripts
```

Stripped trees leave out bounds, scroll offsets, text selection and shortcuts: layout and
per-platform spelling, not semantics.

Each page gets three tests in `crates/strata-gallery/tests/goldens/`: `<id>::a11y`,
`<id>::png` and `<id>::scripts`. A page with no scenes or scripts passes them trivially.

- A golden with no scene or script behind it fails the test. Rename or remove a scene, then
  bless the page, and the stale golden is deleted. A directory inside a page's golden folder
  also fails.
- A failure lists every differing item with the golden's path and a fresh copy under
  `target/tmp/goldens/<kind>/<id>/`. For a PNG it also writes `<scene>-diff.png`: each pixel's
  largest channel difference, times 4, in red.

### PNG tolerance and the renderer

A pixel is off when its largest channel differs by more than 24/255. That is the "visible"
threshold of rune-kit's gallery diff (`scripts/kit-diff.py` at the pinned Rune rev). No off
pixel is allowed (`MAX_OFF_PIXELS = 0` in `tests/goldens/png.rs`). Blessing keeps a PNG golden
that still matches within the tolerance, so a render on another machine doesn't churn it.

PNG goldens and scripts render through the gallery binary headless. They need a Vulkan
adapter, never the desktop display: the binary runs with `DISPLAY` unset and `WAYLAND_DISPLAY`
set to a socket that doesn't exist (`mise run test` and `bless` do the same), so libwayland
can't fall back to `wayland-0`. The harness picks the adapter in this order:

1. the ICD file named by `STRATA_VK_ICD`;
2. lavapipe for this architecture (`/usr/share/vulkan/icd.d/lvp_icd.x86_64.json`, say), then
   `lvp_icd.json`, then every other `lvp_icd*.json` found, joined with `:`;
3. the default adapter.

CI renders on lavapipe (`mesa-vulkan-drivers`). A golden blessed on a hardware GPU (NVIDIA,
say) can differ from lavapipe's render by more than the tolerance at anti-aliased edges, and
then fails in CI even though it passes locally. Install lavapipe (`vulkan-swrast` on Arch,
`mesa-vulkan-drivers` on Debian and Ubuntu) to bless on CI's renderer. A failure message names
the adapter Rune used.

## Scripts

A script is Rune's headless input script (see Rune's README, "Agent tooling") in object form,
with a header the runner checks:

```json
{
  "bullets": ["browser/selection: Shift+Up and Shift+Down extend the selection"],
  "scene": "c1-ten-rows-light",
  "events": [
    {"t": 0.1, "type": "tap", "key": "row-3"},
    {"t": 0.3, "type": "key", "name": "Down", "shift": true}
  ],
  "expect": [
    {"key": "row-4", "states": ["selected"]},
    {"key": "rename-field", "absent": true}
  ]
}
```

Modifier flags (`shift`, `ctrl`, `alt`, `cmd`, `logo`) reach key and wheel events only, and each
such event carries only its own flags (`script_mods` in Rune's `tooling/key_names.rs`): `logo` is
Super, and `cmd` is Ctrl off macOS. A pointer event (`tap`, `down`, `drag`) ignores them. To hold
a modifier through pointer events, use Rune's `{"t": 0.3, "hold": "ctrl"}` event (any of
`shift`, `ctrl`, `alt`, `super`, `cmd`, joined with `+`), and `{"hold": ""}` to let go (Rune
`4805095`). It sets the window's held modifiers, not a key's or a wheel's: a key or wheel event
sent while one is held has only its own flags (`KeyEvent::mods`, the wheel's `mods`), though
`tx.mods()` reads the held modifiers in any handler, a key's included (every `Tx` starts with
them, `runtime/frame.rs`). A key with a `"code"`, or a `{"physical": …}` chord, is a physical
key, which replaces the held modifiers with its own (`runtime/input/physical.rs`): it ends a
`hold`. A `{"physical": …}` chord then lets its modifiers go.

| Key | Rule |
| --- | --- |
| `bullets` | Required, not empty. Each entry is `<node-id>: <bullet text or a prefix unique within the node>`, naming a feature-map Behavior bullet the script checks (`featuremap.py qa --node <id>` lists them). Node ids are `/`-separated segments of `a-z`, `0-9` and `-`. |
| `scene` | Required: a scene of the page whose folder holds the script (golden or review). |
| `events` | Required, not empty: Rune script events. Rune exits with an error on an unknown event type or key name, or on a `key` that names no element. |
| `expect` | Optional. Each entry has a `key`, plus any of `role`, `name`, `value` (equal), `states` (each present), `focused` (true or false) and `absent` (`true`: no node with that key). |
| `displays`, `pointer` | Optional: passed to Rune as they are. |

Any other key is an error, so typos fail. The runner starts the gallery headless with
`--scene <scene> --scale 2 --script <file> --tree <out>`: scripts run at scale 2, while PNG
goldens render at scale 1. It fails when:

- Rune exits with an error;
- Rune reports key warnings (duplicate keys break scripts that target keys);
- an `expect` entry doesn't hold, in bless mode too;
- the accessibility tree the script ends on differs from its golden.

Run one by hand with
`mise run gallery -- --scene <scene> --script tests/scripts/<id>/<name>.json`, adding
`--snap out.png` or `--tree tree.json` to see where it ends.

Name scripts after the behavior group they check, in lowercase kebab case
(`stem-selection.json`, `commit-and-cancel.json`).

## Running and blessing

| Command | Runs |
| --- | --- |
| `mise run test` | Every test, headless |
| `mise run test c1::` | Page c1's three golden tests. The trailing `::` keeps `c1` from matching `c10`. |
| `mise run test c1::png` | One page's PNG goldens |
| `mise run bless c1` | Rewrites only `tests/golden/*/c1/` |
| `mise run bless c1-ten-rows-dark` | Rewrites one scene's a11y and PNG goldens |
| `mise run bless c5/stem-selection` | Rewrites one script's golden |
| `mise run bless` | Rewrites every golden. Avoid it on a branch: it touches other pages. |

Confirm a filtered run matched at least one test.

Give each worktree its own target directory; don't point several at one `CARGO_TARGET_DIR`.
Cargo can then reuse another worktree's build of a workspace crate, because its dep-info names
that worktree's files, and the tests run stale code. To save a full build, copy a target
directory with `cp -a --reflink=auto`, then run
`cargo clean -p strata-gallery -p strata-components -p strata-kit -p strata-platform` in the new
worktree.

`RUNE_BLESS=1` turns blessing on, as in Rune. `STRATA_BLESS` narrows it to a page id, a scene
name or `<id>/<script>`. While blessing, the harness skips every other item: it neither compares
nor writes it. `mise run bless <filter>` sets both and passes the cargo filters `<id>::` and
`registry::bless_filter_names_a_page_scene_or_script`.

- The filter must be lowercase (`a-z`, `0-9`, `-`, `_`, `/`); the task rejects anything else.
- A filter that names no page, no golden scene of its page or no script of its page fails the
  run, so a typo never passes by blessing nothing. Review scenes have no goldens, so their
  names fail too.
- A script whose `expect` entries fail is never blessed.

After blessing, run `mise run test` and review every golden diff (look at each PNG) before you
commit.

## strata-platform features

`mise run compile-features` checks `strata-platform` with no features, then with each feature
alone, so no shim leans on another's dependencies. `mise run check` and CI run it.

- Each shim's feature and its optional dependencies have their own block in
  `crates/strata-platform/Cargo.toml`. Fill in only your own.
- A crate appears once in `[dependencies]`. If another shim's block already declares a
  dependency you need (zbus for P5 and P8, say), move it to the shared block at the top of
  `[dependencies]`, keep it `optional = true`, and enable it with `dep:<name>` from each feature
  that uses it. Two blocks that each declare it merge cleanly in git but make the manifest
  invalid.
- `_transfer` is internal: P0's substrate, enabled by `dnd`, `clipboard` and `foreign-parent`.
- An example gates its own body on its feature, so it needs no `[[example]]` entry and builds
  in every feature combination:

  ```rust
  #[cfg(feature = "csd")]
  fn main() { /* the demo */ }

  #[cfg(not(feature = "csd"))]
  fn main() {
      eprintln!("run with --features csd");
  }
  ```

## Dependencies

Every crate's `[dependencies]` has a comment-separated block per work package that adds
third-party crates, plus a shared block:

| Crate | Blocks |
| --- | --- |
| strata-platform | one per feature (P0–P9) |
| strata-kit | K1, K2, K3, K4; `[build-dependencies]`: K3 |
| strata-components | C10a, C11, C12, C13, C14b, C25 |
| strata-gallery (dev) | the X1, X2 and X3 demos |

- Add a crate to `[workspace.dependencies]` (one line, in alphabetical order; check its licence
  against `deny.toml`), then `<name>.workspace = true` in your block.
- In strata-kit, strata-components and the gallery, a crate declared in any block is usable
  from every module: use it as it is, and don't declare it again.
- C10a adds `syntect` and `two-face` to `[workspace.dependencies]` and to its strata-components
  block. K1 needs syntect's `Theme` type and reuses the workspace entry: `syntect.workspace =
  true` in its own strata-kit block, with the features it needs. C10b and C11 use C10a's
  declaration as it is.

## Shared lists and merge conflicts

Git conflicts when two branches change neighbouring lines. W0 separates the lines that
different work packages edit with a blank line or a comment: the strata-platform features and
dependency blocks, and the gallery's demo dev-dependency blocks (X1, X2, X3, plus a shared
block for a crate two demos need, such as strata-platform with their features), are laid out
this way. Keep those separators. Lists without them, such as
`[workspace.dependencies]` and a crate's `[dependencies]`, can still conflict when two branches
add neighbouring entries. Resolve those on rebase. For `Cargo.lock`, take `main`'s copy and run
`cargo check` to regenerate it ([shared files](work/README.md#shared-files)).

## Compositor and D-Bus tests

strata-platform's shims run against a real compositor and a real session bus. W3's harness
(`crates/strata-platform/tests/support/`) gives each test a private session of its own:

- a session bus: a `dbus-daemon` whose only service directory is the session's;
- a nested headless sway (`WLR_BACKENDS=headless`), with Xwayland;
- an input peer: a virtual pointer and keyboard (a headless seat has neither, so without them
  no client gets pointer events or keyboard focus);
- a private `XDG_RUNTIME_DIR` and home, removed on teardown. Logs stay in
  `target/tmp/session/<test>/` (`<test>-2` for a second session in the same test);
- a watchdog, started first, in a process group of its own that ignores SIGINT, SIGTERM and
  SIGHUP. It knows the test process by its pid and start time. If the test dies without tearing
  down (SIGKILL, Ctrl-C, a CI timeout), it kills the session's processes, removes the Xwayland
  lock file and socket a killed sway leaves, unmounts what is mounted in the directory, and
  removes it (`rm -rf --one-file-system`).

A session's processes are the ones started with `XDG_RUNTIME_DIR` set to its directory, found
by scanning `/proc/*/environ`: the bus, sway, Xwayland (which wlroots forks twice), every
spawned process and any service the bus activates. The desktop's processes have
`/run/user/<uid>`, so nothing outside the session matches, and no pid number that may have
been reused is ever killed (`<runtime dir>/pids` is kept for diagnostics only). The watchdog
finds Xwayland's display from the sockets those processes hold, so it also cleans up after a
sway killed before it said which display it took, and removes a lock only if it names one of
them. Dropping a session waits for the same set of processes to exit, then kills what is left
after the timeout.

Every process starts with an environment built from nothing, so none can reach the desktop's
display, bus or runtime directory. `mise run test-platform` saves the desktop's `DISPLAY`,
`WAYLAND_DISPLAY` and `XDG_RUNTIME_DIR` as `STRATA_OUTER_*` before clearing them, and the
harness refuses a session display that matches them.

### Running them

| Command | Runs |
| --- | --- |
| `mise run test-platform` | Every session test |
| `mise run test-platform p2::` | One work package's |
| `mise run test-platform -- w3:: --test-threads=1 --nocapture` | Any libtest arguments |

The task builds the peer and strata-platform's examples, then runs the `#[ignore]`d tests of
`crates/strata-platform/tests/session/`. `mise run test` and `check` never start a session.

| Variable | Effect |
| --- | --- |
| `STRATA_VK_ICD` | The Vulkan ICD clients use. Without it, lavapipe where installed, else the default driver. An ICD file outside the system's must name its `library_path` absolutely; `LD_LIBRARY_PATH` passes through too. |
| `STRATA_WLR_RENDERER` | sway's renderer. Without it: pixman when the ICD is lavapipe (CI's path, no GPU), else gles2. A hardware Vulkan driver presents through `linux-dmabuf`, which pixman doesn't offer. |
| `STRATA_TEST_TIMEOUT_SCALE` | Multiplies every wait (10 s). CI sets 3. |
| `STRATA_SWAY_DEBUG=1` | sway's debug log in `sway.log` |
| `STRATA_TEST_PEER` | The peer binary, instead of `target/<profile>/strata-test-peer` |
| `STRATA_SESSION_REPORT` | A file a session appends its directory and pids to (W3's kill tests) |
| `STRATA_SESSION_NAME` | The session's log directory name, instead of the test's (a test binary run as a child) |

CI runs them in `.github/workflows/platform.yml` on Ubuntu 26.04 (sway 1.11, libwayland
1.24), with the packages in `crates/strata-platform/tests/support/packages.txt`. A work
package that needs another package adds its own block there.

### Writing one

The tests are one binary: `tests/session/main.rs` declares a module per work package
(`w3`, `p0`–`p5`, `p8`), gated on the package's feature. Write yours in
`tests/session/<id>.rs`; don't edit `main.rs`. Mark each test
`#[ignore = "needs a private session: mise run test-platform"]`.

`crates/strata-platform/examples/harness-window.rs` is W3's: a plain Rune window with no shim
that logs `ready`, `frame`, `layout`, `focus`, `press`, `secondary-press` and `key` as JSON
lines. Use it, or spawn your own work package's example.

```rust
let s = Session::start();
let cmd = s.command(example("csd"), Display::X11);
let app = s.spawn("csd", cmd);
app.wait_json("ready", |v| v["ev"] == "ready");
let window = s.wait_window(app.pid());
s.input().click_at(window.at(0.5, 0.5));
```

| Call | Does |
| --- | --- |
| `Session::start()`, `Session::with(Options { .. })` | A session. `compositor: false` gives the bus alone (P5, P8). |
| `s.command(program, Display::Wayland \| X11 \| None)` | A `Command` with the session's environment |
| `s.spawn(name, cmd)` | Starts it as a `Proc`: stdout kept line by line, both streams logged, killed on drop |
| `proc.wait_json(what, pred)`, `proc.wait_json_from(at, what, pred)`, `proc.wait_line(at, what, pred)`, `proc.wait_for(at, what, f)`, `proc.line_count()`, `proc.send_line(line)` | The first matching JSON line, or from a point on (with its index); any line; any mapping of a line; write to stdin |
| `example(name)` | `target/<profile>/examples/<name>` |
| `s.wait_window(pid)`, `s.wait_window_where(what, pred)`, `s.wait_windows(&[pids])`, `s.focus(&window)`, `s.tree()` | sway's tree: a window, any window, windows tiled apart, keyboard focus, the raw tree. A `Window` has `id` (`[con_id=…]`), `pid`, `shell`, `app_id`, `title`, `content`, `focused` and `floating`. |
| `s.input()` | The virtual pointer and keyboard: `click_at`, `right_click_at`, `pointer_move`, `button`, `button_of`, `path`, `key("ctrl+c")`, `key_down("shift")`, `key_up("shift")` |
| `s.point_until(at, reached)`, `s.point_at(&peer, at)` | Moves the pointer until `reached()` holds; until `peer` reports an enter or motion after the move |
| `s.xwayland()`, `s.runtime_dir()`, `processes_of(dir)`, `alive(pid)` | Xwayland's pid and display, the runtime directory, the processes started with it, whether a pid runs |
| `s.peer(display)` | A peer process ([below](#the-peer)) |
| `s.bus()` | The bus's `address()`, `add_service(name, exec)` (written whole, then the bus reloads), and `service_dir()` |

### The peer

`strata-test-peer` (`tests/support/strata-test-peer/`) is the second process, as Strata's
`tests/qa/foreign_drag_source.py` is: raw `wayland-client` on Wayland and `x11rb` on X11. It
sets and reads selections (`wl_data_device`, data control, X11 `CLIPBOARD` and `PRIMARY`, with
INCR), drags and accepts drops (Wayland and XDND), exports its window, and drives the virtual
devices. Tests drive it with typed calls (`set_selection`, `read`, `drag_start`,
`accept_drops(mimes, actions, preferred)`, `drop_proxy` for an X11 `XdndProxy`, `export`,
`wait_event`, `pointer_since`) over JSON lines. Its `protocol` module documents them. It
reports pointer enter, motion and leave on its window, and as an X11 drag source it honours a
target's `XdndProxy`.

### Traps

- **Wait for the layout before input.** On X11 a window maps at its own size and sway tiles it
  after. Input sent before the app lays out again can land outside its old size. The
  harness's example logs `layout`; W3's tests wait until it matches sway's size.
- **An X11 window can stay unmapped in sway.** wlroots up to 0.20.2 (Ubuntu 26.04 has 0.19)
  maps an Xwayland window only on a buffer commit that arrives after it has paired the window
  with its `wl_surface`. The commit and the pairing travel on different sockets, so a fast
  first commit leaves the window unmapped for good. wlroots fixed this after 0.20.2. The X11
  peer's `map` waits until the window manager lists the window in `_NET_CLIENT_LIST`, and maps
  it again every 500 ms (times `STRATA_TEST_TIMEOUT_SCALE`) until it is, or the test's own
  timeout fails it. About one window in five needed that when two peers mapped at once. An
  app's own X11 window can't be retried this way: if `wait_window` times out on an X11 app,
  this is the likely cause (W3's example window never hit it in over 60 opens).
- **Point, then press, after a relayout.** A new window re-tiles the others. For a moment
  after, a pointer motion can reach no surface (likely sway's saved snapshots of windows it is
  resizing), and a press then goes nowhere, even after `wait_windows` and a `sync()` with each
  peer. This failed a drag on pixman about one run in ten. `s.point_at(&peer, at)` moves the
  pointer, syncs with the peer, and moves it again a point off until the peer reports an enter
  or a motion after the move (an enter from before it doesn't count).
- **Focus needs the keyboard.** `wl_data_device.set_selection` needs keyboard focus on the
  peer's window (`s.focus`); data control needs none.
- **Floating windows.** `floating enable, resize set` didn't always reach a peer as a configure,
  and its old, larger surface then covered the window under it. W3's tests stay tiled.
- sway launches as `setpriv --no-new-privs sway --unsupported-gpu`. The first works around a
  `cap_sys_nice` sway under a zero `RLIMIT_RTTIME`; the second lets sway start on a host
  with NVIDIA's kernel module (sway 1.9 refused to).

### Publishing strata-platform

`strata-test-peer` is a dev-dependency of strata-platform with a version, because cargo-deny's
`wildcards` ban rejects a path without one, but it is never published. So publishing
strata-platform needs one of these first (owner's call; kept as it is for now):

- `allow-wildcard-paths = true` in `deny.toml`'s `[bans]`, which lets a path dev-dependency
  without a version through (needs the owner's approval), with the version then dropped; or
- dropping the dev-dependency from strata-platform's manifest for the release.

### Testing internal modules

A session test can only spawn binaries and call strata-platform's API. P0's substrate is
internal, so `lib.rs` declares it `#[doc(hidden)] pub mod transfer` behind the internal
`_transfer` feature: it is no public API and outside semver, and only strata-platform's own
examples and tests use it. `examples/transfer-probe.rs` is a Rune window with the substrate
attached, driven by JSON lines on stdin; `tests/session/p0.rs` runs it against the peer. P1, P2
and P4 test through their own public APIs. P4's probe, `examples/foreign-parent.rs`, opens a
window per `open` command hidden, parents it from `App::on_window` and shows it, as a portal
backend opens its dialog; `tests/session/p4.rs` parents it to the peer's exported window.

P0's tests that need winit's own Wayland display ("shared" in `p0.rs`) need libwayland 1.23 or
later (`wl_proxy_get_display`), the minimum supported version. On an older, unsupported host,
Ubuntu 24.04 among them, they check only that the probe reports that route Unsupported, and say
so on stderr.

### X2's session tests (the gallery's `tests/x2.rs`)

X2's drag-and-drop wiring is tested in the gallery, not in strata-platform: its app is the
gallery's `dnd_wiring` example, which needs strata-components. `tests/x2.rs` reuses W3's
harness through `#[path]` (it uses none of the helpers the harness gates on strata-platform's
features, whose `expect(dead_code)` would otherwise not hold), and `mise run test-platform`
builds the example and runs these tests after strata-platform's (`mise run test-platform
x2::` runs only them). Each test makes a fixture folder in the session's runtime directory,
starts the demo on it, finds targets with its `locate` command (window points from
`Cx::window_layout`, resolved again at every view until they settle: a grid reflows, a pane
just shown lays out again) and drags with the session's pointer.

| Variable | Effect |
| --- | --- |
| `STRATA_TEST_NAUTILUS=1` | Also runs the Nautilus drags (both ways, best effort), as P2's opt-in test does |

Without `STRATA_TEST_NAUTILUS=1` the Nautilus tests run nothing, but libtest has no skipped
result, so they report `ok`. Each prints `SKIP x2::<test>: …` on stderr, past libtest's
capture, so the line shows in every run. Count those tests as skipped, never as passed.

Traps: a demo window is found by its exact title (the second is `<title> second`), so the
demo logs C7's title intent instead of applying it; a drop into another window of the app
needs time for that window to answer the compositor (its next update), so `drag_out` settles
on the target before the release; the sidebar lays its pin gaps out again while a drag is
over it, so a gap is located during the drag.

### X3's session tests (the gallery's `tests/x3.rs`)

X3's clipboard wiring is tested in the gallery, as X2's drag and drop is: its app is the
gallery's `clipboard_wiring` example, which needs strata-components. `tests/x3.rs` reuses W3's
harness through `#[path]` (it uses none of the helpers the harness gates on strata-platform's
features), and `mise run test-platform` builds the example and runs these tests after
strata-platform's (`mise run test-platform x3::` runs only them). Each test makes a fixture
folder in the session's runtime directory, starts the demo on it with `--apply`, sets it up
with its `open` and `select` commands, and presses Ctrl+C, Ctrl+X and Ctrl+V with the
session's keyboard. The peer reads what the demo offers and offers what it pastes; the
demo's JSON lines (`copied`, `badge`, `paste`, `state`) and the files on disk say what it did.
The data-device variant passes the demo's test-only `--as-if-no-data-control` (P2's
`TestRoute`), so its writes take the key's serial, as on mutter. It proves the route carries
the copy, not that the copy runs inside the key's handler: wlroots takes any serial it has sent
the client. The partial-cut case makes a move fail partway with a read-only folder (a folder
moves only with write access to it), and restores the mode before it pastes again.

| Variable | Effect |
| --- | --- |
| `STRATA_TEST_NAUTILUS=1` | Also runs the Nautilus round trips (copy in Nautilus, paste in the demo; cut in the demo, paste in Nautilus), best effort |

Without `STRATA_TEST_NAUTILUS=1` the Nautilus tests run nothing but report `ok`; each prints
`SKIP x3::<test>: …` on stderr, past libtest's capture. Count those tests as skipped, never as
passed.

Trap: the demo hears another app's copy only as a change of the badge, so a test that offers
files twice in a row first offers text (the badge turns off), then the files (it turns on
again): otherwise a paste could read the clipboard before the demo heard the new copy.

## Media output (P7's `media-audio`)

The sink's tests run in `mise run test` with the rest: they use `Output::Null` (GStreamer's
`fakesink`) on a manual GStreamer clock, so they open no sound server and time moves only when
a test moves it. They need GStreamer's `appsrc`, `audioconvert`, `audioresample` and `volume`
(gst-plugins-base) at run time; on Ubuntu that is `gstreamer1.0-plugins-base`. The first
`gst::init` writes GStreamer's plugin registry cache under `~/.cache/gstreamer-1.0/`.

One test plays sound and is for a person: `media::sink::tests::plays_a_tone_on_the_default_output`
plays two seconds of 440 Hz on the default output. It is `#[ignore]`d and plays only with
`STRATA_PLATFORM_TONE=1`:

```sh
STRATA_PLATFORM_TONE=1 cargo test -p strata-platform --features media-audio plays_a_tone -- --ignored
```

`mise run test-platform` runs every ignored strata-platform test, this one included, without
the variable. It then plays nothing but reports `ok`, and prints
`SKIP p7::plays_a_tone_on_the_default_output: …` on stderr, past libtest's capture. Count it as
skipped, never as passed. The example `cargo run -p strata-platform --features media-audio
--example media` plays a tone and a video ramp in sync, for the by-ear check; with `--null` it
plays nothing.
