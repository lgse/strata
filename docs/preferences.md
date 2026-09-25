# Preference lifecycle

Application-wide preferences live in `ui::preferences::Preferences`. The
`ui::preferences::PreferenceManager` loads them once per application process and
persists changes atomically to
`$XDG_CONFIG_HOME/strata/settings.toml` (normally `~/.config/strata/settings.toml`).
It owns the serialized schema, change notifications, and widget bindings for every
settings consumer, theme-related or not. `ui::theme::ThemeManager` separately owns
the theme catalog, shared CSS application, custom themes, and Omarchy following;
it reads preferences through the `PreferenceManager` and reapplies shared CSS when
appearance preferences change. Fresh installations select Tokyo Night, unless an
available Omarchy theme is followed automatically. Saved theme choices remain
unchanged.

Settings-wide search is transient, panel-local UI state, not a saved preference.
It filters the existing bound controls rather than creating copies. Register new
settings in `settings/search.rs`; `settings_option` tags ordinary rows, while
custom sections use `search::tag`. Keep installation-specific availability
separate with `search::set_available`, so clearing a query cannot reveal an
unsupported release-channel selector. Lazy pages apply the latest query when
they finish loading.

## One initialization and update path

Use `PreferenceManager::bind_preference(anchor, read, apply)` for cached behavior and
controls, and `ThemeManager::bind_theme_preference(anchor, read, apply)` for values
derived from the theme catalog or live Omarchy availability. The binding applies the
current value immediately, then applies only
changes to its selected value. There is no separate startup initializer to keep
in sync with the change handler. Every setter goes through `save_preferences`,
which deduplicates unchanged preferences and publishes changes through the same
notification mechanism. Failed writes are logged, still apply in memory, and
are retried on the next save attempt. If an existing settings file cannot be read
or parsed as TOML, startup logs a warning and uses temporary defaults. Preference
changes still apply in memory, but saving is disabled for that manager's lifetime
to preserve the original file. Fix the file and restart Strata to resume saving.
Missing files allow normal first-run saves; invalid values in otherwise valid
TOML still use the existing per-entry recovery.

Bindings use weak widget anchors and remove their listeners when the anchor is
destroyed. Callbacks must capture weak references to any owned widget/state or
manager. Reentrant changes are delivered in another notification pass, without
holding preference/listener borrows across callbacks. A binding updates its
last-seen value before calling its consumer.

Settings pages **only edit preferences**; they must not initialize application
behavior. Boolean and segmented controls use `settings::bindings` helpers,
which ignore programmatic synchronization instead of writing it back. A
multi-field choice reads its other fields from the manager, not from another
control that might be midway through synchronization.

## Consumers and intentional scopes

