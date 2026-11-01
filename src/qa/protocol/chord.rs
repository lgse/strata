// SPDX-License-Identifier: MIT

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Modifier {
    Ctrl,
    Shift,
    Alt,
    AltGr,
}

impl Modifier {
    pub(crate) fn parse(name: &str) -> Result<Self, String> {
        match name.to_ascii_lowercase().as_str() {
            "ctrl" => Ok(Self::Ctrl),
            "shift" => Ok(Self::Shift),
            "alt" => Ok(Self::Alt),
            "altgr" => Ok(Self::AltGr),
            _ => Err(format!("unknown modifier {name:?}")),
        }
    }

    /// The US keys a chord holds; `altgr` is the right Alt key,
    /// which AltGr layouts turn into AltGr.
    pub(crate) fn keysym_name(self) -> &'static str {
        match self {
            Self::Ctrl => "Control_L",
            Self::Shift => "Shift_L",
            Self::Alt => "Alt_L",
            Self::AltGr => "Alt_R",
        }
    }
}

/// A trailing `+` is the plus key itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Chord {
    pub(crate) modifiers: Vec<Modifier>,
    pub(crate) key: String,
}

impl Chord {
    pub(crate) fn parse(spelling: &str) -> Result<Self, String> {
        if spelling.is_empty() {
            return Err("an empty key".to_owned());
        }
        let parts: Vec<&str> = spelling.split('+').collect();
        let (key, modifiers) = match parts.split_last() {
            Some((&"", modifiers)) => ("+", modifiers),
            Some((key, modifiers)) => (*key, modifiers),
            None => return Err("an empty key".to_owned()),
        };
        let modifiers = modifiers
            .iter()
            .filter(|part| !part.is_empty())
            .map(|part| Modifier::parse(part))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            modifiers,
            key: key.to_owned(),
        })
    }
}
