//! The words assistive technology hears for the picker's parts. Strata translates them.

/// The parts' names and value texts. [`Default`] is English, after GTK's colour editor.
#[derive(Clone, Copy, Debug)]
pub struct PickerWords {
    /// The plane's name: "Saturation and value" (GTK's).
    pub plane: &'static str,
    /// The hue strip's name: "Hue".
    pub hue: &'static str,
    /// The alpha strip's name: "Alpha".
    pub alpha: &'static str,
    /// The hex field's name: "Hex color".
    pub hex: &'static str,
    /// The plane's value text from whole percents of saturation and value:
    /// "Saturation 80%, value 60%".
    pub plane_value: fn(u32, u32) -> String,
    /// The hue strip's value text from whole degrees: "200°".
    pub hue_value: fn(u32) -> String,
    /// The alpha strip's value text from a whole percent: "50%".
    pub alpha_value: fn(u32) -> String,
}

impl Default for PickerWords {
    fn default() -> PickerWords {
        PickerWords {
            plane: "Saturation and value",
            hue: "Hue",
            alpha: "Alpha",
            hex: "Hex color",
            plane_value: |s, v| format!("Saturation {s}%, value {v}%"),
            hue_value: |h| format!("{h}\u{b0}"),
            alpha_value: |a| format!("{a}%"),
        }
    }
}
