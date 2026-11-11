//! The keys the module reads, in one table: each key's namespace and name live here only.

use std::time::Duration;

/// GNOME's interface settings, which every GTK-based Settings portal backend serves.
const INTERFACE: &str = "org.gnome.desktop.interface";
const MOUSE: &str = "org.gnome.desktop.peripherals.mouse";
const PRIVACY: &str = "org.gnome.desktop.privacy";
/// The portal's own standard namespace (`color-scheme` there is Rune's; this module reads
/// only `reduced-motion`).
const APPEARANCE: &str = "org.freedesktop.appearance";

/// The namespaces the watcher reads and follows.
pub(super) const NAMESPACES: [&str; 4] = [INTERFACE, MOUSE, PRIVACY, APPEARANCE];

/// One desktop setting the module reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum Key {
    /// `org.gnome.desktop.interface` `text-scaling-factor` (`d`).
    TextScale,
    /// `org.gnome.desktop.peripherals.mouse` `double-click` (`i`, milliseconds).
    DoubleClick,
    /// `org.gnome.desktop.peripherals.mouse` `drag-threshold` (`i`, pixels).
    DragThreshold,
    /// `org.gnome.desktop.interface` `cursor-blink` (`b`).
    CursorBlink,
    /// `org.gnome.desktop.interface` `cursor-blink-time` (`i`, milliseconds per cycle).
    CursorBlinkTime,
    /// `org.gnome.desktop.interface` `cursor-blink-timeout` (`i`, seconds).
    CursorBlinkTimeout,
    /// `org.gnome.desktop.interface` `enable-animations` (`b`).
    EnableAnimations,
    /// `org.freedesktop.appearance` `reduced-motion` (`u`: 1 reduces, anything else doesn't).
    ReducedMotion,
    /// `org.gnome.desktop.privacy` `remember-recent-files` (`b`).
    RememberRecentFiles,
}

impl Key {
    /// Every key, in the order a re-read reports its changes.
    pub const ALL: [Key; 9] = [
        Key::TextScale,
        Key::DoubleClick,
        Key::DragThreshold,
        Key::CursorBlink,
        Key::CursorBlinkTime,
        Key::CursorBlinkTimeout,
        Key::EnableAnimations,
        Key::ReducedMotion,
        Key::RememberRecentFiles,
    ];

    /// The portal namespace the key lives in.
    pub const fn namespace(self) -> &'static str {
        match self {
            Key::TextScale
            | Key::CursorBlink
            | Key::CursorBlinkTime
            | Key::CursorBlinkTimeout
            | Key::EnableAnimations => INTERFACE,
            Key::DoubleClick | Key::DragThreshold => MOUSE,
            Key::RememberRecentFiles => PRIVACY,
            Key::ReducedMotion => APPEARANCE,
        }
    }

    /// The key's name in its namespace.
    pub const fn name(self) -> &'static str {
        match self {
            Key::TextScale => "text-scaling-factor",
            Key::DoubleClick => "double-click",
            Key::DragThreshold => "drag-threshold",
            Key::CursorBlink => "cursor-blink",
            Key::CursorBlinkTime => "cursor-blink-time",
            Key::CursorBlinkTimeout => "cursor-blink-timeout",
            Key::EnableAnimations => "enable-animations",
            Key::ReducedMotion => "reduced-motion",
            Key::RememberRecentFiles => "remember-recent-files",
        }
    }

    /// The key a portal names `namespace` `name`, if the module reads it.
    pub fn lookup(namespace: &str, name: &str) -> Option<Key> {
        Key::ALL
            .into_iter()
            .find(|k| k.namespace() == namespace && k.name() == name)
    }

    pub(super) const fn bit(self) -> u16 {
        1 << self as u16
    }
}

/// One typed value, as [`DesktopSettings::set`](super::DesktopSettings::set) takes it and a
/// change reports it.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum Setting {
    TextScale(f64),
    DoubleClick(Duration),
    DragThreshold(u32),
    CursorBlink(bool),
    CursorBlinkTime(Duration),
    CursorBlinkTimeout(Duration),
    EnableAnimations(bool),
    ReducedMotion(bool),
    RememberRecentFiles(bool),
}

impl Setting {
    pub const fn key(self) -> Key {
        match self {
            Setting::TextScale(_) => Key::TextScale,
            Setting::DoubleClick(_) => Key::DoubleClick,
            Setting::DragThreshold(_) => Key::DragThreshold,
            Setting::CursorBlink(_) => Key::CursorBlink,
            Setting::CursorBlinkTime(_) => Key::CursorBlinkTime,
            Setting::CursorBlinkTimeout(_) => Key::CursorBlinkTimeout,
            Setting::EnableAnimations(_) => Key::EnableAnimations,
            Setting::ReducedMotion(_) => Key::ReducedMotion,
            Setting::RememberRecentFiles(_) => Key::RememberRecentFiles,
        }
    }

    /// Whether the value can be used: a scale factor that is finite and above 0, and durations
    /// and the drag threshold above 0. GNOME's schemas bound some of these more tightly, but a
    /// value inside that range is the desktop's to choose, so only unusable ones are refused.
    pub fn is_valid(self) -> bool {
        match self {
            Setting::TextScale(f) => f.is_finite() && f > 0.0,
            Setting::DoubleClick(d)
            | Setting::CursorBlinkTime(d)
            | Setting::CursorBlinkTimeout(d) => !d.is_zero(),
            Setting::DragThreshold(px) => px > 0,
            Setting::CursorBlink(_)
            | Setting::EnableAnimations(_)
            | Setting::ReducedMotion(_)
            | Setting::RememberRecentFiles(_) => true,
        }
    }
}
