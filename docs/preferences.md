# Preference lifecycle

Application-wide preferences live in `ui::preferences::Preferences`. The
`ui::preferences::PreferenceManager` loads them once per application process and
persists changes atomically to
`$XDG_CONFIG_HOME/strata/settings.toml` (normally `~/.config/strata/settings.toml`).
The file may be a symlink, for example into a dotfiles repository: Strata follows
up to eight links owned by the current user to a regular file the user owns,
replaces that target atomically in its own directory, keeps its permission bits
and leaves the links in place. Links owned by another user, links to directories
or missing targets, and longer chains are refused. A missing file is created
with owner-only permissions; an existing one keeps its permission bits.
It owns the serialized schema, change notifications, and widget bindings for every
settings consumer, theme-related or not. `ui::theme::ThemeManager` separately owns
the theme catalog, shared CSS application, custom themes, and Omarchy following;
it reads preferences through the `PreferenceManager` and reapplies shared CSS when
appearance preferences change. Fresh installations select Tokyo Night, unless an
available Omarchy theme is followed automatically. Saved theme choices remain
unchanged. Folder peeking is off by default when no value is saved; explicitly
saved choices are preserved. Enable it under **Settings → General → Browsing**.
When enabled, automatic folder peeks in Icons and List wait for a 500 ms hover;
explicit keyboard peeks in those views remain immediate. Miller columns never
show folder-peek popovers, regardless of the saved preference.

Settings-wide search is transient, panel-local UI state, not a saved preference.
It filters the existing bound controls rather than creating copies. Register new
settings in `settings/search.rs`; `settings_option` tags ordinary rows, while
custom sections use `search::tag`. Keep installation-specific availability
separate with `search::set_available`, so clearing a query cannot reveal an
unsupported release-channel selector. Lazy pages apply the latest query when
they finish loading. Give each new target a `settings_keywords.<id>` entry in
every catalog (see [Internationalization](internationalization.md)). Search
routes to pages through the `NAVIGATION` table in `settings.rs`.
`ui::settings::search::tests::every_page_row_is_registered_for_search` builds
every page and fails on a row or section without a target. Register the row, or
tag status-only content with its section's target (the managed-install row uses
"Check for updates"), rather than exempting it.

## One initialization and update path

