# V7 Rune QA backend

| | |
| --- | --- |
| Status | Not started |
| Depends on | V3, V6 |
| Unblocks | V8 |
| Owns | `crates/strata-app/src/qa/`, the target switch in `tests/qa.rs`, `scripts/qa-parity.sh`, `tests/qa/parity/` |
| Lands as | Feature commits on `strata-next` |

## Goal

`strata-rune --qa <directory>` speaks the QA protocol, and
`arrow-keys-move-focus/columns-arrows` passes on both targets from one command. That is
gate G1.

## Scope

### The backend

Implement V3's `Backend` over the offscreen host from V5, in `crates/strata-app/src/qa/`,
behind a `qa` feature that release builds never enable. `strata-rune` forwards the feature.
`strata-qa`'s rule in `deny.toml` gains `strata-app` as a second user.

The slice plan put the backend in `strata-rune`. It needs the `Runtime` and the app state,
which are `strata-app`'s, and `strata-rune` may depend on nothing else.

| Part of `Backend` | On Rune |
| --- | --- |
| The tree | `Runtime::accessibility_tree`. A node is a key path. `Kept` is the path with the node's id, so a recycled row is not taken for the one remembered |
| `role` | The role map below |
| `name`, `description`, `states`, `bounds` | The node's own. Bounds are window points; the window's origin is zero |
| `mark` | A pane, its entry container and its entries, classified by where the adapter put them: the column's group, its list, its rows |
| `focus` | The focused node's `activeDescendant` when it has one, else the focused node. V0 found focus stays on the strip and the row is its active descendant |
| `field` | The focused text field's text, caret and selection. Needed by V9; return `None` until then |
| `chord`, `key` | `Runtime::key` with the key map below, and `Runtime::physical_input` for held modifiers |
| `text_input` | Rune's text entry point. Find it in `runtime/input/` |
| `pointer_move`, `button`, `wheel` | `Runtime::pointer_move`, `pointer_down`, `pointer_up`, `wheel` |
| `settle` | Turn the host until GLib has nothing ready, the event queue is empty, and the runtime is not animating, after `Runtime::settle_all` |
| `sleep` | Turn the host until the time has passed |
| `resize` | `Runtime::set_viewport` |
| `criticals` | GLib's log handler, as `src/qa/criticals.rs` installs it. It uses GLib only: move it to a shared crate rather than copy it |
| `dump` | The same text shape as GTK's tree: one line per node with type, role, name, description, states and box |
| `command`, `check`, `capture` | `None`. The engine then replies that the step is not supported on this target |

Details V0 settled:

- **Settle with `settle_all`.** `Runtime::frame` returns "animating". After one key it
  stays true for 179 ms even with animations off, and under 1 ms with `settle_all`.
- **Run with animations off,** as the GTK runs do through `gtk-enable-animations=false` in
  `tests/fixtures/harness.json`: set the look's `animations` to false.
- **Reveal before aiming.** A virtual list mounts about 100 rows. A target that is not
  mounted is scrolled into view through the list's `scroll-into-view` action, and then
  located again.
- **Size.** The GTK window opens at 1200x760 inside the runner's 1440x900 screen. Use
  1200x760, so `window_width` means the same.

### The role map

A table with a unit test, from AccessKit's role to the AT-SPI name the scenarios use.

| Rune | AT-SPI name | Seen in |
| --- | --- | --- |
| `window` | `frame` | V0 |
| `group` | `grouping` | V0 |
| `scroll-view` | `scroll pane` | V0 |
| `list-box` | `list` | V0 |
| `list-box-option` | `list item` | V0 |
| `label` | `label` | V0 |
| To find | `button`, `toggle button`, `text`, `menu item`, `radio menu item` | The `role` targets in G2 and G3 |

GTK reports an unnamed container as `panel`. Rune's unnamed groups must not match a
scenario's `grouping`; check how the engine treats them when the map is written.

### The key map

A table with a unit test, from the GDK key names the scenarios use to Rune keys.

- **Named keys in the parity set:** `BackSpace`, `Delete`, `Down`, `End`, `Escape`, `F2`,
  `F5`, `F10`, `Home`, `KP_Left`, `Left`, `Menu`, `Page_Down`, `Return`, `Right`, `Tab`,
  `Up`, `space`, `period`.
