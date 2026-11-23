// SPDX-License-Identifier: MIT

//! The settings as plain data: what the app keeps in its state and hands to its components.

use std::time::Duration;

use super::keys::{Key, Setting};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DesktopSettings {
    pub text_scale: f64,
    pub double_click: Duration,
    /// How far, in logical pixels, a press travels before it is a drag. 8 by default.
    pub drag_threshold: u32,
    /// Whether text carets blink. On by default.
    pub cursor_blink: bool,
    /// One blink cycle, on and off. 1200 ms by default.
    pub cursor_blink_time: Duration,
    /// How long a caret blinks after the last key press before it stays on. 10 s by default.
    pub cursor_blink_timeout: Duration,
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
    pub fn animations(&self) -> bool {
        self.enable_animations && !self.reduced_motion
    }

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