| Stored preferences | Consumer / application point |
| --- | --- |
| Default directory | New windows without an explicit target read the current choice before navigating, without opening Settings. Existing windows and explicit targets are unchanged. Missing directories fall back to home and clear the saved choice; Reset also restores home. |
| Folder peeking, single-click previews, columns selection mirror, mode, density, grouping, per-mode click counts, auto-refresh | Every browser binds at construction, including lazily rebuilt view modes. The chooser explicitly disallows folder peeking and the columns selection mirror regardless of the saved values. |
| Hidden files | Shared across existing browsers and new columns. |
| Open folder after dropping files | Drop dispatch reads the saved choice (off by default), including confirmation of cross-device drops. Successful drops reveal the destination only when enabled and the user is still at the transfer origin. Paste and Move/Copy to remain unchanged. |
| Cross-device drag and drop | Drop dispatch reads the current Copy, Move, or Ask strategy; unresolved volume lookups follow the same cross-device policy. |
| Sort key/direction, folders-first | Shared defaults for new columns; an existing column keeps its own sort, selection and navigation. Explicit field sorting updates the persisted defaults. Camera Photos libraries instead open in column-local Device order (see below). |
| Type-to-search, opening search results directly | Keyboard/search actions read the current manager value at dispatch. |
| Include subfolders | Every pane filter binds at construction, including lazy view rebuilds. Enabled by default; disabling indexes only immediate files and folders, without traversing descendants. Live changes cancel pending queries and invalidate old result streams before refreshing the active filter. Global search remains recursive. |
| Element glow | Shared semantic glow color is applied by `ThemeManager` when the appearance preferences change, before Settings opens and live across windows, dialogs, menus, and rebuilt views. Focus outlines and ordinary depth shadows are preserved. |
| Reduced motion | Set before any window is constructed; animation helpers read the current process-wide value. |
| Theme, Omarchy following, text size | `ThemeManager` applies shared CSS when theme selection, Omarchy following, text size, or element glow change; controls and theme-card selections bind to preferences through `ThemeManager::bind_theme_preference`. Newly saved custom themes appear in other open theme pages. Missing themes/Omarchy use the existing fallback policy. |
| Interface renderer | GTK selects the renderer at process startup. The saved GTK default or Cairo choice is read before GTK initializes; GTK default is selected for new installs. The control and Restart button synchronize across Settings windows, but changes take effect only after restarting Strata (via the button or after fully quitting and reopening). An explicit `GSK_RENDERER` always overrides the saved choice. |
| Keybinding hints | Navigation hints and the shortcuts button bind immediately and live. When hidden, the status bar appears only while the clipboard badge or F1 reference needs it; otherwise the empty bar is hidden. |
| Thumbnail workers | Browser construction binds the shared decoder limit before Settings opens. Changes apply across windows and rebuilt views; lowering the limit lets active work finish and retires excess idle supervisors. |
| Icons view thumbnail size | Every browser binds at construction, before the browser mode preference applies, so an Icons pane built at startup already uses the saved size. The popover slider's own live change persists it; other windows' visible Icons panes move their slider (and resize) to match. Clamped to 32–256 px; not exposed in Settings. |
| Hardware video acceleration/backend | Preview providers read the current choice when requesting a preview; changing it does not restart an already playing file. Settings controls and backend availability synchronize live. |
| Preview text wrap | Every text preview and header toggle binds to the saved wrap choice, including newly loaded files. Off by default. |
| Preview autoplay | Read when a video, audio, or GIF preview is first shown. Off by default: playback waits for an explicit play action, and the center play affordance is shown instead. Does not affect resuming playback that was already active before a preview pane was temporarily hidden by a resize. |
| Render documents by default | A newly loaded Markdown or HTML preview reads the current choice for its initial Rendered or Source view. Switching the view of an open document does not change the saved default. |
| Preview mute/volume | Every player's controls and media stream bind to the saved audio state. Slider changes publish/persist together, without a delayed stale save overwriting another window or being discarded when closing a preview. |
| Automatic updates, release channel | Eligibility checks read current preferences. Controls synchronize, and all windows clear outdated notices when these preferences change, even without opening Settings. A package-managed installation's tracked channel is enforced when read, not by constructing Settings. |
| Sidebar order | Existing sidebars bind to the shared order. |
| Sidebar default-place visibility | Existing sidebars bind to the shared Home, Trash, Network, Recent, and standard-folder visibility and rebuild. Enabled by default; hiding removes that place from the sidebar without changing pins or devices. Recent is also omitted when GTK recent-file tracking or the runtime Recent VFS backend is unavailable, and from local-only sidebars. Toggle the location chips under General → Sidebar; existing default-place Unpin context actions remain available where supported. Re-enable a hidden place’s chip to restore it. |
| Modified date format | Modified-time labels read the saved format at every render; already-open labels re-render live. Properties uses full absolute local timestamps for Relative, while preserving ISO 8601 and Long. |
| Folder colors/custom icons | Icon resolution reads the manager; existing customization refreshes notify rendered icons. |

Location, selection, history, each column's sort, filter query, transient theme
catalog filters, dialogs, and preview playback position remain window-local.
Pinned places, portal integration and other externally managed state have their
own stores and are not fields in the application preferences schema. Udiskie
encrypted-volume integration is stored in `$XDG_CONFIG_HOME/udiskie/config.yml`
plus `$XDG_DATA_HOME/strata/udiskie-install/state.toml`.
Synchronization between independently running application processes, or manual
external edits to `settings.toml` while Strata runs, is not supported by this
in-process binding mechanism. External edits are read on the next launch.

## Thumbnail workers

Under **Settings → General → Performance**, use the **Thumbnail workers** −/+
control to choose **1–16** concurrent decoders across all windows. The initial
default uses the available CPU count, capped at four (two if CPU detection fails).
Click the number to reset. Higher values can improve throughput but consume more
CPU and memory; they do not change image resolution or preview playback.

The count is saved as `thumbnail_workers` and applies immediately. Busy jobs finish
normally when the count is lowered. Idle excess supervisors retire asynchronously;
lightweight executor threads remain available for reuse. `STRATA_THUMBNAIL_WORKERS`
seeds the default only when no saved count exists; an explicit saved choice wins.
The reset value also honors that environment variable.

## Camera Photos ordering

The flattened **Photos** view for iPhones and other camera devices opens in
**Device order**, regardless of the saved folder sort. Incoming photos append in
discovery order, with no automatic reshuffle when loading finishes. Newer
date-named folders are visited first, but files within them are not guaranteed
chronological order; this is not a creation-date or capture-date sort.

Choose **Name**, **Size**, **Modified**, or **Type** from the pane's sort menu
(or a sortable List heading) to explicitly sort. Later batches then follow that
sort. Choose **Device order** again to reload the library in discovery order.
Refresh retains the current column's ordering choice; reopening the Photos
library starts in Device order. The direction button and List type grouping are
disabled in Device order; the saved grouping choice is retained for explicitly
sorted, fully loaded libraries and ordinary folders. Camera subfolders and saved
folder defaults are not changed by entering Device order.

## Text size and display scaling

In **Settings → Appearance → Text**, enter an integer text size
from **8 to 48 logical pixels**. The default is **13 px**. The setting applies
immediately across windows, file views, settings, menus, dialogs, and text
previews; opening Settings is not required to initialize it. **Appearance** also
has decrease/increase controls and a size button that resets to the default.
Use **Ctrl++** (or **Ctrl+=**), **Ctrl+−**, and **Ctrl+0** to increase, decrease,
and reset, including while an inline editor or Settings is open.