- **Characters:** the digits `1` to `3`, `\`, and the letters `a c f h i j k l m n o q t v
  z`.
- **Modifiers:** `ctrl`, `shift`, `alt`, `altgr`.

Rune delivers the keypad's keys as the same logical keys as the main block's
(strata-components notes this for Page Up and Page Down). A scenario that depends on
`KP_Left` being distinct becomes `rune` pending.

### The process

- **Usage.** Keep the `usage: strata --qa` prefix in the usage text. `release.yml` fails a
  release binary that contains it, and `ci.yml` requires it in the QA build. With the same
  prefix, those two greps cover this binary as soon as they are pointed at it.
- **Streams, the ready reply, panics.** From V3's engine: stdout is kept for replies,
  application output goes to stderr, the first reply is `{"ready":true}`, and a panic
  replies and exits with code 70.
- **The snapshot.** The engine assembles the ten `app` keys from the tree: `view_mode`,
  `directory`, `panes`, `focused`, `focused_pane`, `dialog`, `menu`, `preview`, `field`
  and `matches`. In the slice `dialog`, `menu` and `preview` are always `null`.
- **`STRATA_INPUT_LOG`.** One line per synthetic event, in the format
  `.agents/skills/qa-scenarios/reference.md` gives: `key <keycode> down|up`,
  `motion <x> <y>`, `pointer <x> <y> button <n> down|up`, in window points. GTK writes X
  keycodes; Rune has none. Write the key's name and say so in the reference.
- **Queries.** `state` and `tree` come from the engine and `dump`. `prefs` reads
  `settings.toml` as the GTK backend does.

### The runner

- `STRATA_QA_TARGET=gtk|rune` picks the binary. `gtk` is the default and changes nothing.
- The GTK binary is `env!("CARGO_BIN_EXE_strata")` at `tests/qa.rs:106`, which covers only
  the root package. The Rune binary comes from an explicit path in an environment
  variable; the runner fails with a clear message when the target is `rune` and the path
  is unset or missing.
- Fixtures, HOME seeding, `fs` steps, peers and artifacts stay as they are. The runner
  still starts Xvfb for a Rune run; the binary ignores it.
- Runner steps keep working by themselves: `fs`, `restart`, `launch` and `proc` act on
  files and the process. `layout` swaps X keymaps, which Rune never sees: owner decision
  6.
- A runner test fails when a Columns run of a file in the parity set is missing from the
  ledger.

### The command and the ledger

`./scripts/qa-parity.sh slice` runs in the pinned container through
`scripts/lib/qa-container.sh`, and shares `target/qa-container` with `qa.sh`. It builds
both binaries, runs the ledger's `required` runs on each target, and exits 0 only when the
slice's exit check holds. `./scripts/qa-parity.sh g1` runs one gate.

`tests/qa/parity/slice.toml` lists every Columns run of every file in the set:

```toml
[[run]]
scenario = "arrow-keys-move-focus"
run = "columns-arrows"
gate = "G1"
status = "required"

[[run]]
scenario = "<scenario>"
run = "<run label>"
gate = "G2"
status = "pending"
reason = "harness"          # harness | component | rune | platform | toolkit | deviation
blocker = "open_menu"
```

In this package the ledger holds G1's run as
`required` and every G2 and G3 run as `pending` with reason `harness` and blocker "not yet
attempted". V8 and V9 replace those.

## Needs approval

- **Building the Rune side in the pinned image.** The image caches no Rune crate.
  `qa-parity.sh` either builds them into `target/qa-container` on first use, with network
  access, or the image's dependency stage grows. The second is an image change.
- **No CI job.** Parity runs locally in this slice.

## Out of scope

- G2 and G3. V8 and V9.
- Pixel capture, visual baselines, clipboard, drag and drop, the portal mode.
- A session host in headless sway.
- Rune mutation patches. The first arrives in V8.

## Tests

| Check | Command | Expect |
| --- | --- | --- |
| The maps | `cargo test -p strata-app --features qa` | Role and key tables pass |
| The backend, in process | The same | A scripted `select`, `key` and `wait` pass against a temporary directory |
| G1 on Rune | `./scripts/qa-parity.sh g1` | `columns-arrows` passes on both targets |
| The ledger | The runner's unit tests | No run of the set is unlisted |
| GTK unchanged | `./scripts/qa.sh` | 758 runs pass. The runner changed, so this is a full run |
| No GTK | `cargo tree -p strata-rune -i gtk4` | No match |

G1's scenario selects `readme.md`, waits for focus, presses Down, and waits for focus and
selection on `todo.txt`, then Up. It needs `select` (a click retried until the selection
takes), `key`, and the `focused` and `selected` predicates.

## Done when

`arrow-keys-move-focus/columns-arrows` passes on both targets from
`./scripts/qa-parity.sh`, and the handoff names the container run and the artifact
directory for each.

## Risks

- **A pass that proves little.** G1 is one run. The backend must fail when the app is
  wrong: before calling it done, break the adapter by hand once and watch G1 fail.
- **The engine assumed GTK somewhere.** V3 was checked against one backend. The first
  step that behaves oddly on Rune may be an engine assumption, and the fix belongs in V3's
  code with the GTK suite rerun.
- **Pointer geometry.** `select` clicks a point inside a row. Row heights differ between
  toolkits, so aim by the node's own box, never by a GTK constant.
