//! [`PromptSpec`]: what one kind of footer prompt shows and which keys it keeps, as data, with
//! presets for Strata's twelve 10xer prompts (`src/ui/tenxer_mode.rs`, `Prompt`).

/// What Up and Down do in a prompt.
///
/// Used by: hosts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Arrows {
    /// They move the listing's cursor while the prompt keeps the keyboard
    /// ([`super::PromptIntent::Step`]): `/`, `?`, `f`, `s`, `create ›`.
    #[default]
    Listing,
    /// They choose another candidate, wrapping: the folder pickers and the history prompts.
    Candidates,
    /// They do nothing: `rename ›`, whose targets were captured when it opened.
    Swallow,
}

/// The words under a candidate list: its keys and what they do. Shown as keycaps with words.
///
/// Used by: hosts.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Legend {
    /// Up and Down: "Choose".
    pub choose: String,
    /// Tab, where it writes the chosen candidate into the prompt: "Complete".
    pub complete: Option<String>,
    /// Enter: "Open", "Move", "Copy" or "Extract".
    pub enter: String,
    /// Escape: "Cancel".
    pub cancel: String,
}

impl Legend {
    /// The legend with Enter doing `enter`, and Tab completing when `complete`.
    pub fn new(enter: &str, complete: bool) -> Self {
        Self {
            choose: "Choose".to_owned(),
            complete: complete.then(|| "Complete".to_owned()),
            enter: enter.to_owned(),
            cancel: "Cancel".to_owned(),
        }
    }
}

/// One kind of footer prompt: its label, its accessible name, and which keys it keeps.
/// Strata's words are the defaults; a translated host replaces them.
///
/// Used by: hosts.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PromptSpec {
    /// The label before the field, in the accent: `filter:`, `go ›`, or a keycap (`/`).
    pub label: String,
    /// The field's accessible name: "Filter this listing".
    pub name: String,
    /// Whether Ctrl+Z and its kin undo. Off for `go ›`, so undo history can never bring back a
    /// typed credential.
    pub undo: bool,
    /// What Up and Down do.
    pub arrows: Arrows,
    /// The candidate list's legend, for a prompt with [`Arrows::Candidates`]; Tab completes
    /// only where it says so.
    pub legend: Option<Legend>,
    /// Whether the hint shows the chosen candidate's place ("2 of 5") when the host gives none:
    /// the history prompts.
    pub position: bool,
}

impl PromptSpec {
    fn new(label: &str, name: &str, arrows: Arrows) -> Self {
        Self {
            label: label.to_owned(),
            name: name.to_owned(),
            undo: true,
            arrows,
            legend: None,
            position: false,
        }
    }

    fn picker(label: &str, name: &str, enter: &str) -> Self {
        Self {
            legend: Some(Legend::new(enter, true)),
            ..Self::new(label, name, Arrows::Candidates)
        }
    }

    fn history(label: &str, name: &str) -> Self {
        Self {
            legend: Some(Legend::new("Open", false)),
            position: true,
            ..Self::new(label, name, Arrows::Candidates)
        }
    }

    /// `/`: find the next name containing the text.
    pub fn find() -> Self {
        Self::new("/", "Find in this listing", Arrows::Listing)
    }

    /// `?`: find the previous one.
    pub fn find_backward() -> Self {
        Self::new("?", "Find backward in this listing", Arrows::Listing)
    }

    /// `f`: hide the items that don't match as you type.
    pub fn filter() -> Self {
        Self::new("filter:", "Filter this listing", Arrows::Listing)
    }

    /// `s`: search paths below the folder.
    pub fn search() -> Self {
        Self::new(
            "search:",
            "Search this folder and its subfolders",
            Arrows::Listing,
        )
    }

    /// `g Space`: go to a typed path or URI, with folders listed as you type. No undo.
    pub fn go() -> Self {
        Self {
            undo: false,
            ..Self::picker("go \u{203a}", "Go to a path or URI", "Open")
        }
    }

    /// `z`: a folder from the navigation history, ranked.
    pub fn jump() -> Self {
        Self::history("jump \u{203a}", "Jump to a visited folder")
    }

    /// `Z`: a folder from the navigation history, last visit first.
    pub fn recent() -> Self {
        Self::history("recent \u{203a}", "Jump to a recently visited folder")
    }

    /// `a`: create a file, or a folder when the name ends in `/`.
    pub fn create() -> Self {
        Self::new(
            "create \u{203a}",
            "Create a file, or a folder ending in /",
            Arrows::Listing,
        )
    }

    /// `r`, F2: rename the cursor item.
    pub fn rename() -> Self {
        Self::new(
            "rename \u{203a}",
            "Rename the focused item",
            Arrows::Swallow,
        )
    }

    /// `M`: move the targets to a folder.
    pub fn move_to() -> Self {
        Self::picker("move to \u{203a}", "Move the selection to a folder", "Move")
    }

    /// `C`: copy the targets to a folder.
    pub fn copy_to() -> Self {
        Self::picker("copy to \u{203a}", "Copy the selection to a folder", "Copy")
    }

    /// `; E`: extract the archive to a folder.
    pub fn extract_to() -> Self {
        Self::picker(
            "extract to \u{203a}",
            "Extract the archive to a folder",
            "Extract",
        )
    }
}
