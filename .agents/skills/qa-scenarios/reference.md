# QA scenario reference

The full catalogue behind [SKILL.md](SKILL.md). The parsers are the source of
truth: the header is `Header` in `tests/qa/scenario.rs`, process steps are
`parse_command` in `src/qa/protocol/command/parse.rs` (also compiled into the runner, so a
malformed step fails at load), and runner steps are the `RUNNER_COMMANDS` in
`tests/qa.rs`. When this file and the code disagree, the code wins; fix this
file.

Every timeout defaults to 15 000 ms and every poll runs about every 50 ms.

## Contents

- [Header](#header)
- [Step keys every step takes](#step-keys-every-step-takes)
- [Variables and notes](#variables-and-notes)
- [Process steps](#process-steps)
- [Runner steps](#runner-steps)
- [fs steps](#fs-steps)
- [Targets](#targets)
- [Predicates](#predicates)
- [Protocol, replies, and the snapshot](#protocol-replies-and-the-snapshot)
- [Artifacts](#artifacts)

## Header

Line 1 of the file, one JSON object. Unknown keys are rejected.

| Key | Required | Meaning and validation |
| --- | --- | --- |
| `scenario` | yes | Equals the file stem. Lowercase words of `a-z0-9` joined by `-`; unique across all folders. |
| `modes` | yes | Views to run in: any of `columns`, `icons`, `list`. Each mode is one run. The runner writes `browser_mode` for the run's mode last, over every `prefs`. |
| `prefs` | no | Preference overrides written into `settings.toml` over the fixture manifest's defaults (`tests/fixtures/harness.json`, key `preferences`). An object value becomes a TOML table: `{"keymap":{"default":{"rename":["<Control>e"]}}}`. |
| `layout` | no | Keyboard layout switched to once the app is ready; default `us`. One of the manifest's `layouts`: `us`, `ru`, `ru-only`, `fr`, `de`, `caps-lock`, `cn-altgr-pinyin`. Key names always mean US key positions. |
| `variants` | no | `[{"id":..., "modes", "prefs", "layout", "vars", "tree", "setup", "teardown"}]`. Each variant reruns the file; its `modes` replace the header's, `prefs` and `layout` apply over the header's, `vars` become `$variables`, `tree` replaces the header's, `setup` runs after the header's and `teardown` after the header's. Run labels become `<mode>-<id>`. |
| `tree` | no | A manifest tree name (default `canonical`) or an inline tree: an object of directories (objects) and files (text). `{000..199}.txt` expands to one zero-padded file per number with `{}` in its text replaced by the number; a file's text may be a list of strings and `{"repeat":"r-{},{}\n","from":0,"to":999}`. Directories get mode 755, files 644, and every time is pinned to the manifest's `mtime`. |
| `images` | no | `{"beta/thumb.png":{"width":64,"height":64,"rgb":[230,40,60]}}`: solid-colour PNGs written into the tree after its times are pinned. |
| `gtk_settings` | no | `{"gtk-enable-animations":"true"}`: rewrites existing lines of the manifest's GTK `settings.ini`; a key not already there is an error. |
| `setup` | no | `fs` action steps (no checks, no `timeout_ms` or `message`) run after the tree is written, before launch. |
| `teardown` | no | `fs` action steps run once setup began, however the run ends; for example `chmod` back to a readable mode. |
| `env` | no | Variables added to the app's environment, `$variables` replaced. Never one the runner owns (`RUNNER_OWNED_ENVIRONMENT`, `XDG_DIRECTORIES`, and `QA_ENVIRONMENT` in `tests/qa.rs`): the display, buses, runtime, HOME and XDG directories, input log, shader cache, and accessibility switches. |
| `session_bus` | no | `true` starts a private session bus with no service directories, shared by the app and the `dbus_send`, `launch`, and `foreign` steps (all three require it). Without it the app gets a bus address that cannot connect. |
| `portal` | no | `true` (requires `session_bus`) starts `strata --qa --portal`, the FileChooser portal backend, instead of the browser. There is no window until `portal_open`; `adopt` then drives the chooser. Until a chooser opens, the snapshot is empty. |
| `input_method` | no | An fcitx5 input method name such as `chewing` or `pinyin` (requires `session_bus`): starts a private fcitx5 on the run's display and bus whose group holds the US keyboard and that method; Ctrl+Space switches to it. Pick the toolkit path with `env` (`GTK_IM_MODULE`). Host runs only, since the pinned image has no fcitx5; give such a scenario a `skip` reason. See "Wayland and input methods" in `docs/testing.md`. |
| `fixture_root` | no | `/tmp/<name>`: builds the tree at that fixed path instead of a fresh one, for a screenshot where the path renders. The path must not exist (never reused); runs with a `fixture_root` never overlap one another. |
| `mutation` | no | The name of a patch in `tests/qa/mutations/` (without `.patch`) that must make the scenario fail; each patch belongs to one scenario. `./scripts/qa-mutation.sh` checks it. Not combinable with `skip`. |
| `known_bug` | no | An issue reference. A failure becomes the expected result. Needs `known_bug_step`. |
| `known_bug_step` | with `known_bug` | The file line (header = 1) of the step expected to fail. It must run in every run, carry no `when`/`if`/`unless`, and not be a `layout`, `flag`, `note`, `slice`, `increment`, `stem`, `parent`, `screenshot`, `restart`, `http`, or `foreign_stop` step. A failure anywhere else, or any error, fails the run; a pass fails with "remove the marker". |
| `skip` | no | A non-empty reason. Keeps the scenario out of the default run (`SKIPPED <scenario> (run it by name): <reason>`); naming it still runs it. Not combinable with `mutation` or `known_bug`. |
| `cost` | no | `{"<run label>": number}` for exactly every run label, the run's median time over a typical run's, as `scripts/qa_cost.py` writes it (one decimal, at least 0.1); the runner requires a positive finite number. |
| `quality` | no | `{"impact":1-5,"coverage":1-5,"criticality":1-5,"diagnostic":1-5,"score":0-100,"note":"<one line>"}`. All six keys required and no other; the ratings and `score` are integers; `note` is one non-empty line with no line break of any kind (including U+0085, U+2028, U+2029, and the vertical tab, form feed, and U+001C–U+001E separators). The runner recomputes `score` from the four ratings (`quality_score` in `tests/qa/scenario.rs`) and rejects a mismatch, naming the expected value. The `qa-quality` skill defines the ratings. |

Leave header keys this reference does not list as they are.

### Runs and labels

A scenario without `variants` has one run per mode, labelled by the mode
(`columns`). With variants, each variant runs in its own modes (else the
header's), labelled `<mode>-<id>` (`columns-ru`). Labels must be distinct.
Each run is a test named `<scenario>/<label>`.

## Step keys every step takes

Every line after the header is one JSON object with exactly one command key
plus that command's options. These keys work on any step; the runner strips
the first five before the step runs:

| Key | Meaning |
| --- | --- |
| `"modes":["icons","list"]` | Runs only in those modes. |
| `"variants":["ru"]` | Runs only in those header variants; each id must exist. |
| `"when":PREDICATE` | Runs only if the predicate holds just before it (asked with `holds`). |
| `"if":"name"` / `"if":["a","b"]` | Runs only if every named flag held. Flags come from an earlier `expect` or `flag` with `"as"`, or a `log` `contains`. |
| `"unless":"name"` / `["a","b"]` | Skipped if any named flag held. A skipped `expect`/`flag` that keeps a flag keeps it false. |
| `"message":"..."` | What the step waits for or asserts, in words; a timed-out `wait` fails as `timed out after <ms> ms: <message> expected <predicate>, actual <value>`, a failed `expect` as `<message> expected ..., actual ...`, and a timed-out `fs` check or target lookup as `timed out waiting for <message>: ...`. |
| `"timeout_ms":N` | Overrides the 15 s default where the command waits. `0` on an `fs` check is a plain assertion. |

## Variables and notes

`$name` in any string is replaced as a whole token (`$view` leaves `$viewport`
alone); `$$` is a literal `$`. An unknown `$name` fails the scenario before it
launches.

| Variable | Value |
| --- | --- |
| `$root` | The fixture directory's name, which is also its pane's name. |
| `$fixture` | The fixture directory's absolute path. |
| `$fixture_quoted` | That path percent-escaped as a `file://` URI path (without the scheme), for a `.trashinfo` `Path=` or a typed URI. |
| `$home`, `$data`, `$config`, `$cache` | The run's HOME and XDG data, config, and cache directories. The app's settings are `$config/strata/settings.toml`; its trash is `$data/Trash`. |
| `$environment` | The directory holding HOME and the XDG directories. |
| `$python` | `python3`, the pinned image's system interpreter. |
| `$view` | `Columns`, `Icons`, or `List`, as `view_mode` names the run's view. |
| `$mode` | `columns`, `icons`, or `list`. |
| variant `vars` | Each `$key` of the run's variant. |

A note keeps a value read during the run under `"as"`; later steps use it as
`$name`. Where a string is exactly `$name`, the value replaces it as is (a
list stays a list); inside other text, a list shows as `['a', 'b']`. A note
may only be used after the step that keeps it and may not hide a variable.

## Process steps

These run inside `strata --qa` on the GTK main loop; each reply comes after
the main context drains. Input is real XTEST.

### Input

| Step | Meaning |
| --- | --- |
| `{"key":"ctrl+x","repeat":2}` | Taps a chord (`ctrl`, `shift`, `alt`, `altgr` + a GDK key name such as `Return`, `F2`, `period`, `Page_Down`); the key comes up 20 ms after it went down. |
| `{"type":"nested-notes"}` | One key tap per character. |
| `{"key_down":"ctrl"}`, `{"key_up":"Escape"}` | One modifier or key down or up, no tap, no pause. |
| `{"click":TARGET,"button":1,"count":1,"mods":["shift"],"at":[4,"50%"]}` | Moves to the target and clicks. `button` 3 is a right-click, `count` 2 a double-click. `"release_mods_first":true` lifts the modifiers between press and release. `"left_bound":"browser"` keeps the aim right of a remembered box's left edge. |
| `{"select":{"entry":"readme.md","dir":"$root"}}` | Clicks the entry, and again after 4 s if it is still not selected. |
| `{"select_with_keyboard":"todo.txt","message":"..."}` | Home, then Down (Right in Icons) until the entry has focus, at most 40 steps. |
| `{"open_menu":TARGET,"until":PREDICATE,"message":"..."}` | Clicks, and clicks again each second while the target is not `checked` and the predicate does not hold. `"open_states":["checked","pressed"]` counts either as open. |
| `{"move":TARGET,"at":[...]}`, `{"move_by":[2,0]}` | Moves the pointer to a target, or by an offset from where it is. |
| `{"wheel":TARGET,"clicks":3,"direction":"down","at":[...]}` | Wheel clicks over the target, `down` (default) or `up`, with optional `mods`. |
| `{"wheel":TARGET,"clicks":10,"until":PREDICATE,"tries":40}` | Turns only while the predicate does not hold, checking before each turn, at most `tries` turns; or, with `message` and `timeout_ms` instead of `tries`, polls until it holds and fails on timeout. |
| `{"drag":{"from":TARGET,"to":TARGET},"steps":16,"release":true,"mods":[]}` | Presses on `from` (its first rendered image, else its centre; point-like targets aim where they always do), moves in `steps`, releases unless `"release":false`. |
| `{"drag":{"from":T,"from_at":[...],"to":T,"to_at":[...]},"after_press":PREDICATE,"message":"..."}` | Checks the predicate once after the press; a mismatch lets go and fails. `after_press` may instead list `{"wait":...,"message":...}` and `{"expect":...,"message":...}` steps run in order. |
| `{"button_down":1}`, `{"button_up":1}` | One button where the pointer is; `button_up` also ends an abandoned drag. |
| `{"keys_until":PREDICATE,"keys":["Home","Left"],"times":2}` | Presses the keys in turn for at most `times` rounds, stopping once the predicate holds after a round. `"check_first":true` checks before each round; `times` may be a noted list (its length); `"await_focus_change":{"timeout_ms":5000,"message":"..."}` waits for focus to move after each round; `"exhausted":"..."` fails with that text if no round satisfied it. |
| `{"click_result_rows":{"selected":true},"mods":["ctrl"]}` | Clicks, in tree order, every search result row (else every list item) whose states match when its turn comes. |
| `{"set_scrollbar":"horizontal","value":0}` | Sets the first scroll bar of that orientation; no pointer input. |
| `{"resize":{"width":420,"height":300}}` | Resizes the main window; without `height`, keeps its height. |
| `{"activate":TARGET,"message":"..."}` | Runs the widget's default action without pointer input. A role target without `within` may be scrolled away; one with `within`, or a `dialog_button`, is looked up on screen first. |
| `{"reveal":{"role":"button","name":"Modified date format"}}` | Turns the wheel three clicks at a time at the window's centre, at most twelve times, until the control is on screen. |
| `{"reveal_control":{"role":"spin button","name":"Text size in pixels"}}` | Scrolls a settings page's own pane until the control is wholly shown, then settles it. |
| `{"adopt":"PNG conversion test","timeout_ms":20000,"message":"..."}` | Portal runs: waits for a showing window with this title and drives it from then on (snapshot, targets, input log). |
| `{"clipboard_set":"text"}`, `{"clipboard":"999","message":"..."}` | Owns the clipboard with the text from a second display connection; reads the clipboard and fails unless it matches, exactly or as `{"starts_with":...,"contains":...,"longer_than":5}`. |
| `{"sleep":600}` | Waits; the main loop keeps running. Never use it to wait for state. |

A click holds the button 100 ms; a double click's two clicks are 90 ms apart,
inside GTK's 400 ms double-click time, and a drag after a selecting click
waits 600 ms so it is no double click. The pointer approaches a point in two
steps, 6 pixels off and then on it, so GTK sees enter and motion rather than a
jump. The X server times a tapped key's release (XTEST's delay), never a
frame wait: the server autorepeats a key held past 660 ms, and a busy app can
hold a frame that long.

### Waits and checks

| Step | Meaning |
| --- | --- |
| `{"wait":PREDICATE,"message":"...","timeout_ms":15000}` | Polls until the predicate holds; on timeout the run stops with the widget tree. `"samples":10` needs that many polls in a row; `"settling":TARGET` settles the target before every poll. |
| `{"wait":PREDICATE,"never":PREDICATE,"never_message":"..."}` | As `wait`, but fails if `never` held at any poll, the last included. |
| `{"expect":PREDICATE or [PREDICATE,...],"message":"..."}` | Checks now, records mismatches, and continues. With `timeout_ms` it polls until all hold first; with `hold_ms` too they must hold that long in a row. `"as":"name"` keeps whether it held, as a flag. |
| `{"during":PREDICATE,"ms":1200,"every_ms":50,"message":"..."}` | Fails as soon as the predicate stops holding within `ms`. `"during":[{"check":P,"message":"..."},...]` checks several in order. |
| `{"settle":TARGET,"timeout_ms":15000,"as":"entered"}` | Waits until the target's box repeats for three polls and keeps it (as the last settle, and under `as`), for `moved` and `kept`. |
| `{"holds":PREDICATE}` | Replies `true` or `false` in `result`; never fails. |
| `{"query":"state"}`, `"tree"`, `"prefs"` | The snapshot, the widget tree as text, or `settings.toml` as JSON in `result`. |

### Memory, notes, and pixels

| Step | Meaning |
| --- | --- |
| `{"remember":{"field":true},"as":"filter"}` | Keeps the focused field, for `{"field":"filter"}` targets and `field` checks. Any other target keeps its box (for `left_bound`, `moved`, `kept`) and its widget (for `replaced`, `alive`, `remembered`). |
| `{"keep":[0,0,240,80],"as":"foreign"}` | Keeps a screen box for `kept` targets. |
| `{"note":"entries","dir":"$root","as":"names"}` | Keeps an accessor's value. Accessors: `directory`, `pane`, `panes`, `entries`, `selected`, `all_selected`, `matches`, `focused`, `focused_pane`, `visible`, `menu_items`, `editable_fields`, `visible_top`, `middle_visible` (`unless_first`), `bounds` and `scroll_target` (with `"of":TARGET`), `number` (with `"expr":NUMBER`). |
| note shaping | `from`/`through` slice a list between two names; `"middle":3` keeps three from the middle; `"plus":[...]` unions and sorts; `"sorted":true`; `"pick":-1` keeps one item. `"until_found":true` (with `message`, `timeout_ms`) polls until non-null; `"until_names":NAMES` polls until the list matches; a `number` note takes `"until":[{"op":">","than":300}]`. |
| `{"capture":TARGET,"inset":3,"as":"before"}` | Keeps the screen pixels in the target's box, inset on every side, for `repainted`; any part off the screen is black. |
| `{"measure_text":"label","as":"resting"}` | Ink position (the luma-weighted mean row) and contrast in a kept box, for `contrast_below` and `drop_samples`. Fails as `the source label disappeared` when the box holds no ink. |
| `{"drop_samples":"label","resting":"resting","dragging":"dragging","escape":true,"as":"released"}` | Ends a drag (optional Escape, then button 1 up) and samples the kept box at 60, 120, and 180 ms; each sample must come within 240 ms and sit within 1.5 px of `resting`, and the last must have 1.25 times `dragging`'s contrast. Keeps the release instant for `elapsed`. |

## Runner steps

These run in the runner (`drive` in `tests/qa/drive.rs`), between process commands.

### Control and values

| Step | Meaning |
| --- | --- |
| `{"restart":true}` | SIGTERM to the app's process group, 5 s, then SIGKILL; starts it again with the same environment, fixture, display, and input log, with a fresh app log. It needs the US layout active (it reads the keymap again) and fails otherwise: after a `layout` step, add `{"layout":"us"}` before it. |
| `{"restart":{"directory":"$fixture/target","env":{"RUST_LOG":"trace"},"stdout":"closed","stderr":"null"}}` | Restarts on another directory with extra variables, and standard output or error as a closed pipe or `/dev/null`. |
| `{"layout":"ru"}` | Switches the run's keyboard layout. |
| `{"flag":PREDICATE,"as":"enabled"}` | Keeps whether the predicate holds now; never fails. With `timeout_ms`, polls until it holds or time runs out. |
| `{"each":"names","as":"name","derive":{"caption":{"number_plus":320,"suffix":"×180"}},"step":STEP}` | Runs a process step once per item of a noted list, `$name` the item; each `derive` variable is the last number in the item's stem plus `number_plus`, between optional `prefix` and `suffix`. |
| `{"slice":"rows","middle":3,"pick":-1,"as":"last"}` | Part of a noted list. |
| `{"increment":"first","as":"next"}` | A noted name with its trailing number one higher, keeping its width. |
| `{"stem":"source","as":"stem"}`, `{"parent":"output","as":"dir"}` | A noted name's stem; a noted path's parent. |
| `{"fail":"the dialog never accepted the name"}` | Fails with that message; ends an unrolled retry. |
| `{"log":{"count":"browser sandbox started"},"as":"initial"}` | Reads the app log (colours stripped): `count` of a text, `matches` (numbers a pattern's first group finds; `pick` keeps one), or `contains` (kept as a flag). With `as` it keeps the value; without, it waits until the count is `equals`, `at_least`, or `at_most` a number or a noted list's length, plus `plus`. |
| `{"proc":{"gone":"$pids"}}`, `{"proc":{"parent_is_app":"$pid"},"timeout_ms":0}`, `{"proc":{"kill":"$pid"}}`, `{"proc":{"app_cwd":"/"}}` | Waits until noted pids exit; checks a pid's parent is the app; SIGKILLs an app child (only); waits until the app's working folder is the path. |
| `{"launch":["$fixture/requested-missing"],"percent_path":true}` | Runs another `strata` with arguments in the fixture directory (needs `session_bus`, 30 s limit); it hands them to the running app over the bus and exits. `percent_path` turns `%XX` into raw bytes. |
| `{"dbus_send":["--print-reply","--dest=...","/path","iface.Method","array:string:file://..."]}` | `dbus-send --session` on the private bus (needs `session_bus`); fails on a non-zero exit. |
| `{"screenshot":"hover.png"}` | Captures the X root into the run's artifacts. |
| `{"baseline":"columns-view"}` | Captures the X root and compares it with the committed image for the GTK version the runner links (`baseline_directory()` in `tests/qa/baseline.rs`): a pixel differs when a channel moves by more than 24; more than 0.5 % differing pixels, a size change, or a missing baseline fails. Artifacts keep `<name>.actual.png`, plus `.expected.png` and `.diff.png` on a mismatch. Writes the capture as the baseline only under `STRATA_QA_UPDATE_BASELINES=1 ./scripts/qa.sh <scenario>`; takes no `message`. |

### Peers

| Step | Meaning |
| --- | --- |
| `{"portal_open":"Title","options":{"current_folder":"$fixture","multiple":true,"filters":[["PNG images",[[1,"image/png"]]]]}}` | Portal runs: waits up to 20 s for the backend's bus name, then sends `OpenFile` without waiting for the answer. A request still open when the run ends is closed once the final state is read. |
| `{"portal_response":{"done":false},"message":"..."}` | Asserts the request has not answered yet. |
| `{"portal_response":{"done":true,"status":0,"names":[...],"paths":[...],"keep":"output","png":{...},"discard":true},"timeout_ms":20000,"message":"..."}` | Waits for the answer and checks its status, chosen names (a set) or paths (in order); `keep` notes the first path, `png` checks it as the `fs` `png` check does, `discard` removes its download directory. |
| `{"http":"<base64>","as":"url"}`, `{"http_requests":1,"message":"..."}` | Serves the bytes as `image/png` to every GET on 127.0.0.1 until the final state is read, its URL noted; asserts how many requests came. |
| `{"foreign":{"paths":["$fixture/a"],"move_only":true},"as":"foreign","message":"..."}` | Starts a separate GTK drag source offering the paths (needs `session_bus`) and keeps its button's box as a `kept` target. |
| `{"foreign_label":"Foreign drag finished","message":"..."}`, `{"foreign_stop":true}` | Waits for the drag source to show a label; stops it. |
| `{"zipcrypto":{"path":"password.zip","password":"zipsecret","member":"some.txt","contents":"...","deflated":true},"as":"collision"}` | Writes a ZipCrypto archive with a random header and notes the first numeric password below 4096 that also passes the header check. `contents` may be `{"repeat":"text","count":64}`. |

## fs steps

`{"fs":{"path":..., <one kind>...}, "timeout_ms":..., "message":...}`. A
relative path is under the fixture; an absolute one (`$data/Trash/files`)
stands alone. Checks poll until they hold (`timeout_ms` 0 asserts once);
actions act at once. Actions that remove, rename, or generate refuse a path
whose resolved parent lies outside the fixture, `$environment`, and the
run's scratch directory.

### Checks

| Kind | Holds when |
| --- | --- |
| `"exists":true` | The path exists (a final symlink is not followed). |
| `"is":"file"` / `"dir"` / `"symlink"` | It is that kind. |
| `"text":"..."` | The file holds exactly that text. |
| `"names":[...]` | The directory's names are exactly these. |
| `"empty":true` | The directory is empty or missing. |
| `"unchanged":"original"` | The listing equals one kept with `remember`. |
| `"names_from":"original","minus":[...],"plus":[...]` | The names are a kept set less `minus` plus `plus`. |
| `"names_without":"clicked"` | No name contains the part. |
| `"glob_none":".strata-compression-*"` | No name matches (`*` is any run). |
| `"copy_of":"documents/report.md"`, `"size_of":...` | The bytes, or the size, equal another file's. |
| `"bytes_of":"original"` | The bytes equal ones kept with `keep_bytes`; a read error fails at once. |
| `"has":{"contains":[...],"lacks":[...]}` | The text contains every part and none of the others; an unreadable file contains nothing. |
| `"lines":N` or `{"at_least":N}` | Line count, exactly or at least; a missing file has none, and an unreadable or non-UTF-8 one fails at once. |
| `"lines_equal":[...]`, `"last_line":"..."` | All lines, or the last one. |
| `"size":N`, `"executable":true`, `"mtime":N` or `{"local":[Y,M,D,h,m,s]}` | Size; all execute bits set; whole-second modification time, a `local` time read in the runner's time zone. |
| `"pixel":{"at":[x,y],"rgb":[r,g,b]}` | An image's pixel. |
| `"png":{"size":[w,h],"at":[x,y],"min":[...],"max":[...]}` | A PNG of that size whose pixel lies within bounds. |
| `"is_zip":true`, `"zip_members":[[name,fixture file],...]`, `"zip_member":[name,fixture file]`, `"tar_members":[...]` | Archive shape and member bytes. |
| `"toml":{"a.b":value}`, `"toml_prefix":{"key":[...]}`, `"toml_like":"kept","toml_keys":[...]` | Values at dotted TOML keys; a list's prefix; equality with kept TOML bytes (optionally only some keys). |
| `"json":{"key":value}`, `"json_set":{"key":[...]}` | Values at top-level JSON keys; lists compared as sets. |
| `"default_app":{"for":"todo.txt","is":"x.desktop"}` | In a `mimeapps.list`, the first default for the file's content type. |
| `"opened":["$fixture/todo.txt"]` | A stub launcher's record lists exactly as many paths and each of them. |
| `"launches":{"count":3,"within":"$fixture"}`, `{"first_dir":...}` | A stub terminal launcher's `cwd<TAB>--dir=...` lines. |
| `"can_trash":false` | GIO's `access::can-trash`. |
| `"alpha_range":[0,255]` | An image's lowest and highest alpha, so `[0,255]` holds both a transparent and an opaque pixel. |
| `"cached_in":"$cache/thumbnails/large"` + `exists`/`is`/`pixel`/`alpha_range`/`keep_mtime`/`mtime_of` | Checks the path's cached freedesktop thumbnail instead. |
| `"all_cached_in":"...","except_suffix":".stl"` | Every entry but those has a cached thumbnail. |
| `"keep_mtime":"stored"` then `"mtime_of":"stored"` | Keeps the nanosecond mtime; waits until it is that again. |

### Actions

| Kind | Does |
| --- | --- |
| `"write":"text"`, `"append":"text"` | Writes or appends. A range in the last component (`full/{000..179}.txt`) writes one file per number, `{}` in the text standing for it. |
| `"mkdir":true` (`"parents":true`) | Makes a directory, once per number of a range. |
| `"populate":TREE` | Writes a tree (header-tree shape) and pins its times. |
| `"chmod":"555"` | Sets the mode. |
| `"remove":true`, `"rename":"archive/documents"` | Deletes (recursively for a directory); renames within the fixture. |
| `"symlink":"missing"`, `"program":"true"` | A symlink; a copy of a program from `PATH` with its mode and times. |
| `"remember":"original"`, `"remember_names":"original"`, `"keep_bytes":"original"` | Keeps a listing, a name set, or the bytes. |
| `"volume":"/dev/shm"` (`"prefix"`, `"with":{...}`), `"remove_volume":true` | Links a fresh directory at the path, which must lie on another device than the fixture (removed when the run ends; `with` writes files into it first); removes it early, leaving the link dangling. |
| `"image":{"width":...,"height":...,"rgb":[...]}` | A solid PNG in one write, so a listing never sees it grow; a range in the name writes several, `width_step` wider each, each followed by its `thumbnail` if given. |
| `"thumbnail":{"cache":"$cache/thumbnails/large","width":...,"height":...,"rgb":[...],"mode":"700"}` | A freedesktop cached thumbnail for the file: named after the MD5 of its URI, with `Thumb::URI` and `Thumb::MTime`; `mode` sets the cache directory's. |
| `"hex":"0001..."`, `"base64":"..."`, `"random":N`, `"truncate":N` | Bytes in one write; N random bytes; a sparse file of size N. |
| `"raw_photo":true`, `"pdf_image":{...}`, `"magick":[...]`, `"ffmpeg":[...]`, `"from_repository":"tests/fixtures/documents/text.pdf"` | Generated media: a raw photo, a one-page PDF of a solid image, ImageMagick or `ffmpeg` output at the path, or a repository file's bytes. |
| `"tar_gz":{"mode":"644","mtime":0,"members":[[name,text or {"repeat":..,"count":..}]],"flip_from_end":8}` | A gzipped tar; `flip_from_end` corrupts the trailer. |
| `"zip":[[name,text or {"text":..,"date_time":[...],"mode":"100755"} or {"image":{...}}],...]` | A stored zip of the entries in order. |
| `"replace_lines":{"suffix":".trashinfo","prefix":"DeletionDate=","with":"..."}` | Rewrites matching lines of every matching file in a directory. |
| `"percent_path":true` | With any kind: `%XX` in the path stands for a raw byte, for non-UTF-8 names. |

## Targets

A target names a widget the way a person sees it, never a screen point. It is
resolved from the widget's bounds in its native surface plus that surface's X
origin (which covers popovers), looked up for up to 15 s, and the pointer aims
only once its box comes through a painted frame unchanged: a row a listing has
just added or replaced can report a box before its first allocation lands. A
lookup that breaks, such as an unrealized surface, is an error, never
`absent`.

| Target | Finds |
| --- | --- |
| `{"entry":"todo.txt","dir":"archive"}` | An entry row; any pane without `dir`. `"part":"label"`, `"icon"`, or `"metadata"` picks its name label, image, or first other label. |
| `{"row":"readme.md"}` | The first rendered list item with the name, else the first table cell (chooser rows). |
| `{"pane":"archive"}` | A pane; aims at an empty point unless `at` is given. |
| `{"background":true,"dir":...}` | An empty point in a pane (a point; takes no `at`). |
| `{"container":"archive"}` | The list or grid inside a pane. |
| `{"viewport":null}` | The scroll viewport around a pane's entry list. |
| `{"role":"button","name":"Settings"}` | By role and accessible name. Options: `"nth":-1`, `"name_ends_with":["/a"]`, `"name_matches":"^\\d+$"` (regex), `"description":...`, `"states":["selected"]` (also `focused`, `checked`, `pressed`, `sensitive`, `selectable`, `editable`, `busy`, `horizontal`, `vertical`), `"within":TARGET`, `"has":{role target}` (a match containing that widget), `"rendered":false` (presented but maybe scrolled away, which `state`, `description`, `activate`, and `remember` read as it is). Without `name` any name matches. `role` may be a list, or `*` for any. |
| `{"named":"Apply to All","within":{"dialog":true}}` | Any role by name, inside another target. |
| `{"menu_item":"New File"}` | A menu item, in the open menu if there is one. |
| `{"sidebar":"Home"}`, `{"header":"Choose sort field"}` | A sidebar place; a column header button. |
| `{"dialog":true}`, `{"dialog_button":"Close"}` | The modal dialog; a button in it. |
| `{"field":true}`, `{"field":"filter"}`, `{"field":true,"within":TARGET}` | The focused editable text, the one remembered as `filter`, or the editable text inside another target. |
| `{"focused_widget":true}`, `{"remembered":"focused"}` | The focused widget; a widget `remember` kept. |
| `{"preview":true}` | The quick preview drawer. |
| `{"search_result":"todo.txt"}`, `{"result":"beta/match-note.txt"}`, `{"result_row":-1,"containing":"match-note"}` | Search result rows by name, by path, or by index among those whose name contains a part (or is `"name"`). |
| `{"window":true}`, `{"window":true,"y_of":TARGET}` | The main window; with `y_of`, a point near its right edge at that target's centre height. |
| `{"toplevel":"Copy to"}`, `{"other_window":"Strata"}` | Another application window by title (a floating chooser; a second browser window). A title also matches windows titled `"<folder> — <title>"`, so `"Strata"` finds every browser window and `"docs — Strata"` only the one showing `docs`. |
| `{"parent":TARGET}`, `{"ancestor":"scroll pane","of":TARGET}` | A widget's parent; its nearest ancestor with the role. |
| `{"kept":"edge"}` | A box or point `settle`, `remember`, or `keep` kept. |
| `{"point":[X,Y]}` | A point from target coordinates, in whole pixels: each is `{"of":TARGET,"at":OFFSET}` (its centre without `at`), `{"mid":[A,B]}`, `{"max":[A,B]}`, or `{"extent":TARGET}` (its width or height taken as a screen coordinate), each with optional `"plus":N`. |

`at` is `[x, y]` from the target's top-left: pixels (negative from the far
edge), a share (`"80%"`, `"2/3"`), a share plus pixels (`"2/3+3"`),
`"start-6"` or `"end+60"` (pixels from an edge, may lie outside), or
`"end-1/6"`. Without `at` the pointer aims at the centre. Shares round down
and have no lower bound, so `"1/6"` of a row under 6 pixels tall is 0.

### Accessible names

Targets resolve through the names `src/ui/accessibility.rs` sets for screen
readers:

- An entry row is labelled with its name and described as `Folder` or `File`.
- A pane is labelled with its directory and described with its presentation;
  an empty, unreadable, or loading pane's focusable surface is labelled with
  its directory and described with its status text.
- Menu items carry the menu role with the accelerator in the description.
- Modal dialogs carry the dialog role and a name.
- A button with a text label is named by that label, through the
  `labelled-by` relation GTK gives it, before any label set on the button,
  unless `accessibility::clear_labelled_by` dropped the relation or
  `set_labelled_by` replaced it. A menu button's inner toggle takes the menu
  button's name and description.
- An entry row's name is its list item's; `set_labelled_by` joins its labels'
  names. A list view row is `selected` as its selected state flag says.
- Tooltips never name a target. An icon-only button gets
  `accessibility::set_label` with its tooltip text.

Role names are AT-SPI's: `button` (not `push button`), `panel` for a plain
box, `grouping`, `slider`, `password text`, `toggle button`, `check box`,
`list item`, `menu item`, `check menu item`, `scroll pane`, `label`, `text`,
`image`, `frame`. The `tree` query prints each widget's role, name, and
description: copy targets from it. When a control has no usable name, name
it in the app; never reach around the accessibility layer.

## Predicates

A predicate is an object with exactly one check, plus an optional `dir` (a
pane name) where the check reads a pane.

### Browser state

| Predicate | Holds when |
| --- | --- |
| `{"directory":"archive"}`, `{"pane":"archive"}`, `{"panes":NAMES}` | The focused pane's directory, else the deepest pane's; the deepest pane; the panes left to right. |
| `{"entries":NAMES,"dir":...}`, `{"selected":NAMES,"dir":...}` | A pane's entry names, or selected names, in visual order. |
| `{"all_selected":NAMES}` | Every pane's selection, left to right. |
| `{"entry":"a.txt","dir":...}`, `{"entry_gone":"a.txt"}` | The entry is listed, or no longer. |
| `{"focused":"a.txt"}` | The focused entry's name; `true` for any, `null` for none; `{"one_of":[...]}`. |
| `{"focused_pane":true}`, `{"focused_in_matches":true}` | A pane holds focus; the focused name is among the matches. |
| `{"matches":NAMES,"dir":...}` | The search results, else the pane's entries. |
| `{"visible":NAMES,"dir":...}`, `{"visible_top":NAMES}` | Rows wholly inside the pane's viewport; rows a `visible_top` note listed. |
| `{"scroll_value":{"at_least":N},"dir":...}` | The pane's first showing scroll bar's value. |
| `{"view_mode":"Icons"}`, `{"window_width":420}` | The first pane's presentation; the window's width. |
| `{"dialog":"Unable to rename item"}` | The dialog's name; `true` for any, `null` for none. |
| `{"menu_items":NAMES}`, `{"menu_items":{}}`, `{"menu_items":null}` | The open menu's items; any open menu; no menu. |
| `{"preview":"text"}`, `{"preview":true}`, `{"preview":null}`, `{"peek":true}` | The preview drawer shows the text; is open; is closed. A folder peek is open. |
| `{"pref":{"key":"show_hidden","value":true}}` | `settings.toml` on disk: `value` (null when absent), `"contains":"size"`, or `"one_of":[...]` (exactly one of the three). |
| `{"field":{"of":"rename","name":"Rename","text":"a","caret":4,"selection":[0,4],"focused":true,"showing":true}}` | A field (remembered as `of`, else the first on screen with `name`, else the focused one). `text` exactly or `{"starts_with"|"ends_with"|"contains"|"lowercase_contains":...}`; `caret` exactly or `{"min":1,"max":72}`; `selection` `null` for none. |
| `{"editable_fields":1}`, `{"editable_text":"todo"}` | How many editable text fields are on screen; one holds the text. |
| `{"file":{"path":"$fixture/x","exists":false}}`, `{"file":{"path":...,"mode_bits":{"mask":"100","set":true}}}` | File state read by the process, so `any`/`all` can join it with window state. |

### Widgets

| Predicate | Holds when |
| --- | --- |
| `{"present":TARGET}`, `{"absent":TARGET}` | Found on screen, or not found or not on screen. |
| `{"presented":TARGET}` | Found, even scrolled away. |
| `{"on_screen":TARGET}` | Within the window's height. |
| `{"state":{"target":T,"selected":true,"focused":false,"checked":false,"pressed":true,"sensitive":true}}` | Each given state matches; also `selectable`, `editable`, `focusable`, `busy` (what `accessibility::set_busy` recorded), and `marked` (an entry row outlined as the result Enter in its filter field opens). An active switch is `pressed` and `checked`; a tab's `selected` is what `accessibility::set_selected` recorded. |
| `{"name":{"target":T,"text":"0:05"}}`, `{"text":{"target":T,"text":"..."}}`, `{"description":{"target":T,"text":"Ctrl+C"}}` | Accessible name, text content (a label's or entry's text, a text view's buffer), or description (the recorded one, or a picture's alternative text), exactly or as a pattern (`{"contains":...}`). |
| `{"mentions":{"target":T,"text":"..."}}` | Some name or description in the subtree contains the text; `"role":"label"` limits it; `"words":"a b"` allows any whitespace; `"equals"` matches a whole collapsed name. |
| `{"subtree_text":{"target":T,"contains":"...","strip_newlines":true}}` | The subtree's joined text contains the part. |
| `{"child_names":{"of":T,"role":"button","names":NAMES}}` | A target's visible children with the role. |
| `{"list_sections":{"within":T,"section":"Recommended Applications","names":NAMES}}` | The selectable items under a heading row. |
| `{"focused_widget":"Custom color…"}` | The focused widget's name, exactly or `{"contains":...}`; `null` for none. |
| `{"focused_widget_is":T}`, `{"focus_within":T}`, `{"focus_outside":T}` | Focus is on the target; inside it; on a named widget outside it. |
| `{"any_result_row":{"focused":true}}` | Some search result row has the state. |
| `{"panes_with":{"target":{role target},"names":[...]}}` | The panes, left to right, showing that role target. |
| `{"alive":"rename"}`, `{"replaced":{"target":T,"since":"listed"}}` | A remembered widget still belongs to a window; the target now resolves to another widget than the one remembered (a reload rebuilt the row). |
| `{"some_window":{"title":"Strata","holds":PREDICATE}}`, `{"windows":{"title":"Strata","count":6}}` | Some showing window with the title satisfies the predicate in its own tree; how many such windows show. |
| `{"diagnostics_mention":["could not","permission denied"]}` | Any of the texts appears in the app windows' names and descriptions (lowercased). |
| `{"children":{"comm_prefix":"xdg-terminal-ex","count":0}}` | How many app child processes have a command name with the prefix. |
| `{"hit_path":{"mark":"search: txt","hits":{...},"name":"todo.txt"}}`, `{"footer_fill":{"ends":[...],"total":180}}` | Footer search hit and selection-count readouts: `footer_fill` holds when both ends are listed, cleared, and on screen, and a label is described as `<total - cleared> of <total> items selected`. |

### Geometry and pixels

Use these only where position or pixels are the behaviour (a scroll target
reached, a drag landing, a repaint, a visual baseline); never to pin layout.

| Predicate | Holds when |
| --- | --- |
| `{"moved":{"target":T,"direction":"up","since":"entered"}}` | Moved since the last `settle` (or a named one): `up`, `down`, `left`, `right`, or `none`. With `none`, `"within":1` allows that much drift on both axes and keeps the size; `"y_within":2` allows that much vertical drift and ignores the rest. |
| `{"resized":{"target":T,"since":"sidebar","width":0}}` | Width change against a kept box, exactly or `{"min":30}`. |
| `{"inside":{"target":T,"of":T,"axis":"y"}}`, `{"contains_point":{"target":T,"point":T,"axis":"x"}}` | Box containment. |
| `{"compare":{"left":NUMBER,"op":"<=","right":NUMBER}}` | `op` is `<`, `<=`, `==`, `!=`, `>=`, or `>`. `NUMBER` is a literal, `{"of":T,"get":"width"}` (`x`, `y`, `width`, `height`, `right`, `bottom`, `center_x`, `center_y`, `value`), `{"pref":"text_size","default":0}`, `{"count":{role target}}` (also `"rendered":false` and `"states"`), or `sum`, `minus`, `times`, `divide`, `floor_divide`, `ceil`, `abs` over them. A target off screen is a mismatch, so a wait keeps polling. |
| `{"editor_within_viewport":{"of":"rename"}}`, `{"editor_within_window":{"of":"rename","pane":"$root"}}` | The inline editor stays inside the viewport or window. |
| `{"blank":{"target":T,"at":[...]}}`, `{"band_selected":{"from":"start","to":"end"}}` | The point lands on nothing but panels; the marquee band's visible rows are all selected. |
| `{"pixel":{"target":T,"rgb":[...]}}`, `{"accent_pixels":{"target":T,"inset":2,"more_than":100}}` | The centre pixel's colour; more than N pixels in the box, inset on every side, are exactly the accent of the theme `settings.toml` names in `data/themes/catalog.toml`. |
| `{"repainted":{"since":"before"}}`, `{"contrast_below":{"of":"dragging","to":"resting","share":0.8}}`, `{"elapsed":{"since":"released","ms":600}}` | A fresh grab of a captured box differs from it outside the cursor's square (from 2 px before the pointer to 32 px after it); `of`'s contrast is below `share` times `to`'s; time since a kept instant. |
| `{"largest_image":{"within":T,"wider_than":40,"size":[160,80]}}` (or `"exceeds"`) | Of the rendered images inside the target wider than `wider_than`, the largest by area (the first of equals) exists, is exactly `size`, or is larger than `exceeds` on either axis. |

### Combinators and NAMES

`{"all":[...]}`, `{"any":[...]}`, `{"not":PREDICATE}`.

`NAMES` is an exact list in visual order, or an object of any of:

| Key | Holds when |
| --- | --- |
| `contains`, `excludes` | Every listed name is present; none is. |
| `count`, `at_least` | Exactly, or at least, that many names. |
| `first` | The first name is this one. |
| `first_above`, `any_from` | The first name sorts after this one; some name sorts at or after it. |
| `starts_with`, `ends_with` | The names begin, or end, with this list. |
| `in_order` | Each is listed, their first occurrences in this order. |
| `set` | The names, as a set, are exactly these. |
| `occurrences` | `{"a.txt":2}`: each name occurs that many times. |
| `toggled` | `{"names":[...],"against":[...]}`: each of `names` is listed exactly when `against` does not list it. |
| `sorted_casefold_from` | `1`: the names from that index on are sorted case-insensitively. |

Strings compare by code point. `panes` takes `NAMES` too.

## Protocol, replies, and the snapshot

`strata --qa <directory>` (cargo feature `qa`, never in releases) opens the
normal window on the directory, then reads one JSON command per line on
stdin, runs it on the GTK main loop, and writes one reply per line on stdout
once the main context drains. `strata --qa --portal` runs the FileChooser
portal backend instead. Input is real XTEST sent from inside the process;
state is read from the widget tree by accessible names, with no AT-SPI bridge.

`STRATA_INPUT_LOG=<path>` appends each synthetic event, one per line; the
runner sets it for every run and saves the file as `input.log`:

```text
key <keycode> down|up
motion <x> <y>
pointer <x> <y> button <n> down|up
```

Coordinates are X root coordinates minus the main window's X surface origin
(a portal run's adopted chooser's), and keycodes are the X server's.
`motion` records each XTEST motion, both steps of the approach and every drag
point included; `pointer` is a button event at the final pointer position
before it, with the wheel as buttons 4 and 5.

A reply is `{"ok","app","changed","criticals","result","failures","error"}`:
`ok` false with `error` stops the run; `failures` lists
`{"predicate","actual","message"}`; `changed` names the `app` keys that differ
from the previous reply; any GLib warning or critical in `criticals` fails
the run. `app` is the snapshot:

```json
{
  "view_mode": "Columns",
  "directory": "strata-qa-fixture-ab12CD",
  "panes": [
    {"name": "strata-qa-fixture-ab12CD", "selected": ["archive"], "focused": null},
    {"name": "archive", "selected": [], "focused": "todo.txt"}
  ],
  "focused": "todo.txt",
  "focused_pane": "archive",
  "dialog": null,
  "menu": null,
  "preview": null,
  "field": null,
  "matches": ["todo.txt"]
}
```

The `app` keys are exactly these:

- `view_mode`: the first pane's presentation, or null with no pane on screen.
- `directory`: the focused pane's directory, else the deepest pane's;
  `focused_pane`: the focused pane's name.
- `panes`: left to right, each with its name, its selected names in visual
  order, and its focused entry or null. A focused widget that is not mapped
  (a row scrolled away) holds no focus.
- `focused`: the name of the entry holding focus.
- `dialog`: the open dialog's name, or null.
- `menu`: the open menu's items, or null.
- `preview`: every non-empty name and text in the open preview drawer in tree
  order, or null when it is closed.
- `field`: the focused editable field as
  `{"name","text","caret","selection","focused"}` (`selection` is
  `[start, end]` or null), or null. Caret and selection come from
  `gtk::Editable`, so they are the offsets AT-SPI's text interface reports.
- `matches`: the deepest pane's search results, or else its entries.

The snapshot, every target, and the input log belong to the window the run
started with; reach a floating chooser through a `toplevel` target.

Setup, launch, layout, artifact, and teardown problems, a reply that never
arrives, and a process that exits are harness errors, not step failures: they
fail even a `known_bug` run, which only a scenario step can satisfy.

## Artifacts

A failed run, or every run with `STRATA_QA_KEEP_ARTIFACTS=1`, writes
`target/qa-artifacts/<scenario>/<run label>/`:

| File | Holds |
| --- | --- |
| `failures.txt` | Failed runs only: every error and failure, one per line. |
| `result.json` | Scenario, run, mode, errors, and each failure with its line and step. |
| `transcript.jsonl` | `{"command":..., "reply":...}` per step, starting with the `{"launch":true}` reply (and one per restart): the `app` snapshot after every step. |
| `tree.txt` | The widget tree: `Type role="..." name="..." description="..." (x,y wxh)`, headed by `focus: ...` (marked when focus sits on an unmapped widget). |
| `state.json` | The snapshot after the last step, once two reads 250 ms apart agree (at most 5 s). |
| `screenshot.png` | The X root at the end (the sway output under `STRATA_QA_DISPLAY=wayland`). |
| `strata.log`, `xvfb.log` | The app's log and the X server's (`sway.log` under Wayland). |
| `fcitx5.log` | With `input_method`: the input method's log. |
| `input.log` | Every synthetic event, as above. |
| `fixture.txt` | The fixture listing with sizes and link targets. |
| `<name>.png`, `<name>.actual.png`, ... | `screenshot` and `baseline` captures. |
