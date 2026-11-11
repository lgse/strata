//! The desktop's settings from the XDG Settings portal, with their changes delivered on
//! Rune's loop: the text scale, the double-click interval, the drag threshold, the caret blink,
//! the animations switches and whether to remember recent files.
//!
//! Rune lacks, at the pinned rev (`2a44156`):
//!
//! - **Settings portal keys beyond `color-scheme`.** Rune reads only `org.freedesktop.appearance`
//!   `color-scheme` from `org.freedesktop.portal.Settings` (`platform/portal_appearance.rs`). It
//!   reads reduced motion from GSettings or `kdeglobals` instead, only when a window is created
//!   or focused, so it skips the portal and its change signals, and the portal's standard
//!   `reduced-motion` key. Draft
//!   [U12](https://github.com/wmfeht/strata-components/blob/main/docs/decisions/S5-upstream.md#u12),
//!   not filed.
//! - **The desktop's double-click interval.** Rune pairs presses within a fixed 0.5 s on Linux,
//!   and a windowed app can't change `Runtime::double_click_interval`. This module reads the
//!   desktop's interval, but only the app's own click handling can use it (a component that
//!   counts clicks itself); Rune's `on_double_press`, `.window_drag()`, word selection and
//!   `Drag::clicks` keep 0.5 s. Draft
//!   [U13](https://github.com/wmfeht/strata-components/blob/main/docs/decisions/S5-upstream.md#u13),
//!   not filed.
//! - Rune's text caret doesn't blink at `2a44156`, so the caret blink values have no consumer in
//!   Rune; they are for an app that draws its own caret.
//!
//! Work package: P5, `docs/work/phase-1-leaves/P5-desktop-settings.md`. The findings behind it
//! are `docs/decisions/S1-linux-window.md` (Q4, Q6).
//!
//! # The shape
//!
//! [`Watcher::start`] runs a thread ([`THREAD_NAME`]) with its own session-bus connection. It
//! subscribes to the portal's `SettingChanged` and to the portal's name owner, then reads every
//! key in one `ReadAll`, taking changes all the while (those that arrive during the read apply
//! after its answer, where they are newer than it). After that it only waits on the connection: no timers, no
//! polling. Its sink hears [`Event`]s in order: one
//! [`Event::Snapshot`], then an [`Event::Changed`] per change and an [`Event::Fallback`] the
//! first time each fallback happens. [`follow`] is the sink that applies them to the app's state
//! through `AppHandle::update_if`, so the view re-runs only when a value changed:
//!
//! ```ignore
//! struct State { desktop: DesktopSettings, /* … */ }
//!
//! fn main() {
//!     let app = App::new(AppSpec::new("App", (800.0, 600.0), State::default, view));
//!     // Keep the watcher until `run` returns; dropping it stops the thread.
//!     let watcher = settings::follow(&app.handle(), |s: &mut State| &mut s.desktop).ok();
//!     let first = watcher.as_ref().map(Watcher::snapshot).unwrap_or_default();
//!     app.init(move || State { desktop: first, ..State::default() }).run();
//! }
//! ```
//!
//! The values are plain data. An app hands them to its components itself: the scale to its
//! type ramp, the interval and threshold to its click handling, [`DesktopSettings::animations`]
//! to `Cx::reduced_motion` on every commit (Rune's override lasts one commit), falling back to
//! `cx.system_reduced_motion()` while neither animations key [is known](DesktopSettings::is_known).
//!
//! # The keys
//!
//! | Field | Portal namespace, key (type) | Default |
//! | --- | --- | --- |
//! | `text_scale` | `org.gnome.desktop.interface` `text-scaling-factor` (`d`) | 1.0 |
//! | `double_click` | `org.gnome.desktop.peripherals.mouse` `double-click` (`i`, ms) | 400 ms |
//! | `drag_threshold` | `org.gnome.desktop.peripherals.mouse` `drag-threshold` (`i`, px) | 8 |
//! | `cursor_blink` | `org.gnome.desktop.interface` `cursor-blink` (`b`) | on |
//! | `cursor_blink_time` | `org.gnome.desktop.interface` `cursor-blink-time` (`i`, ms) | 1200 ms |
//! | `cursor_blink_timeout` | `org.gnome.desktop.interface` `cursor-blink-timeout` (`i`, s) | 10 s |
//! | `enable_animations` | `org.gnome.desktop.interface` `enable-animations` (`b`) | on |
//! | `reduced_motion` | `org.freedesktop.appearance` `reduced-motion` (`u`, 1 reduces) | off |
//! | `remember_recent_files` | `org.gnome.desktop.privacy` `remember-recent-files` (`b`) | on |
//!
//! The scale must be a `d`, finite and above 0; integer keys take any integer type above 0.
//! Anything else keeps the key's value and reports [`Fallback::Invalid`]. A key the portal's
//! answer lacks has its default, isn't [known](DesktopSettings::is_known), and reports
//! [`Fallback::Unset`]. Each fallback is reported once; the module never prints, and the app
//! decides what to log.
//!
//! Values follow the order the bus delivered them in: an answer never overwrites a change
//! received after it, and a change received before an answer (which already holds it) doesn't
//! replay after it.
//!
//! # Without a portal
//!
//! - No session bus: the defaults and [`Fallback::NoBus`]. Losing the bus later reports it
//!   too, and the watcher stops with the values it had.
//! - No portal, one that can't be activated, or a failed read: the defaults and
//!   [`Fallback::NoPortal`]. The watcher reads again when a portal takes the name.
//! - A portal slower than [`Options::timeout`]: the defaults and [`Fallback::NoPortal`] at the
//!   timeout, and its answer as changes when it lands.
//! - The portal leaving the bus keeps its values, still known ([`Fallback::PortalLost`]). A new
//!   portal is read again and what differs arrives as changes; a key the new portal lacks goes
//!   back to its default and is no longer known.
//!
//! The portal is the only source. There is no fallback to GSettings, XSETTINGS (`Xft/DPI`,
//! `Net/DoubleClickTime`) or `kdeglobals`, so a session without a Settings portal (an X11 window
//! manager with only `xsettingsd`, say) gets the defaults, where GTK on X11 read XSETTINGS.
//! The `org.gnome.*` keys come from GTK-based backends: xdg-desktop-portal-gnome, and
//! xdg-desktop-portal-gtk (Hyprland's Settings backend; xdg-desktop-portal-hyprland has none).
//! xdg-desktop-portal-kde serves only `org.freedesktop.appearance` and `org.kde.kdeglobals.*`,
//! so on KDE without the GTK backend only `reduced-motion` is known.

mod fallback;
mod keys;
mod parse;
mod snapshot;
mod watcher;

#[cfg(test)]
mod tests;

use std::io;

use rune::AppHandle;

pub use fallback::Fallback;
pub use keys::{Key, Setting};
pub use snapshot::DesktopSettings;
pub use watcher::{Event, Options, THREAD_NAME, Watcher};

/// Starts a [`Watcher`] with the default [`Options`] whose snapshot and changes land in
/// `at(state)` on Rune's loop, through `AppHandle::update_if`: the view re-runs only when a
/// value changed. Fallbacks aren't delivered; an app that logs them uses [`Watcher::start`]
/// with its own sink. Keep the watcher as long as the app runs.
pub fn follow<S: 'static>(
    handle: &AppHandle<S>,
    at: fn(&mut S) -> &mut DesktopSettings,
) -> io::Result<Watcher> {
    let handle = handle.clone();
    Watcher::start(Options::default(), move |event| {
        let settings = match event {
            Event::Snapshot(settings) | Event::Changed { settings, .. } => settings,
            Event::Fallback(_) => return,
        };
        handle.update_if(move |state, _| {
            let slot = at(state);
            let changed = *slot != settings;
            *slot = settings;
            changed
        });
    })
}