Use `PreferenceManager::bind_preference(anchor, read, apply)` for cached behavior and
controls, and `ThemeManager::bind_theme_preference(anchor, read, apply)` for values
derived from the theme catalog or live Omarchy availability. The binding applies the
current value immediately, then applies only
changes to its selected value. There is no separate startup initializer to keep
in sync with the change handler. Every setter goes through `save_preferences`,
which deduplicates unchanged preferences and publishes changes through the same
notification mechanism. The setter returns once the change is published; the file
is written by Strata's one background settings writer (`storage::writer`), which
runs every save of Strata's own settings and state files in order on its own
thread and keeps only the newest contents of a file whose save is still waiting.
Quitting waits up to 10 seconds for queued saves; a crash before a queued save
runs loses that last change. Failed writes are logged with the path and reason, still
apply in memory, and are retried on the next save, so a transient failure such as
a full disk recovers once its cause is fixed. The first failure also opens a
"Settings can't be saved" dialog with the path and reason, saying that changes
last only until Strata closes. It appears once per failure streak, in the active
browser window where the change was made, and again only after a save has
succeeded and then failed. A failure while no browser window is active, such as
a background Omarchy theme update, is only logged and leaves the notice for the
next failure. If an existing settings file cannot be read
or parsed as TOML, startup logs a warning and uses temporary defaults. Preference
changes still apply in memory, but saving is disabled for that manager's lifetime
to preserve the original file. The first change made in a browser window then
opens a "Settings file can't be read" dialog instead, once. Fix the file and restart
Strata to resume saving. Portal file choosers never show these dialogs; their
failures are only logged.
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
| Default directory | New windows without an explicit target read the current choice before navigating, without opening Settings. Existing windows and explicit targets are unchanged. The folder is checked off the interface thread: one proven missing or not a folder falls back to home and clears the saved choice, while one that cannot be checked, such as on a server that does not answer, opens with its error and keeps the choice. Reset also restores home. |
| Folder peeking, single-click previews, columns selection mirror, mode, density, grouping, per-mode click counts | Every browser binds at construction, including lazily rebuilt view modes. Miller columns disallow folder-peek popovers regardless of the saved value; Icons and List retain the preference. The chooser explicitly disallows folder peeking and the columns selection mirror regardless of the saved values. |
| Watch network folders | On by default. Every browser and the portal chooser bind it at construction, before Settings opens, and follow changes live across windows, tabs and rebuilt views. On, open folders on mounts that other computers or programs also change (NFS, SMB, sshfs and other FUSE mounts except `fuseblk`, and GVfs network locations) are checked every few seconds while their window shows them and at once when it regains focus, and re-read when a check finds a change. Off, those folders are not checked or re-read, so other computers' changes show only after a refresh. Those with a working change monitor still show this computer's changes at once; those without one (GVfs SMB and SFTP locations, or a folder whose monitor could not be created) show them only after a refresh too. Local folders without a working change monitor are checked either way. It replaces the former Auto-refresh setting: a saved `auto_refresh_interval` is ignored and dropped on the next save. Set under **Settings → General → Performance**. See [Changes made elsewhere](filesystem-compatibility.md#changes-made-elsewhere) for the cadence and what a check costs. |
| Show sizes in Columns view | Enabled by default. Miller columns bind file-size and immediate folder-item-count visibility at construction, across windows and rebuilt columns. The existing badge appears on hover, selection, or the active path; disabling it hides the badge. Column widths and the row presentation are unchanged. Settings → General → Browsing and the Columns Appearance menu edit the same saved choice. List and Icons metadata are unchanged. |
| Hidden files | Shared across existing browsers and new columns. Hidden entries stay in the model and filesystem monitor stream even while hidden, so revealing them is an in-memory filter change, not a directory rescan. |
| Open folder after dropping files | Drop dispatch reads the saved choice (off by default), including confirmation of cross-device drops. Successful drops reveal the destination only when enabled and the user is still at the transfer origin. Paste and Move/Copy to remain unchanged. |
| Cross-device drag and drop | Drop dispatch reads the current Copy, Move, or Ask strategy; unresolved volume lookups follow the same cross-device policy. |
| Default sort key/direction | The sort for folders without their own; see [Per-folder sort and thumbnail size](#per-folder-sort-and-thumbnail-size). Set under **Settings → General → Browsing → Default sort**; open columns that follow the default re-sort live in every window. With per-folder settings off, an explicit field sort updates the default instead and existing columns keep their own sort. Camera Photos libraries open in column-local Device order (see below). |
| Folders first | Application-wide: toggling it in any pane's sort menu re-sorts every open column in every window and applies to new columns. Recent keeps its fixed order. |
| Remember sort and icon size per folder | On by default. Browsers and the portal chooser bind at construction, before Settings opens, and resolve each column's folder sort and the Icons folder's size live. Off restores the last-sort-wins defaults; saved folder values stay stored but unused. See [Per-folder sort and thumbnail size](#per-folder-sort-and-thumbnail-size). |
| Type-to-search, opening search results directly | Keyboard/search actions read the current manager value at dispatch. |
| Global search exclusions | Each global-search invocation reads the current rules before indexing, including before Settings opens. Existing search snapshots keep their rules until reopened. Exclusion editors synchronize across windows. Pane filters, 10xer path search, folder-history search, and destination pickers retain their explicit browsing scope and ignore these rules. |
| Search time limit, search entry limit | Global search, pane filters, 10xer path search, and folder pickers read the current limits when they start a query, including before Settings opens. A changed limit restarts the next query's index walk; an open result snapshot keeps its limits until it requeries. See [Search limits](#search-limits). |
| 10xer mode | Interactive browsers and the portal file chooser bind pane Close/filter/refresh/sort chrome and the Yazi-style keymap at construction, including lazily rebuilt views. Window Search hides in interactive browsers. Window Close, chooser Accept/Cancel, and List column headings stay. Unclaimed letter commands return focus from non-text window chrome to the listing; text fields, menus, and previews retain their own input. Off by default; toggle with `Ctrl+Shift+M` or Settings → General → Browsing. Enabling shows a brief non-interactive splash that respects Reduce motion; reopening an already-enabled window does not replay it. |
| Include subfolders | Every pane filter binds at construction, including lazy view rebuilds. Enabled by default; disabling indexes only immediate files and folders, without traversing descendants. Live changes cancel pending queries and invalidate old result streams before refreshing the active filter. Global search remains recursive. 10xer mode ignores it: entering or leaving the mode re-runs active filters in the new scope. |
| File icon colors | Brand colors by default: Catppuccin file-type icons keep their own palette, Mocha on dark themes and Latte on light ones. Theme colors draws every one of their strokes in the theme accent instead. `ThemeManager` applies the choice with the other appearance preferences, before Settings opens and live across windows and rebuilt views; Lucide interface icons and folders are unaffected. Set under **Settings → Appearance → File icons**. |
| Element glow | Shared semantic glow color is applied by `ThemeManager` when the appearance preferences change, before Settings opens and live across windows, dialogs, menus, and rebuilt views. Focus outlines and ordinary depth shadows are preserved. The video preview's ambient light, and the band it needs around the frame, follow the same switch live. |
| Reduced motion | Set before any window is constructed; animation helpers read the current process-wide value. |
| Corner roundness | `corner_roundness` stores a percent of the stylesheet's corner radii (0–200, default 60). `ThemeManager` reparses the shared stylesheet with every pixel radius scaled when the value changes, before Settings opens and live across windows, dialogs, menus, and rebuilt views; percentage radii that draw circles are unchanged. Set under **Settings → Appearance → Shape**. |
| Interface font | `interface_font` stores a font family name; absent means the bundled JetBrains Mono. `ThemeManager` loads the family into a stylesheet below Strata's own, so the explicit monospace of text, source, and command previews still wins, and into GTK's font setting, before Settings opens and live across windows, dialogs, menus, and rebuilt views. The bundled families follow the saved one, so text keeps the default look if it is uninstalled. Independent of text size. Choose it, or Default to restore the bundled font, under **Settings → Appearance → Text**; the picker lists installed families and filters them as you type. |
| Theme, Omarchy following/variant, text size | `ThemeManager` applies shared CSS when theme selection, Omarchy following, Omarchy variant, text size, or element glow change; controls and theme-card selections bind to preferences through `ThemeManager::bind_theme_preference`. Newly saved custom themes appear in other open theme pages. Missing themes/Omarchy use the existing fallback policy. |
| Language | Initialized before application UI from the saved manual language or system locale priorities. The selection and Restart button synchronize across Settings windows, but the running interface keeps its startup language until a full restart. This deliberate restart-to-apply exception avoids rebuilding active browsers, dialogs, and operations. |
| Interface renderer | GTK selects the renderer at process startup. The saved GTK default or Cairo choice is read before GTK initializes; GTK default is selected for new installs. The control and Restart button synchronize across Settings windows, but changes take effect only after restarting Strata (via the button or after fully quitting and reopening). An explicit `GSK_RENDERER` always overrides the saved choice. |
| Keymap overrides | Every key dispatcher, the portal chooser included, reads the tables built from the saved overrides at press time, before Settings opens; labels that show shortcuts follow changes live across windows. See [Keymap](#keymap). Edited under **Settings → Keyboard**, one table per mode. |
| Keybinding hints | Navigation hints and the shortcuts button bind immediately and live. When hidden, the status bar appears only while the clipboard badge or F1 reference needs it; otherwise the empty bar is hidden. |
| Show thumbnails | Enabled by default. **Settings → Appearance → File icons** switches file thumbnails to the existing file-type icons. Applies before Settings opens and live across browser windows, the portal chooser, searches, peeks, and rebuilt views. Disabling cancels pending thumbnail requests and prevents cache reads and decoding for new requests; re-enabling restores thumbnails for still-bound entries. Live thumbnail changes preserve pending dimensions and duration metadata; rebinding or unbinding a row still discards stale updates. Custom icons and the preview panel are unchanged. |
| STL model thumbnails | Off by default. **Settings → Appearance → File icons** enables fixed-view, software-rendered thumbnails of local STL models when Show thumbnails is also enabled. Applies before Settings opens and live to bound slots across browser windows, the portal chooser, searches, peeks, and rebuilt views; disabling also hides cached STL artwork and cancels pending requests. Rendering uses the existing queue and sandbox, with at most one background STL render at a time across the application process, within the general worker budget. Cancellation and timeout release the background slot after sending SIGKILL; a detached waiter reaps the helper, so a killed process stuck in I/O does not stop the queue. Quick Preview is independent and may render concurrently. Existing input and geometry limits apply; failures leave the file icon. 3MF, FreeCAD, and G-code continue to use embedded images only for file thumbnails. |
| Thumbnail workers | Browser construction binds the shared decoder limit before Settings opens. Changes apply across windows and rebuilt views; lowering the limit lets active work finish and retires excess idle supervisors. |
| List column order and visibility | Browser windows and the portal chooser share column order and visibility, including newly opened List panes. Dragging a heading previews its destination with an accent-colored line; column order changes only on release. Cancelling or releasing outside the headings leaves the order unchanged. The Columns menu, heading context menu, and Appearance → Customize columns edit the same saved choices. Name cannot be hidden. Hiding, showing, reordering, or resetting columns changes only their presentation, never the sort key or direction. A hidden sort column remains active, including for saved folder sorts, newly opened folders, and view switches; showing it again restores its sort indicator. Widths retain their independent browser and chooser scopes below. |
| Browser and chooser column widths | A Columns resize or a List heading resize, including double-click autofit, saves the unscaled width. A Columns drag or autofit stops at the visible browser area: the window minus the preview divider and the sidebar at its full width, so while a narrow window rails the sidebar, columns stop short of the window edge by the width the rail gives up. Columns opened later, including a saved width from before, are narrowed to the current area, and so are open columns when the window first lays out and after a text-size change; returning to the earlier text size restores the dragged width. Resizing the window does not narrow open columns; dragging the edge of one that is wider than the area snaps it to the area. New columns and rebuilt List panes read the latest saved defaults before Settings opens. Dragging a Columns edge resizes that column as the pointer moves, with a **Column width** caption that appears once the pointer rests on the edge and follows it through the drag, and when the drag ends the window's other open columns ease to the same width; double-click autofit changes only its own column. Other windows keep their widths. Browser defaults (`browser_column_width`, `browser_list_columns`) and chooser defaults (`chooser_column_width`, `chooser_list_columns`) are independent. The Name column keeps expanding until it is resized itself. Not exposed in Settings. |
| Icons view thumbnail size | The default size for folders without their own; see [Per-folder sort and thumbnail size](#per-folder-sort-and-thumbnail-size). Every browser binds at construction, and an Icons pane resolves its folder's size before it is built, so it opens at that size. With per-folder settings off, the popover slider sets the default. Visible Icons panes in other windows resize when their folder's size or the default changes. Clamped to 32–256 px; not exposed in Settings. |
| Hardware video acceleration/backend | Preview providers read the current choice when requesting a preview; changing it does not restart an already playing file. Settings controls and backend availability synchronize live. |
| Units | Metric by default: kilometres and metres. Imperial uses miles and feet. Settings → General → Units edits the saved `unit_system` choice. Open previews, including the chooser, update immediately without resetting playback; new files and later sessions use the saved choice. |
| Preview text wrap | Every text preview and header toggle binds to the saved wrap choice, including newly loaded files. Off by default. |
| Preview autoplay | Read when a video, audio, or GIF preview is first shown. Off by default: playback waits for an explicit play action; the generic player shows its center play affordance, and the audio and video views their transport play button. When on, playback starts silent and fades in on a slow-in, slow-out curve, over 1 s from the first frame for video and over 0.5 s from the first sample for audio, unless the saved audio state is muted or the file is shorter than 10 s, which plays at full volume at once; any play, pause, seek, volume or mute input brings the sound in at once, and the saved volume is never changed. Continuing playback into the next file with `<` / `>` keeps its sound. Does not affect resuming playback that was already active before a preview pane was temporarily hidden by a resize. |
| Render documents by default | A newly loaded Markdown or HTML preview reads the current choice for its initial Rendered or Source view. Switching the view of an open document does not change the saved default. |
| Preview mute/volume | Every player's controls and media stream bind to the saved audio state. Slider changes publish/persist together, without a delayed stale save overwriting another window or being discarded when closing a preview. |
| Automatic updates, release channel | Eligibility checks read current preferences. Controls synchronize, and all windows clear outdated notices when these preferences change, even without opening Settings. A package-managed installation's tracked channel is enforced when read, not by constructing Settings. On by default. The switch controls the automatic check, which runs at every start once the first window paints and again when a window or **Settings → Updates** opens if no check has completed in the last 24 hours. Turning it off also locks the channel, hides shown update notices, and discards an in-flight check's result; turning it back on or changing the channel checks immediately. The Updates page reuses the latest completed check and follows later results in every open Settings window, without starting another request just to display them. Running or completed installations keep their own status. **Check now** and the release-notes card (fetched when the Updates page opens, unless cached) ignore the switch. |
| Sidebar order | Existing sidebars bind to the shared order. |
| Sidebar default-place visibility | Existing sidebars bind to the shared Home, Trash, Network, Recent, and standard-folder visibility (Desktop, Documents, Downloads, Music, Pictures, and Videos) and rebuild. Enabled by default; hiding removes that place from the sidebar without changing pins or devices. Recent is also omitted when GTK recent-file tracking or the runtime Recent VFS backend is unavailable, and from local-only sidebars. Toggle the location chips under General → Sidebar; existing default-place Unpin context actions remain available where supported. Re-enable a hidden place’s chip to restore it. |
| Sidebar expanded | Browser windows bind their header toggle at construction: a saved collapsed state starts closed without animation. The header toggle, `Ctrl+B`, or `Ctrl+N` in 10xer mode steps the sidebar from full to compact to hidden and back to full; each step persists and updates all open browser windows live. Expanded by default; not exposed in Settings. The portal chooser keeps its own unsaved sidebar state. |
| Sidebar compact | The second step of the header toggle shows the icon rail and persists it; existing sidebars bind to it and change live, and a saved compact state starts as the rail at any window width. When the full sidebar does not fit beside the content (one column, or the preview and its minimum when a preview is present), a sidebar that is not compact by choice rails automatically; stepping from hidden to full shows the full sidebar anyway, and that stays until the window or the preview next runs out of room. Space that returns releases only an automatic rail. Off by default; not exposed in Settings. The portal chooser keeps its own unsaved compact state. |
| Modified date format | Modified-time labels read the saved format at every render; already-open labels re-render live. Properties uses full absolute local timestamps for Relative, while preserving ISO 8601 and Long. |
| Folder colors/custom icons | Icon resolution reads the manager; existing customization refreshes notify rendered icons, including local sidebar folders, customization previews, and Properties. Sidebar folder icons retain their customization in collapsed mode and across row rebuilds. Local sidebar folders expose the shared Customize action. Entries are keyed by absolute local path, and file operations performed in Strata update them item by item as each item lands, saving once the burst of changes settles. Renames, moves, and their undo/redo carry the item's entries and those of everything under it to the real destination (including a Keep both name), even when a later operation superseded the one that moved them, or a failure or cancel stopped it partway. A move to a remote location drops them. Replace, for a move or a copy, takes the replaced item's entries at and under the destination with it to Trash; a moved item then brings its own. Merge keeps the destination folder's own entry; destination files it moves to Trash take their entries with them, as Replace does, and a merged move carries the merged folder's descendants over same-named destination entries. Trash removes the entries from `settings.toml` and keeps them in memory for the session, at most 10,000 entries (the oldest go first). Put back or undo re-applies them only to the same item, recognised by its mount's [identity rule](filesystem-compatibility.md#identity), at exactly the path it was trashed from; another item that left that path stays plain. Permanent delete removes them. Copies, including duplicates, start uncustomized, and a new item at a path whose item Strata moved or removed starts plain; entries left behind by changes outside Strata, or by an uncustomized item replacing another where Trash is unsupported, still apply to whatever appears at that path. Changes made outside Strata (shell commands, other applications, batch-rename actions, or an open folder renamed underneath a window) are not followed, nor are items that land after the operation's window closed, entries for missing paths are not pruned at startup, so unmounted volumes keep theirs, and a FAT-family destination that renames a moved folder's descendants leaves their entries at names that cannot exist there. |
| Device display labels | `device_labels` stores Strata-only labels by filesystem UUID, with mount URI fallback when a UUID is unavailable. Sidebar device rows and Properties bind at construction, update across open windows, and reapply on row rebuilds. Set label… edits the label; blank restores the system-provided name. Filesystem labels, mount/boot configuration, and other applications remain unchanged. Label editors retain their local draft while their Save action follows the latest shared value. Send-to menus and destructive Format confirmations retain system-provided drive names. No Settings page is required. |
| Recent Send-to destinations | `send_to_recent_destinations` stores up to three relative directory paths per stable removable-device ID. The selection menu lists those that a check on the device's lane last found inside its current canonical root, checks again each time it opens, and activation checks again; no Settings control is exposed. |
| Restore open tabs | Plain launches reopen the previous tabs in strip order with the previously active tab selected, when the Startup toggle is on (the default). Explicit folder arguments, reveal requests, and unlock flows bypass restore. Toggle it under General → Startup; the toggle binds live across Settings windows. |
| Window buttons (minimize, maximize, close) | Every window header binds at construction and updates live. Minimize and maximize are hidden by default; close is shown. The maximize button also restores a maximized window. |

Tab locations persist in a separate session store (`$XDG_CONFIG_HOME/strata/tabs.toml`),
saved whenever tabs change and validated on load.

Reading the files that build the first window happens on the interface thread
before that window shows: `settings.toml`, `folder-views.toml`, `tabs.toml`, custom
themes, the Omarchy theme state and GTK's bookmarks file. These startup reads are an
exception to keeping file calls off the interface thread. Every later save goes
through the settings writer: preferences, folder settings, the tabs session, folder
history, recent applications, custom themes, pinned folders and custom actions. The
source-preview style scheme is never saved: it is built in memory and read through
`resource://`. Open With reads recent applications on the writer
too, so a launch that is still being recorded is listed. Every later re-read after
another process changed a file (folder settings and default sort and size saved by
the portal chooser, custom themes, Omarchy theme changes) is parsed on the writer
and applied on the interface thread; the file monitors that notice those changes are
created and released on the monitors worker that also watches open folders, and once
each of them is live what it watches is read once more, so a change made while it was
starting is not missed (Omarchy follows the resolved theme directory, rebinding
its watch when a theme symlink points elsewhere); a change of this process's own that is still being saved
wins over an older save from elsewhere. Selection, history, the sort of
folders that are not remembered, filter query, transient theme
catalog filters, dialogs, and preview playback position remain window-local.
Pinned places, portal integration and other externally managed state have their
own stores and are not fields in the application preferences schema. Udiskie
encrypted-volume integration is stored in `$XDG_CONFIG_HOME/udiskie/config.yml`
plus `$XDG_DATA_HOME/strata/udiskie-install/state.toml`.
Synchronization between independently running application processes, or manual
external edits to `settings.toml` while Strata runs, is not supported by this
in-process binding mechanism. External edits are read on the next launch.
A save writes only the settings this process changed or repaired at startup since
its last successful save, over the file as it is on disk, so it keeps what another
process, such as the portal chooser, wrote to the other settings, and keys this
version does not know. Folder colors, custom icons, device labels, Send to
destinations and Omarchy variants are merged entry by entry. If two processes change
the same setting, the later save wins. A file that cannot be read or parsed is left
unchanged and the save fails.

## Per-folder sort and thumbnail size

With **Settings → General → Browsing → Remember sort and icon size per folder** on
(the default), choosing a sort field or direction in a pane, a List heading, or the
10xer sort chord remembers it for that folder only, and the Icons thumbnail slider
remembers the size for the folder it shows. Folders without their own value use
the default sort and default size; values are exact per folder and are not
inherited by subfolders. A value equal to the default is not stored, so the folder
follows later default changes. Every open column, tab, and window showing the same
folder updates live. View mode, density, hidden files, grouping, folders first, and
List column widths stay application-wide.

In Icons, **Ctrl+Alt++** (or **Ctrl+Alt+=**) and **Ctrl+Alt+−**, **Ctrl+Shift+wheel**,
and in 10xer mode **+** / **−** on the listing step the thumbnail size through 32,
36, 40, 44, 48, 56, 64, 72, 80, 96, 112, 128, 144, 160, 192, 224, and 256 px; a size
between two steps moves to the nearer one in that direction. **Ctrl+Alt+0** resets
it: a remembered folder follows the default again, any other folder returns to the
default, and with per-folder settings off the default returns to 64 px. These
change the size exactly as the slider does and change no size in Columns or List.
Each change briefly shows the new size in the bottom-right corner.

A pane's sort menu shows **Make this the default sort** when its sort differs from
the default and **Reset to default sort** when the folder has its own sort; the
thumbnail popover has the matching size actions. Making a value the default leaves
other folders' values alone, except those now equal to it, which are dropped. With
per-folder settings off, a new default drops nothing; turning them back on drops
the values that equal the defaults at that point. **Forget all folder settings**
clears every saved value.

Local folders are keyed by path and folders on removable drives by filesystem UUID
plus their path inside the drive, so values survive remounting; drives read through
a FUSE driver for local disks (`fuseblk`, such as ntfs-3g or exfat-fuse) count as
local. Remote shares, other FUSE filesystems, Trash, Recent, Camera Photos, and
paths that are not valid UTF-8 are not remembered: their sort stays with the open
column, and a size change lasts while the folder is shown. Renames and moves
performed in Strata, including undo and redo, carry a folder's values and those of
the folders inside it, following the Replace, Merge, Trash, Put back, and undo
rules of folder colors (see [Consumers and intentional
scopes](#consumers-and-intentional-scopes)); permanently deleting in Strata,
including a deletion moved to the background, drops them. Opening a local folder
that was deleted also drops them, but only when its parent still lists other
entries and holds no mount points, so the folders of a drive that is not mounted
yet keep their values. Changes made by other applications leave entries that the
least-recently-used limit of 5,000 folders eventually evicts; opening a folder
counts as using it, while re-sorting open columns does not.

Values live in `$XDG_STATE_HOME/strata/folder-views.toml` (normally
`~/.local/state/strata/folder-views.toml`), never in `settings.toml` or in the
folders themselves. Writes are batched, so a burst of changes such as a slider drag
is one write, and run on the settings writer, never on the click that made them;
pending changes are written when Strata or the portal chooser quits. Saves from another Strata process, such as
the portal chooser, are merged instead of overwritten, open views follow them, and
**Forget all folder settings** clears them too. Open views also follow default
sort and size changes saved by another Strata process. The file follows the
`settings.toml` recovery rules: a missing file starts empty; an unreadable file,
invalid TOML, or an unknown `version` uses no saved values, logs a warning, and is
never overwritten during that session, with a "Folder settings file can't be read"
notice on the first change in a browser window; valid TOML keeps its valid entries
and fields, clamps out-of-range sizes, and drops the rest on the next save; and a
failed write, or a save refused because the file has become unreadable, invalid
TOML, or an unknown `version` since startup, keeps changes in memory, retries on
the next change, and shows "Folder settings can't be saved" once per failure streak.

## Thumbnail workers

Under **Settings → General → Performance**, use the **Thumbnail workers** spin
button, the same control as Text size, to choose **1–16** concurrent decoders
across all windows. The initial default uses the available CPU count, capped at
four (two if CPU detection fails). Higher values can improve throughput but
consume more CPU and memory; they do not change image resolution or preview
playback.

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
and reset, including while an inline editor or Settings is open; **Ctrl+wheel**
does the same. Each briefly shows the new size in the bottom-right corner; changes
made in Settings do not. **Ctrl+Shift+wheel** resizes Icons thumbnails instead (see
[Per-folder sort and thumbnail size](#per-folder-sort-and-thumbnail-size)).
Over a PDF preview, **Ctrl+wheel** zooms the pages instead. With the PDF page
focused, **Ctrl+0** resets page zoom without changing the saved text size.

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

## Omarchy variants

Under **Settings → Appearance**, **Omarchy variant** sits below **Follow Omarchy**.
**Lighter** keeps file columns on the theme's regular background; **Darker** (the
default) moves them onto the darker pane color used by the sidebar, headers, and
preview; **Contrast** strengthens readability on the Darker layout. See [Themes](themes.md#omarchy-quattro) for color behavior.
The choice is remembered per Omarchy theme and saved under `[omarchy_variants]`
keyed by the theme name, for example `tokyo-night = "lighter"` or `"contrast"`;
Darker is the default and is not stored. It applies at startup and live across
windows and previews, and switching Omarchy themes restores that theme's own
choice. It is remembered but inactive when following is off. A legacy global
`omarchy_variant = "high_contrast"` migrates to Contrast for the Omarchy theme
active when Strata next starts; other legacy values are dropped, since earlier
versions always saved one. Missing or invalid saved values fall back to Darker
without resetting other preferences.

## Language

**Settings → General → Language** offers **Auto-detect** (the default) and
English, French, Hungarian, German, Spanish, Japanese, Brazilian Portuguese,
Korean, Vietnamese, Italian, Russian, Simplified Chinese, and Traditional Chinese.
Language names use their native spelling so the selector remains recognizable
after an accidental choice.
Save a manual choice as `language = "fr"` or `language = "zh-CN"`, for example,
or `language = "auto"` to follow the system again. Missing and unknown language
values recover to Auto-detect.

Changes are saved immediately and synchronize across Settings windows. Use
**Restart now**, or fully quit all Strata windows and reopen it, to apply the
new language. Until then, existing and newly opened application UI retains the
startup language. Changing language never rebuilds an active browser or resets
its navigation, selection, filters, or ongoing operations.

Auto-detect reads the colon-separated GNU `LANGUAGE` priority list, then the
message locale selected by the first nonempty `LC_ALL`, `LC_MESSAGES`, or `LANG`.
An explicit `C` or `POSIX` message locale means English and suppresses
`LANGUAGE`. Encoding and modifier suffixes are ignored; regional variants use
the supported base language (all Portuguese variants use Brazilian Portuguese,
`zh_TW`, `zh_HK`, `zh_MO`, and any `zh-Hant` variant use Traditional Chinese,
and every other Chinese locale, including bare `zh`, uses Simplified Chinese).
Unsupported languages fall back to English. Strata does not modify the desktop
locale or the environment inherited by launched programs. System/provider error
messages, externally supplied content, and toolkit-owned strings can therefore
remain in the system language. Filenames, paths, user-defined names, scripts,
and protocol identifiers are not translated.

Translations are compiled into the binary with `rust-i18n`; installing system
locale packages is not required. See [Internationalization](internationalization.md)
for catalog and contributor guidance.

## Interface renderer

**Settings → Appearance → Rendering** offers **GTK default** (initial choice) and **Cairo**.
Cairo avoids small-text artifacts on some systems but can use more CPU and reduce
rendering performance. The choice is saved as `interface_renderer = "cairo"` or
`"system"`. Use **Restart now** after changing the choice, or fully quit and reopen Strata,
including closing all windows. A caller-supplied `GSK_RENDERER` (even an empty value)
takes precedence. Cairo selected by Strata is a process environment setting,
so applications started by Strata may inherit it; it does not change desktop
or system-wide settings.

Independently of the renderer choice, Strata switches GTK's `gtk-font-rendering`
setting to **manual** for its own process. GTK's default automatic mode sizes GPU
glyph atlas slots from unhinted glyph extents while rasterizing with slight
hinting, so at some device pixel sizes (15px JetBrains Mono, for example) the GL
and Vulkan renderers drop the top pixel row of glyphs such as `T`
([GNOME/gtk#8395](https://gitlab.gnome.org/GNOME/gtk/-/issues/8395)). Manual mode
hints metrics as well, so the extents match the raster, and it follows the
desktop's antialiasing and hint-style settings. This is process-local and does
not change the desktop setting.

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

## Global search exclusions

In **Settings → General → Search & filtering → Global search exclusions**, enter
a folder name or absolute directory path and press **Enter** or **Add**, or use
**Browse…** to select a directory. The scrollable list below the form shows saved
exclusions, with a remove button for each. Folder names match case-insensitively
anywhere in the tree; directory paths are
case-sensitive and exclude only that subtree. `~/` expands to home. Folder names
are literal, not glob patterns. Built-in tool/cache exclusions remain in effect.
Rules are stored as `search_exclusions`; invalid saved rules are ignored. Root,
the entire home directory, relative paths, and paths containing `..` are rejected.
Changes apply the next time global search opens, across all windows.

## Search limits

In **Settings → General → Search & filtering**, **Search time limit** (10 s by
default, up to 15 min or No limit) and **Search entry limit** (200,000 by default,
up to 5,000,000 or No limit) bound every index walk. A walk stops when either is
reached and the results show the **Partial results** hint; hovering it lists the
reasons and, for a limit stop, points back to these settings. Longer limits let slow
or network mounts list every folder; the entry limit bounds index memory, since
every indexed entry keeps its path. Stored as `search_time_limit` (seconds) and
`search_entry_limit`; 0 means no limit, and other unlisted values round up to the
next choice. Changes apply to the next query in every window.

## Filter scope

In **Settings → General → Search & filtering**, **Include subfolders** is
on by default. Turn it off to match only immediate files and folders, without
redundant path subtitles. The choice applies to pane filtering in Columns, Icons,
and List views, not global search.
Changing it refreshes active filters across windows and is saved for next launch.
With it on, changes that other programs make in subfolders of the watched folders
reach active filters on F5 or when the filter is opened again; changes
in a watched folder show within about a second (see
[Filename patterns while filtering](keyboard-navigation.md#filename-patterns-while-filtering)).
[10xer mode](10xer-mode.md) does not use it: there **f** filters only the
current folder and **s** always searches below it.

## 10xer mode

In **Settings → General → Browsing**, **10xer mode** is off by default.
Its subtitle is **Opinionated keyboard-centric mode with Yazi-style navigation. Disables some features. Toggle with Ctrl+Shift+M.**
The footer shows a compact **10X** pill at the right, immediately before the item count, while the mode is on.
While the mode is on, the 10xer mode row and the F1 / `~` reference are labeled **(experimental feature, under active development)**. The footer pill shows only **10X**; the note is in its accessible description. Those surfaces list only the commands that currently run.
It hides window Search and pane Close/filter/refresh/sort chrome in
interactive browsers and the portal file chooser (window Close and chooser
Accept/Cancel stay; List column headings stay),
disables type-to-search and the conflicting default shortcuts, and installs
the Yazi-style map with footer prompts (`/` `?` `f` `s` `a` `r` `g Space`
`z`/`Z`), `g`-chord keycaps, and non-conflicting GUI keys (`Ctrl+C`/`X`/`V`,
`F2`, `F5`, `Delete`, …). Keyboard navigation stays in the Columns, List, and
Icons panes. The sidebar, window header, footer, and other controls outside
those panes stay pointer-operated. **`s`** is current-folder recursive fuzzy path
search and **`f`** filters the current folder with the same fuzzy terms;
**`S`** is unbound. Toggle with **Ctrl+Shift+M**, close the window with **Q**. The
preference is defined in `src/ui/preferences.rs`, saved as `tenxer_mode` in
`settings.toml`, and live-updates every window. Real mode transitions clear
prompts and credentials, chords, find highlights, hidden filters/search
(including forced recursion from **s**), and a keyboard folder peek. Leaving visual mode preserves the ordinary listing's filled selection;
leaving preview keyboard ownership restores its previous focusability without
closing the drawer. Default **Ctrl+F** then follows the saved **Include subfolders**
choice again. Initial preference binding applies chrome without
transition teardown or a file-list focus grab at disabled startup.
Context-menu shortcut hints and the F1 / `~` reference list the currently
active map (including the kept Ctrl/F-key conventions). **Type to search**,
**Keep arrows in file list**, and **Include subfolders** stay saved and
editable; while the mode is on they are unused and those rows show the subtitle
**Not used in 10xer mode.**
**Mirror columns selection** and **Single-click previews** keep applying, so in
Columns the cursor opens the folder or preview under it. **i** opens the next
Miller column without focusing it, or toggles the folder-peek popover in List
and Icons. See
[10xer mode](10xer-mode.md) for the keymap.

## Keymap

`settings.toml` stores keyboard shortcut overrides only, one table per map:
`[keymap.default]` and `[keymap.tenxer]` (10xer mode). Each entry names a
command by its stable id from `ui::keys::bindings` and lists its accelerators,
primary first; the primary is the one menus show, and all of them work. A file
without overrides has no `keymap` tables, so existing files save unchanged.

```toml
[keymap.default]
rename = ["F2"]
duplicate = []

[keymap.tenxer]
"go.home" = ["<Shift>h"]
toggle_sidebar = ["<Control>n", "F9"]
```

- An entry replaces that map's whole entry for the command; the other map keeps
  its own. An empty list unbinds the command, so its key reaches GTK. A
  structural key (`escape`, `activate`, `focus_next`, `focus_previous`, `up`,
  `down`, `left`, `right`) with an empty list leaves Strata's handlers, while
  GTK widgets keep their own handling of the physical key, and physical Escape
  always cancels.
- Accelerators use GTK's syntax, as `Binding::accelerator` writes them: Ctrl,
  Alt, Shift and Super only, Shift written out (`<Shift>g`; a letter's case does
  not matter), digits by digit-row position, keypad keys as the main keys they
  act as. A plain digit also accepts Shift, which AZERTY needs.
- A plain printable key (no Ctrl, Alt or Super) that an entry adds acts only
  where nothing owns typing: a focused text field or popover, and
  type-to-search while it is on, keep the key, as they keep structural
  alternates. Such a key on `shortcut_reference` opens the reference wherever
  it would not type, as 10xer's **~** does. Repeated accelerators in one entry
  count once.
- In `[keymap.tenxer]`, chord steps such as `go.home` match only while their
  trigger is pending. Shift with a Sort step reverses it on whatever key the
  step is bound to; a key bound to a step itself, such as `<Shift>a`, picks
  that step instead.
- A command a map ships no key for runs there only if one of its stages reads
  it. In `[keymap.default]` that is `open_with`, `move_to`, `copy_to`,
  `restore`, `invert_selection`, `search_subfolders`, `jump_visited`,
  `jump_recent`, `half_page_up`, `half_page_down`, and the `sort.*`,
  `copy_as.*` and `actions.*` steps, which act as plain keys there, without a
  trigger or Shift reversal, and the custom action slots `"custom_action.1"` to
  `"custom_action.10"`, which run the Nth action Settings → Actions lists; in
  `[keymap.tenxer]`, `duplicate`, `pin`,
  `jump_to_folder`, `arrow_scope` and, on a key that types nothing, `copy_path`. Any
  other such entry, such as `yank` or `"go.home"` in `[keymap.default]`, does
  nothing, and the load-time check below logs it.
  Digit ranges are not commands and keep their keys. Quote ids that contain a
  dot: unquoted, TOML reads `go.home = [..]` as a table, which is logged and
  left out.
- An uppercase letter without `<Shift>`, such as `<Control>E`, is logged once
  as a warning, because GTK reads it as the plain letter while labels show
  Shift+e as **E**; the entry still applies as the plain letter. Keys recorded
  in Settings are always saved in the lowercase, Shift-explicit form.
- An entry whose id names no command, that is not a list, or that holds an
  accelerator Strata cannot read is logged once with the file's path, kept
  verbatim in the file, and left out of the tables, so that command keeps its
  shipped keys and a newer version's entries survive an older version's save.
  A map that is not a table is dropped without touching the other map.
- Keys an entry adds beyond its shipped ones are checked against the rest of
  their table when the tables are first built (see
  [Checking keymap entries](keyboard-shortcuts.md#checking-keymap-entries)),
  with type-to-search as saved. Each finding is logged once with the file's
  path, the map, the id, the accelerator and the finding: an error, such as a
  key another command holds, as a warning, and a key that works only in some
  places as information. The entry still applies.
- `PreferenceManager` builds the tables (`keymap()`) on first use and rebuilds
  them on every change through `set_bindings`, `reset_binding` and `reset_map`;
  saving the shipped keys removes the entry, and `reset_map` keeps unknown ids.
  `keys::active_keymap()` reads these tables, so dispatch in every window and
  the portal file chooser follows the file loaded at startup before Settings
  opens. Text built once that shows shortcuts binds to `shown_keys` (10xer mode,
  whether type-to-search is active, since labels then leave out the plain keys a
  user added to the default map, and `keymap_revision()`) through
  `shortcut_reference::follow_keys`: footer
  hints, the F1 reference, sidebar keycaps, tooltips, Settings copy that names
  keys, and context-menu hints with their native `accel` attributes follow a
  change in every open window.
- Where 10xer key presses fall through to default-map stages (keys the 10xer
  stages do not claim, text fields, popovers, the file chooser's own keys,
  thumbnail resizing), the default
  table's overrides apply in both modes; a `[keymap.tenxer]` entry changes only
  what the 10xer stages claim.

**Settings → Keyboard** edits the same overrides: capturing a key appends it,
removing a chip or resetting writes the entry again, and saving the shipped keys
removes the entry. A captured key goes through the same check first, with
type-to-search as it is then: an error (the key never fires there, or stops
another key working) refuses it and keeps the capture waiting, and a warning
(the key works only in some places) saves it with a note under the row. Resets
and hand edits can still leave errors in a table; the page shows them under the
rows they affect. It reads the tables and the checker's findings when an
override or type-to-search changes, in every open window; see
[Settings keyboard page](keyboard-shortcuts.md#settings-keyboard-page) and, for
users, [Keymap](keymap.md).

Like other preferences, hand edits made while Strata runs are read on the next
launch.

## Content and Frame density

The Appearance menu's DENSITY section has two sliders, Frame above Content. Each one stores a logical px
value, with tick marks at its Compact and Airy values. The clickable labels under the
ticks snap to those presets. Any other value counts as Custom. A slider position
updates live in every window and is saved immediately; the menu stays open.

| Slider  | Preferences                                     | Range | Compact | Airy | Type             |
|---------|-------------------------------------------------|-------|---------|------|------------------|
| Frame   | `frame_density`, `frame_density_custom_px`      | 0–12  | 0       | 4    | `FrameDensity`   |
| Content | `browser_density`, `browser_density_custom_px`  | 0–16  | 4       | 8    | `BrowserDensity` |

`*_density` is `"compact"`, `"airy"`, or `"custom"`; `*_custom_px` is read only for
`"custom"`. Old settings files that have only `browser_density` keep loading. Both
types implement `browser_modes::DensityScale`, whose `from_px` maps a snap-point
value to its preset. As a result, a slider resting on 4 px is saved as Compact rather
than as Custom.

**Content** is the vertical margin of Columns and List rows. Compact and Airy keep the
static `density-compact`/`density-airy` rules in `style.css` and their 20/16 Icons
column caps. A Custom value sets `density-custom`.
`browser_modes::browser_density_custom_css` then extrapolates every preset property
linearly through those two rules, so the slider never jumps at a preset. Those
properties are row margins and minimum heights, plus the Icons grid padding, cell
margins, and card padding.

**Frame** is one extra vertical inset applied in lock step to the browser header,
tab strip, Icons/List pane header, Miller column headers, preview header, and
sidebar rows, so their heights stay aligned. Miller headers keep the two extra
bottom pixels that balance their top indicator. The same value spaces out the icon
buttons in those bars through `margin-left`. At 0 px, which is Compact, no rule is
emitted.

`ThemeManager::apply_density` loads both generated stylesheets into one provider
above the static stylesheet. It reloads only when either density changes, so
dragging a slider never reapplies the theme.

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
notifications, reentrant changes, listener cleanup, failed-write retries and
notices, symlinked settings files, chooser overrides, type-to-search keyboard
behavior, and synchronized media controls. Run GTK tests on the private display described in [testing](testing.md).