The size is saved numerically, for example `text_size = 27`. Existing `"small"`,
`"medium"`, and `"large"` settings still load as 11, 13, and 15 px. Out-of-range
integers are clamped; unknown legacy names use 13 px.

Desktop text scaling multiplies the chosen size once. GTK/compositor monitor
scaling then converts logical coordinates to device pixels; Strata does not
multiply widget geometry by a monitor's scale factor. Moving between monitors
therefore does not overwrite the saved size. Toolbar/row icons and initial
column widths follow typography. Grid captions reserve their measured space,
Settings compacts its navigation relative to text size, and oversized dialogs
and settings content remain scrollable within the available window.

Thumbnail zoom, image/PDF zoom, media decode resolution, volume, and playback
position remain independent of interface text size. At extreme sizes on small
logical displays, scrolling or resizing panes may be necessary. Physical
mixed-DPI monitor transitions still need compositor-specific manual testing.

## Interface renderer

**Settings → Appearance → Rendering** offers **GTK default** (initial choice) and **Cairo**.
Cairo avoids small-text artifacts on some systems but can use more CPU and reduce
rendering performance. The choice is saved as `interface_renderer = "cairo"` or
`"system"`. Use **Restart now** after changing the choice, or fully quit and reopen Strata,
including closing all windows. A caller-supplied `GSK_RENDERER` (even an empty value)
takes precedence. Cairo selected by Strata is a process environment setting,
so applications started by Strata may inherit it; it does not change desktop
or system-wide settings.

## Element glow

In **Settings → Appearance → Effects**, turn off **Element glow** to remove
accent-colored glow from dialogs, menus, controls, and animated feedback.
It is enabled by default and saved as `element_glow = true`. Changes apply
immediately across windows. Focus outlines, ordinary depth shadows, and animation
movement are unchanged; use **Reduce motion** to disable nonessential animations.

## Drag-and-drop destination

In **Settings → General → File transfers**, enable **Open folder after dropping
files** to show the destination after a successful drop: a child column in
Columns, or navigation in Icons and List. It is off by default and saved as
`open_folder_after_drop = false`. Changes apply to subsequent drops across
windows without restarting. Navigating away during a transfer is respected.
Paste and **Move/Copy to…** continue to reveal their destination independently.

## Modified date format

In **Settings → General → Date & time**, **Modified date format** selects how file
modified times appear; each choice lists a live example rendered from the
current time. **Relative** (default) uses these buckets in lists and previews:

- Under a minute: "Just now"; under an hour: whole minutes such as "5m ago".
- Under 24 elapsed hours: whole hours such as "3h ago", even across midnight.
  On a long daylight-saving fall-back day, timestamps still within the same
  local calendar day continue to show actual whole elapsed hours, such as
  "24h ago", without clamping.
- After that, 1–6 local calendar days ago: full weekday name, such as "Monday".
- 7–30 local calendar days ago: whole calendar weeks, such as "2w ago".
- Older dates: "Sep 1, 23:30" in the current local year, or "Sep 1, 2025" in
  an earlier year.

Timestamps up to a minute in the future read "Just now" as clock skew; further
future timestamps use an absolute date and time. Calendar boundaries and date
labels use local time, while minute/hour buckets use elapsed time across DST.
**ISO 8601** always renders `2026-09-17 14:30`; **Long** renders
"September 17, 2026, 14:30". Saved as `date_format = "relative"`. Open labels
re-render immediately when the choice changes, and the 30-second refresh still
applies for elapsed buckets.

**Properties** always shows an absolute local date, time, and year. With Relative
selected, it uses "Sep 24, 2026, 9:30 PM"; ISO 8601 and Long retain their selected
formats. This applies both to cached timestamps shown when the dialog opens and
to asynchronously loaded file/folder metadata. Open Properties dialogs follow
format changes live, just like lists and previews.

## Filter scope

In **Settings → General → Search & filtering**, **Include subfolders** is
on by default. Turn it off to match only immediate files and folders, without
redundant path subtitles. The choice applies to pane filtering in Columns, Icons,
and List views, not global search.
Changing it refreshes active filters across windows and is saved for next launch.

## Adding a preference

1. Add its backward-compatible serialized field/default, getter and setter.
2. Bind cached consumers at construction, or read directly at action dispatch.
   Do not add behavior initialization to a Settings page.
3. Bind its controls with the shared helpers; document any deliberate override.
4. Extend the **exhaustive** fixture in `ui/preferences/fixtures.rs` (it has no
   `..Default` escape hatch) and the setter notification/persistence coverage.
   That test compares the union of changed keys against every serialized field,
   so extending the fixture without exercising the new setter still fails.
5. Test the effective behavior with a saved non-default value before opening
   Settings, a live change in two windows, and any relevant lazy view rebuild.
   Test both directions; a test that only saves and deserializes is insufficient.

The regression suites also check no writes from opening Settings, no duplicate
notifications, reentrant changes, listener cleanup, failed-write retries,
chooser overrides, type-to-search keyboard behavior, and synchronized media
controls. Run GTK tests on the private display described in `e2e-testing.md`.
