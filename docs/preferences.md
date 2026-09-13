# Preference lifecycle

Application-wide preferences live in `ui::theme::Preferences`. `ThemeManager`
loads them once per application process and persists changes atomically to
`$XDG_CONFIG_HOME/strata/settings.toml` (normally `~/.config/strata/settings.toml`).
The manager's historical name does not make non-theme settings window-local.
Fresh installations select Tokyo Night, unless an available Omarchy theme is
followed automatically. Saved theme choices remain unchanged.

Settings-wide search is transient, panel-local UI state, not a saved preference.
It filters the existing bound controls rather than creating copies. Register new
settings in `settings/search.rs`; `settings_option` tags ordinary rows, while
custom sections use `search::tag`. Keep installation-specific availability
separate with `search::set_available`, so clearing a query cannot reveal an
unsupported release-channel selector. Lazy pages apply the latest query when
they finish loading.

## One initialization and update path

Use `ThemeManager::bind_preference(anchor, read, apply)` for cached behavior and
controls. The binding applies the current value immediately, then applies only
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
| Folder peeking, single-click previews, mode, density, grouping, per-mode click counts, auto-refresh | Every browser binds at construction, including lazily rebuilt view modes. The chooser explicitly disallows folder peeking regardless of the saved value. |
| Hidden files | Shared across existing browsers and new columns. |
| Cross-device drag and drop | Drop dispatch reads the current Copy, Move, or Ask strategy; unresolved volume lookups follow the same cross-device policy. |
| Sort key/direction, folders-first | Shared defaults for new columns; an existing column keeps its own sort, selection and navigation. Sorting a column updates the persisted defaults. |
| Type-to-search, opening search results directly | Keyboard/search actions read the current manager value at dispatch. |
| Include subfolders | Every pane filter binds at construction, including lazy view rebuilds. Enabled by default; disabling indexes only immediate files and folders, without traversing descendants. Live changes cancel pending queries and invalidate old result streams before refreshing the active filter. Global search remains recursive. |
| Element glow | Shared semantic glow color is applied by the manager before Settings opens and updated live across windows, dialogs, menus, and rebuilt views. Focus outlines and ordinary depth shadows are preserved. |
| Reduced motion | Set before any window is constructed; animation helpers read the current process-wide value. |
| Theme, Omarchy following, text size | Shared CSS is applied by the manager; controls and theme-card selections bind to preferences. Newly saved custom themes appear in other open theme pages. Missing themes/Omarchy use the existing fallback policy. |
| Keybinding hints | Footers and settings controls bind immediately and live. |
| Hardware video acceleration/backend | Preview providers read the current choice when requesting a preview; changing it does not restart an already playing file. Settings controls and backend availability synchronize live. |
| Preview mute/volume | Every player's controls and media stream bind to the saved audio state. Slider changes publish/persist together, without a delayed stale save overwriting another window or being discarded when closing a preview. |
| Automatic updates, release channel | Eligibility checks read current preferences. Controls synchronize, and all windows clear outdated notices when these preferences change, even without opening Settings. A package-managed installation's tracked channel is enforced when read, not by constructing Settings. |
| Sidebar order | Existing sidebars bind to the shared order. |
| Folder colors/custom icons | Icon resolution reads the manager; existing customization refreshes notify rendered icons. |

Location, selection, history, each column's sort, filter query, transient theme
catalog filters, dialogs, and preview playback position remain window-local.
Pinned places, portal integration and other externally managed state have their
own stores and are not fields in the application preferences schema.
Synchronization between independently running application processes, or manual
external edits to `settings.toml` while Strata runs, is not supported by this
in-process binding mechanism. External edits are read on the next launch.

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

## Element glow

In **Settings → Appearance → Effects**, turn off **Element glow** to remove
accent-colored glow from dialogs, menus, controls, and animated feedback.
It is enabled by default and saved as `element_glow = true`. Changes apply
immediately across windows. Focus outlines, ordinary depth shadows, and animation
movement are unchanged; use **Reduce motion** to disable nonessential animations.

| Glow enabled (default) | Glow disabled |
| --- | --- |
| ![Element glow enabled](images/element-glow-on.png) | ![Element glow disabled](images/element-glow-off.png) |

## Filter scope

In **Settings → General → Search & filtering**, **Include subfolders** is
on by default. Turn it off to match only immediate files and folders, without
redundant path subtitles. The choice applies to pane filtering in Columns, Icons,
and List views, not global search.
Changing it refreshes active filters across windows and is saved for next launch.

Default recursive results:

![Filtering with subfolders included](images/filter-scope-recursive.png)

The same query with subfolders excluded:

![Filtering only the current directory](images/filter-scope-directory.png)

## Adding a preference

1. Add its backward-compatible serialized field/default, getter and setter.
2. Bind cached consumers at construction, or read directly at action dispatch.
   Do not add behavior initialization to a Settings page.
3. Bind its controls with the shared helpers; document any deliberate override.
4. Extend the **exhaustive** fixture in `ui/theme/tests/preferences.rs` (it has no
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
