// SPDX-License-Identifier: MIT

use serde::{Deserialize, Deserializer, Serialize};

/// Interface size in logical pixels; monitor scaling remains GTK's responsibility.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct TextSize(u32);

impl TextSize {
    pub const MIN: u32 = 8;
    pub const MAX: u32 = 48;
    pub const DEFAULT: Self = Self(13);

    pub fn new(pixels: u32) -> Self {
        Self(pixels.clamp(Self::MIN, Self::MAX))
    }

    pub fn root_font_px(self) -> u32 {
        self.0
    }

    pub fn stepped(self, delta: i32) -> Self {
        Self::new(self.0.saturating_add_signed(delta))
    }

    pub fn for_shortcut(
        self,
        key: gtk::gdk::Key,
        modifiers: gtk::gdk::ModifierType,
    ) -> Option<Self> {
        use gtk::gdk::{Key, ModifierType as Modifiers};
        if !modifiers.contains(Modifiers::CONTROL_MASK)
            || modifiers.intersects(Modifiers::ALT_MASK | Modifiers::SUPER_MASK)
        {
            return None;
        }
        match key {
            Key::plus | Key::equal | Key::KP_Add => Some(self.stepped(1)),
            Key::minus | Key::KP_Subtract => Some(self.stepped(-1)),
            Key::_0 | Key::KP_0 if !modifiers.contains(Modifiers::SHIFT_MASK) => {
                Some(Self::default())
            }
            _ => None,
        }
    }
}

impl Default for TextSize {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl<'de> Deserialize<'de> for TextSize {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Stored {
            Pixels(i64),
            Legacy(String),
        }
        Ok(match Stored::deserialize(deserializer)? {
            Stored::Pixels(value) => Self::new(value.clamp(0, i64::from(Self::MAX)) as u32),
            Stored::Legacy(value) => match value.as_str() {
                "small" => Self(11),
                "large" => Self(15),
                _ => Self::default(),
            },
        })
    }
}
