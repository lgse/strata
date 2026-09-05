# File-manager tab behavior research

Research for issue #108, checked against upstream documentation and source on 2026-09-06.

## Baseline behavior

- GNOME's tab stack supports pointer reordering and detaching, plus keyboard switching and reordering. Its standard shortcuts include Ctrl+Tab, Ctrl+Shift+Tab, Ctrl+Page Up/Down, Ctrl+Shift+Page Up/Down, first/last-tab commands, and numbered selection. [AdwTabView](https://gnome.pages.gitlab.gnome.org/libadwaita/doc/main/class.TabView.html) · [AdwTabViewShortcuts](https://gnome.pages.gitlab.gnome.org/libadwaita/doc/main/flags.TabViewShortcuts.html)
- GNOME's tab bar scrolls when tabs do not fit, keeps pinned tabs visible, supports action widgets beside tabs, and accepts arbitrary drag-and-drop content on tabs. [AdwTabBar](https://gnome.pages.gitlab.gnome.org/libadwaita/doc/main/class.TabBar.html)
- Nautilus creates a regular new tab at the current tab's location; only a search location falls back to Home. [Nautilus `nautilus_window_new_tab`](https://gitlab.gnome.org/GNOME/nautilus/-/blob/f5716df9f801b330fc5f63dcf5a08ddad1e4420f/src/nautilus-window.c#L541)
- Nautilus supports new, restore closed, move left/right, move to a new window, close, and close other tabs. It stores navigation state when a tab closes. [Nautilus tab menu](https://gitlab.gnome.org/GNOME/nautilus/-/blob/f5716df9f801b330fc5f63dcf5a08ddad1e4420f/src/resources/ui/nautilus-window.blp#L54) · [Nautilus close/restore implementation](https://gitlab.gnome.org/GNOME/nautilus/-/blob/f5716df9f801b330fc5f63dcf5a08ddad1e4420f/src/nautilus-window.c#L647)
- Nautilus opens a selected item in a background tab and supports opening path-bar ancestors and back/forward destinations in tabs. [Nautilus file-view action](https://gitlab.gnome.org/GNOME/nautilus/-/blob/f5716df9f801b330fc5f63dcf5a08ddad1e4420f/src/nautilus-files-view.c#L1413) · [Nautilus path-bar action](https://gitlab.gnome.org/GNOME/nautilus/-/blob/f5716df9f801b330fc5f63dcf5a08ddad1e4420f/src/nautilus-pathbar.c#L163)
- Dolphin's new-tab command also duplicates the active location. It supports current/last insertion preferences, middle-click close, drag reordering, detaching, custom tab names, close other/left/right, and reopening recently closed tabs. [Dolphin tab widget](https://invent.kde.org/system/dolphin/-/blob/860f17b4e44bc34b9edb93fa36ee88a1b61e80de/src/dolphintabwidget.cpp#L192) · [Dolphin tab bar](https://invent.kde.org/system/dolphin/-/blob/860f17b4e44bc34b9edb93fa36ee88a1b61e80de/src/dolphintabbar.cpp#L154) · [Dolphin command reference](https://docs.kde.org/stable_kf6/en/dolphin/dolphin/command-reference.html)
- Dolphin allows adaptive, fixed, or full-width tabs and optional always-visible tab and close controls. [Dolphin tab settings](https://invent.kde.org/system/dolphin/-/blob/860f17b4e44bc34b9edb93fa36ee88a1b61e80de/src/settings/interface/folderstabssettingspage.cpp#L126)

## Strata priority

### Required baseline

- Independent live workspace state per tab.
- Current-location new tabs, with Home only as a fallback.
- Reliable select and close controls; middle-click close.
- Pointer and keyboard reordering.
- Ctrl+Tab and Ctrl+Shift+Tab switching in addition to Page Up/Down.
- Scrollable overflow and automatic reveal of the selected tab.
- Folder/file-list drops onto a tab, with delayed activation while hovering.
- Tab context menu: new tab, close, close others, and close tabs to the right.
- Full-path tooltips and duplicate-title disambiguation.

### Follow-up features

- Restore closed tabs with complete pane/navigation state.
- Move a tab to a new window and detach by dragging.
- Open folder tabs in the background.
- Tab overview/search when large tab counts make a strip insufficient.
- Optional pinned or renamed tabs only if real workflows justify them.
