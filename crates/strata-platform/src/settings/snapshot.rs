//! The settings as plain data: what the app keeps in its state and hands to its components.

use std::time::Duration;

use super::keys::{Key, Setting};

/// The desktop's settings, with each key's default until the desktop gives a value.
///
/// Every field holds a usable value, so an app reads them directly. [`Self::is_known`] says
/// whether a value came from the desktop: an app with a second source for a setting (Rune's own
/// reduced-motion read, say) uses it for the keys the desktop left unknown.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DesktopSettings {
    /// Text size multiplier, not rounded: the app multiplies its text size by it once and
    /// rounds the result. 1.0 by default.
    pub text_scale: f64,
    /// Two presses closer than this pair into a double click. 400 ms by default, GNOME's and
    /// GTK's.
    pub double_click: Duration,
    /// How far, in logical pixels, a press travels before it is a drag. 8 by default.
    pub drag_threshold: u32,
    /// Whether text carets blink. On by default.
    pub cursor_blink: bool,
    /// One blink cycle, on and off. 1200 ms by default.
    pub cursor_blink_time: Duration,
    /// How long a caret blinks after the last key press before it stays on. 10 s by default.
    pub cursor_blink_timeout: Duration,
    /// GNOME's animations switch. On by default. [`Self::animations`] combines it with
    /// [`Self::reduced_motion`].
    pub enable_animations: bool,
    /// The portal's standard reduced-motion preference. Off by default.
    pub reduced_motion: bool,
    /// Whether the desktop keeps a list of recently used files. On by default.
    pub remember_recent_files: bool,
    known: u16,
}

impl Default for DesktopSettings {
    fn default() -> Self {
        DesktopSettings {
            text_scale: 1.0,
            double_click: Duration::from_millis(400),
            drag_threshold: 8,
            cursor_blink: true,
            cursor_blink_time: Duration::from_millis(1200),
            cursor_blink_timeout: Duration::from_secs(10),
            enable_animations: true,
            reduced_motion: false,
            remember_recent_files: true,
            known: 0,
        }
    }
}

impl DesktopSettings {
    /// Whether animations should run: GNOME's switch is on and the desktop doesn't ask for
    /// reduced motion.
    pub fn animations(&self) -> bool {
        self.enable_animations && !self.reduced_motion
    }

    /// Whether `key`'s value came from the desktop rather than its default. A key stops being
    /// known when a later read of the portal (after it restarted, say) lacks it: it goes back
    /// to its default. A portal that leaves the bus without a successor keeps its values, and
    /// they stay known.
    pub fn is_known(&self, key: Key) -> bool {
        self.known & key.bit() != 0
    }

    /// `key`'s current value.
    pub fn get(&self, key: Key) -> Setting {
        match key {
            Key::TextScale => Setting::TextScale(self.text_scale),
            Key::DoubleClick => Setting::DoubleClick(self.double_click),
            Key::DragThreshold => Setting::DragThreshold(self.drag_threshold),
            Key::CursorBlink => Setting::CursorBlink(self.cursor_blink),
            Key::CursorBlinkTime => Setting::CursorBlinkTime(self.cursor_blink_time),
            Key::CursorBlinkTimeout => Setting::CursorBlinkTimeout(self.cursor_blink_timeout),
            Key::EnableAnimations => Setting::EnableAnimations(self.enable_animations),
            Key::ReducedMotion => Setting::ReducedMotion(self.reduced_motion),
            Key::RememberRecentFiles => Setting::RememberRecentFiles(self.remember_recent_files),
        }
    }

    /// Puts `key` back to its default and marks it unknown. Returns whether anything changed.
    pub fn reset(&mut self, key: Key) -> bool {
        let before = *self;
        let default = DesktopSettings::default();
        self.set(default.get(key));
        self.known &= !key.bit();
        *self != before
    }

    /// Takes a value from the desktop and marks its key known. Returns whether anything
    /// changed: false for the value it already holds, and for an unusable value
    /// ([`Setting::is_valid`]), which it leaves out.
    pub fn set(&mut self, setting: Setting) -> bool {
        if !setting.is_valid() {
            return false;
        }
        let before = *self;
        match setting {
            Setting::TextScale(v) => self.text_scale = v,
            Setting::DoubleClick(v) => self.double_click = v,
            Setting::DragThreshold(v) => self.drag_threshold = v,
            Setting::CursorBlink(v) => self.cursor_blink = v,
            Setting::CursorBlinkTime(v) => self.cursor_blink_time = v,
            Setting::CursorBlinkTimeout(v) => self.cursor_blink_timeout = v,
            Setting::EnableAnimations(v) => self.enable_animations = v,
            Setting::ReducedMotion(v) => self.reduced_motion = v,
            Setting::RememberRecentFiles(v) => self.remember_recent_files = v,
        }
        self.known |= setting.key().bit();
        *self != before
    }
}
