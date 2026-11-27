// SPDX-License-Identifier: MIT

use rune::Icon;
use rune_kit::glyphs;

pub mod catppuccin;
pub mod lucide;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct IconSet {
    /// The catalog's id (`lucide`, `catppuccin-mocha`, `catppuccin-latte`).
    pub name: &'static str,
    /// The catalog's icon for each semantic name.
    pub icon: fn(IconName) -> &'static Icon,
    /// The catalog's icon by its own name (`Icon::name`), or by an [`IconName::id`], or `None`.
    pub named: fn(&str) -> Option<&'static Icon>,
    /// Every icon in the catalog, for pickers. `named` finds each by its `Icon::name`.
    pub all: &'static [Icon],
}

/// The name and the icon count: the icons themselves are about 0.7 MB of SVG.
impl std::fmt::Debug for IconSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IconSet")
            .field("name", &self.name)
            .field("icons", &self.all.len())
            .finish_non_exhaustive()
    }
}

impl IconSet {
    /// The same icons under another catalog id.
    pub const fn renamed(self, name: &'static str) -> IconSet {
        IconSet { name, ..self }
    }
}

pub fn catalogs() -> [IconSet; 2] {
    [lucide::SET, catppuccin::SET]
}

pub const PLACEHOLDER: IconSet = IconSet {
    name: "placeholder",
    icon: placeholder,
    named: placeholder_named,
    all: &PLACEHOLDERS,
};

fn placeholder(name: IconName) -> &'static Icon {
    &PLACEHOLDERS[name as usize]
}

/// One of rune-kit's glyphs, under a placeholder name.
macro_rules! kit {
    ($glyph:ident) => {
        (glyphs::$glyph.view, glyphs::$glyph.body)
    };
}

macro_rules! line {
    ($body:literal) => {
        (
            16.0,
            concat!(
                r#"<g fill="none" stroke="currentColor" stroke-width="1" stroke-linecap="round" stroke-linejoin="round">"#,
                $body,
                "</g>"
            ),
        )
    };
}

macro_rules! icon_names {
    ($( $(#[$doc:meta])* $variant:ident = $id:literal => $art:expr, )*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[non_exhaustive]
        pub enum IconName {
            $( $(#[$doc])* $variant, )*
        }

        impl IconName {
            /// Every name, in declaration order.
            pub const ALL: &'static [IconName] = &[$(IconName::$variant),*];

            /// The name's id: snake_case, and the placeholder icon's `Icon::name`.
            pub const fn id(self) -> &'static str {
                match self {
                    $(IconName::$variant => $id,)*
                }
            }
        }

        static PLACEHOLDERS: [Icon; IconName::ALL.len()] = [
            $(Icon { name: $id, view: $art.0, body: $art.1 },)*
        ];

        fn placeholder_named(key: &str) -> Option<&'static Icon> {
            match key {
                $($id => Some(&PLACEHOLDERS[IconName::$variant as usize]),)*
                _ => None,
            }
        }
    };
}

icon_names! {
    // Entries.
    Folder = "folder" => kit!(FOLDER),
    FolderFilled = "folder_filled" => line!(r#"<path fill="currentColor" fill-opacity="0.92" d="M2.5 12.5v-9h4l1.5 1.5h5.5v7.5z"/>"#),
    FolderOpen = "folder_open" => kit!(FOLDER_OPEN),
    /// A file of no particular kind, and the fallback for an unknown icon key.
    File = "file" => kit!(FILE),
    /// A text document.
    FileText = "file_text" => kit!(FILE_TEXT),
    /// Source code.
    FileCode = "file_code" => kit!(FILE_CODE),
    FileImage = "file_image" => kit!(FILE_IMAGE),
    /// The symlink badge on an entry.
    Symlink = "symlink" => line!(r#"<path d="M4 12.5V9.5a3 3 0 0 1 3-3h5.5"/><path d="M10 4l2.5 2.5L10 9"/>"#),
    Lock = "lock" => kit!(LOCK),
    /// An archive: zip, tar and the like (C14b archive preview).
    Archive = "archive" => line!(r#"<rect x="2.5" y="3" width="11" height="3" rx="0.5"/><path d="M3.5 6v7h9V6M6.5 8.5h3"/>"#),
    Unlock = "unlock" => line!(r#"<rect x="3.5" y="7.5" width="9" height="6" rx="1"/><path d="M5.5 7.5V5a2.5 2.5 0 0 1 4.8-1"/>"#),

    /// The home folder.
    Home = "home" => line!(r#"<path d="M2.5 7.5L8 3l5.5 4.5"/><path d="M4 6.5v7h3v-3h2v3h3v-7"/>"#),
    /// The desktop folder.
    Desktop = "desktop" => line!(r#"<rect x="2.5" y="3" width="11" height="7.5" rx="1"/><path d="M8 10.5V13M5.5 13h5"/>"#),
    /// The documents folder.
    Documents = "documents" => line!(r#"<path d="M5.5 2.5h4.5l2.5 2.5v7.5h-7z"/><path d="M3.5 4.5v9.5h7"/>"#),
    /// The downloads folder.
    Downloads = "downloads" => line!(r#"<path d="M8 2.5v8M4.5 7L8 10.5L11.5 7M3 13.5h10"/>"#),
    /// The music folder.
    Music = "music" => line!(r#"<path d="M6 12V4l7-1.5v8"/><circle cx="4.5" cy="12" r="1.5"/><circle cx="11.5" cy="10.5" r="1.5"/>"#),
    /// The pictures folder.
    Pictures = "pictures" => line!(r#"<rect x="2.5" y="3" width="11" height="10" rx="1"/><path d="M2.5 11l3.5-3.5 3 3 2-2 2.5 2.5"/><circle cx="10.5" cy="6" r="1"/>"#),
    /// The videos folder.
    Videos = "videos" => line!(r#"<rect x="2.5" y="3.5" width="11" height="9" rx="1"/><path d="M7 6v4l3.5-2z"/>"#),
    /// Recent files.
    Recent = "recent" => line!(r#"<circle cx="8" cy="8" r="5.5"/><path d="M8 5v3l2 1.5"/>"#),
    /// Network locations and remote servers.
    Network = "network" => line!(r#"<circle cx="8" cy="8" r="5.5"/><ellipse cx="8" cy="8" rx="2.5" ry="5.5"/><path d="M2.5 8h11"/>"#),
    Computer = "computer" => line!(r#"<rect x="3.5" y="3.5" width="9" height="6.5" rx="0.5"/><path d="M1.5 12.5h13"/>"#),

    /// An internal or external drive.
    Drive = "drive" => line!(r#"<rect x="2" y="5" width="12" height="6" rx="1.5"/><circle cx="11.5" cy="8" r="0.75" fill="currentColor" stroke="none"/>"#),
    /// A USB stick.
    Usb = "usb" => line!(r#"<rect x="5" y="6.5" width="6" height="7" rx="1"/><path d="M6.5 6.5V2.5h3v4"/>"#),
    /// An optical disc.
    Optical = "optical" => line!(r#"<circle cx="8" cy="8" r="5.5"/><circle cx="8" cy="8" r="1.5"/>"#),
    /// A phone or camera.
    Phone = "phone" => line!(r#"<rect x="5" y="2" width="6" height="12" rx="1.2"/><path d="M7.5 12h1"/>"#),
    Eject = "eject" => line!(r#"<path d="M3.5 9.5L8 4l4.5 5.5z"/><path d="M3.5 12.5h9"/>"#),

    // Pins and the trash.
    Pin = "pin" => line!(r#"<path d="M6 2.5h4M7 2.5v4L4.5 9.5h7L9 6.5v-4M8 9.5v4"/>"#),
    Unpin = "unpin" => line!(r#"<path d="M6 2.5h4M7 2.5v4L4.5 9.5h7L9 6.5v-4M8 9.5v4"/><path d="M2.5 2.5l11 11"/>"#),
    Trash = "trash" => line!(r#"<path d="M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l0.7 9h5.6l0.7-9"/>"#),
    TrashFull = "trash_full" => line!(r#"<path d="M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l0.7 9h5.6l0.7-9"/><path d="M6.8 7v4.5M9.2 7v4.5"/>"#),
    /// A drag handle for reordering (C8 pins, C4 headings).
    Grip = "grip" => line!(r#"<circle cx="6" cy="4" r="0.9" fill="currentColor" stroke="none"/><circle cx="10" cy="4" r="0.9" fill="currentColor" stroke="none"/><circle cx="6" cy="8" r="0.9" fill="currentColor" stroke="none"/><circle cx="10" cy="8" r="0.9" fill="currentColor" stroke="none"/><circle cx="6" cy="12" r="0.9" fill="currentColor" stroke="none"/><circle cx="10" cy="12" r="0.9" fill="currentColor" stroke="none"/>"#),

    // Navigation.
    ChevronLeft = "chevron_left" => line!(r#"<path d="M9.5 4.5L6 8l3.5 3.5"/>"#),
    ChevronRight = "chevron_right" => line!(r#"<path d="M6.5 4.5L10 8l-3.5 3.5"/>"#),
    ChevronUp = "chevron_up" => kit!(CHEVRON_UP),
    ChevronDown = "chevron_down" => kit!(CHEVRON_DOWN),
    /// Go back in history.
    Back = "back" => line!(r#"<path d="M13 8H3M7 4L3 8l4 4"/>"#),
    /// Go forward in history.
    Forward = "forward" => line!(r#"<path d="M3 8h10M9 4l4 4-4 4"/>"#),
    Up = "up" => line!(r#"<path d="M8 13V3M4 7l4-4 4 4"/>"#),
    Down = "down" => line!(r#"<path d="M8 3v10M4 9l4 4 4-4"/>"#),
    /// Reload a listing.
    Refresh = "refresh" => line!(r#"<path d="M13 8a5 5 0 1 1-1.5-3.6"/><path d="M13 2.5V5h-2.5"/>"#),
    More = "more" => line!(r#"<circle cx="3.5" cy="8" r="0.9" fill="currentColor" stroke="none"/><circle cx="8" cy="8" r="0.9" fill="currentColor" stroke="none"/><circle cx="12.5" cy="8" r="0.9" fill="currentColor" stroke="none"/>"#),
    /// The application menu.
    Menu = "menu" => line!(r#"<path d="M2.5 4h11M2.5 8h11M2.5 12h11"/>"#),

    // Actions.
    Plus = "plus" => line!(r#"<path d="M8 3v10M3 8h10"/>"#),
    Minus = "minus" => line!(r#"<path d="M3 8h10"/>"#),
    Close = "close" => kit!(CLOSE),
    ClosePanel = "close_panel" => line!(r#"<rect x="2.5" y="3" width="11" height="10" rx="1"/><path d="M10 3v10M5.5 6l2 2-2 2"/>"#),
    Search = "search" => kit!(SEARCH),
    Filter = "filter" => line!(r#"<path d="M2.5 3.5h11l-4.2 5v4.5l-2.6-1.3V8.5z"/>"#),
    Check = "check" => kit!(CHECK_SMALL),
    Copy = "copy" => kit!(COPY),
    Cut = "cut" => line!(r#"<circle cx="4.5" cy="11.5" r="2"/><circle cx="11.5" cy="11.5" r="2"/><path d="M6 10L12 2.5M10 10L4 2.5"/>"#),
    Paste = "paste" => line!(r#"<rect x="3.5" y="3" width="9" height="11" rx="1"/><path d="M6 3V2h4v1"/><path d="M8 6v5M6 9l2 2 2-2"/>"#),
    FolderPlus = "folder_plus" => line!(r#"<path d="M2.5 12.5v-9h4l1.5 1.5h5.5v7.5z"/><path d="M8 7v4M6 9h4"/>"#),
    FilePlus = "file_plus" => line!(r#"<path d="M4 2.5h5.5l2.5 2.5v8.5H4z"/><path d="M8 7v4M6 9h4"/>"#),
    AppWindow = "app_window" => line!(r#"<rect x="2" y="3" width="12" height="10" rx="1"/><path d="M2 6h12"/>"#),
    CopyPlus = "copy_plus" => line!(r#"<rect x="5" y="5" width="8" height="8" rx="1"/><path d="M3 11V3h8"/><path d="M9 7v4M7 9h4"/>"#),
    FolderInput = "folder_input" => line!(r#"<path d="M2.5 12.5v-9h4l1.5 1.5h5.5v7.5z"/><path d="M5.5 9h5M8.5 7l2 2-2 2"/>"#),
    FolderOutput = "folder_output" => line!(r#"<path d="M2.5 12.5v-9h4l1.5 1.5h5.5v7.5z"/><path d="M10.5 9h-5M7.5 7l-2 2 2 2"/>"#),
    Send = "send" => line!(r#"<path d="M2.5 3.5L13.5 8l-11 4.5 2-4.5z"/><path d="M4.5 8h4"/>"#),
    PackagePlus = "package_plus" => line!(r#"<rect x="2.5" y="3" width="11" height="3" rx="0.5"/><path d="M3.5 6v7h9V6"/><path d="M8 7.5v4M6 9.5h4"/>"#),
    Extract = "extract" => line!(r#"<rect x="2.5" y="3" width="11" height="3" rx="0.5"/><path d="M3.5 6v7h9V6"/><path d="M8 7.5v4M6.3 9.8L8 11.5l1.7-1.7"/>"#),
    FolderArchive = "folder_archive" => line!(r#"<path d="M2.5 12.5v-9h4l1.5 1.5h5.5v7.5z"/><path d="M8 6.5v5M6.5 8.5h3"/>"#),
    Palette = "palette" => line!(r#"<circle cx="8" cy="8" r="5.5"/><circle cx="6" cy="6.5" r="0.8" fill="currentColor" stroke="none"/><circle cx="10" cy="6.5" r="0.8" fill="currentColor" stroke="none"/><circle cx="6.5" cy="10" r="0.8" fill="currentColor" stroke="none"/>"#),
    Route = "route" => line!(r#"<circle cx="4.5" cy="11.5" r="1.5"/><circle cx="11.5" cy="4.5" r="1.5"/><path d="M6 11.5h2.5a2 2 0 0 0 0-4H7.5a2 2 0 0 1 0-4H10"/>"#),
    FileType = "file_type" => line!(r#"<path d="M4 2.5h5.5l2.5 2.5v8.5H4z"/><path d="M6 8h4M8 8v3.5"/>"#),
    CircleX = "circle_x" => kit!(CIRCLE_X),
    Undo = "undo" => line!(r#"<path d="M5.5 6.5L3 9l2.5 2.5"/><path d="M3 9h7a3 3 0 0 1 0 6H7"/>"#),
    Terminal = "terminal" => line!(r#"<rect x="2" y="3" width="12" height="10" rx="1"/><path d="M4.5 6.5L6.5 8l-2 1.5M8 10.5h3.5"/>"#),
    Properties = "properties" => line!(r#"<path d="M3 4.5h10M3 8h10M3 11.5h10"/><circle cx="5.5" cy="4.5" r="1.2" fill="currentColor" stroke="none"/><circle cx="10.5" cy="8" r="1.2" fill="currentColor" stroke="none"/><circle cx="7" cy="11.5" r="1.2" fill="currentColor" stroke="none"/>"#),
    Rename = "rename" => line!(r#"<path d="M10.5 3l2.5 2.5-7 7H3.5V10z"/>"#),
    Open = "open" => line!(r#"<path d="M9 3h4v4M13 3L7.5 8.5"/><path d="M11 9.5V13H3V5h3.5"/>"#),
    Wrap = "wrap" => line!(r#"<path d="M2.5 4h11M2.5 8h9a2 2 0 0 1 0 4H8"/><path d="M9.5 10.5L8 12l1.5 1.5M2.5 12h3"/>"#),
    Source = "source" => line!(r#"<path d="M5.5 4.5L2 8l3.5 3.5M10.5 4.5L14 8l-3.5 3.5"/>"#),
    Rendered = "rendered" => kit!(EYE),
    Eye = "eye" => kit!(EYE),
    EyeOff = "eye_off" => kit!(EYE_OFF),

    // Views.
    ViewList = "view_list" => line!(r#"<path d="M5.5 4h8M5.5 8h8M5.5 12h8"/><circle cx="3" cy="4" r="0.8" fill="currentColor" stroke="none"/><circle cx="3" cy="8" r="0.8" fill="currentColor" stroke="none"/><circle cx="3" cy="12" r="0.8" fill="currentColor" stroke="none"/>"#),
    ViewColumns = "view_columns" => line!(r#"<rect x="2.5" y="3" width="11" height="10" rx="1"/><path d="M6.2 3v10M9.8 3v10"/>"#),
    ViewIcons = "view_icons" => line!(r#"<rect x="2.5" y="2.5" width="4.5" height="4.5" rx="0.8"/><rect x="9" y="2.5" width="4.5" height="4.5" rx="0.8"/><rect x="2.5" y="9" width="4.5" height="4.5" rx="0.8"/><rect x="9" y="9" width="4.5" height="4.5" rx="0.8"/>"#),
    ViewSettings = "view_settings" => line!(r#"<path d="M13.5 5H7M9 11H2.5"/><circle cx="5" cy="5" r="2"/><circle cx="11" cy="11" r="2"/>"#),
    Sidebar = "sidebar" => line!(r#"<rect x="2.5" y="3" width="11" height="10" rx="1"/><path d="M6.5 3v10"/>"#),
    SortAscending = "sort_ascending" => line!(r#"<path d="M5 13V3M2.5 5.5L5 3l2.5 2.5"/><path d="M9 5h2M9 8h3M9 11h4.5"/>"#),
    SortDescending = "sort_descending" => line!(r#"<path d="M5 3v10M2.5 10.5L5 13l2.5-2.5"/><path d="M9 5h4.5M9 8h3M9 11h2"/>"#),

    // Status.
    Info = "info" => line!(r#"<circle cx="8" cy="8" r="5.5"/><path d="M8 7.5V11"/><circle cx="8" cy="5.2" r="0.7" fill="currentColor" stroke="none"/>"#),
    Warning = "warning" => line!(r#"<path d="M8 2.5L14 13H2z"/><path d="M8 6.5v3"/><circle cx="8" cy="11.3" r="0.7" fill="currentColor" stroke="none"/>"#),
    Error = "error" => kit!(CIRCLE_X),
    Success = "success" => line!(r#"<circle cx="8" cy="8" r="5.5"/><path d="M5.5 8.2l1.8 1.8 3.2-3.8"/>"#),
    ListChecks = "list_checks" => line!(r#"<path d="M8 4h5.5M8 8h5.5M8 12h5.5M2.5 4.5l1 1 2-2M2.5 12.5l1 1 2-2"/>"#),
    Enter = "enter" => line!(r#"<path d="M13 3v4.5a2 2 0 0 1-2 2H3"/><path d="M6 6.5l-3 3 3 3"/>"#),
    Keyboard = "keyboard" => kit!(KEYBOARD),
    Help = "help" => line!(r#"<circle cx="8" cy="8" r="5.5"/><path d="M6.5 6.3a1.6 1.6 0 1 1 2.2 1.5c-0.5 0.2-0.7 0.6-0.7 1.1"/><circle cx="8" cy="11" r="0.7" fill="currentColor" stroke="none"/>"#),
    Clipboard = "clipboard" => line!(r#"<rect x="3.5" y="3" width="9" height="11" rx="1"/><path d="M6 3V2h4v1"/><path d="M6 7h4M6 9.5h4"/>"#),
    /// Settings.
    Settings = "settings" => line!(r#"<circle cx="8" cy="8" r="2"/><path d="M8 1.8v2M8 12.2v2M1.8 8h2M12.2 8h2M3.6 3.6l1.4 1.4M11 11l1.4 1.4M3.6 12.4L5 11M11 5l1.4-1.4"/>"#),

    /// Play.
    Play = "play" => line!(r#"<path d="M5 3.5v9l7.5-4.5z"/>"#),
    /// Pause.
    Pause = "pause" => line!(r#"<rect x="4" y="3.5" width="2.5" height="9" rx="0.5"/><rect x="9.5" y="3.5" width="2.5" height="9" rx="0.5"/>"#),
    /// Skip back: the previous chapter or track.
    SkipBack = "skip_back" => line!(r#"<path d="M4 3.5v9M12 3.5v9L6 8z"/>"#),
    /// Skip forward: the next chapter or track.
    SkipForward = "skip_forward" => line!(r#"<path d="M12 3.5v9M4 3.5v9L10 8z"/>"#),
    /// The volume.
    Volume = "volume" => line!(r#"<path d="M2.5 6v4H5l3.5 3V3L5 6z"/><path d="M10.5 6a3 3 0 0 1 0 4M12 4.5a5 5 0 0 1 0 7"/>"#),
    /// Muted.
    VolumeMute = "volume_mute" => line!(r#"<path d="M2.5 6v4H5l3.5 3V3L5 6z"/><path d="M10.5 6.5l3 3M13.5 6.5l-3 3"/>"#),
    /// Repeat or loop playback.
    Repeat = "repeat" => line!(r#"<path d="M3 7.5V6.5a2 2 0 0 1 2-2h8M11 2.5l2 2-2 2"/><path d="M13 8.5v1a2 2 0 0 1-2 2H3M5 13.5l-2-2 2-2"/>"#),
    /// Full screen.
    Fullscreen = "fullscreen" => line!(r#"<path d="M2.5 6V2.5H6M10 2.5h3.5V6M13.5 10v3.5H10M6 13.5H2.5V10"/>"#),

    /// Zoom in.
    ZoomIn = "zoom_in" => line!(r#"<circle cx="7" cy="7" r="4.5"/><path d="M13.5 13.5l-3.3-3.3M5 7h4M7 5v4"/>"#),
    /// Zoom out.
    ZoomOut = "zoom_out" => line!(r#"<circle cx="7" cy="7" r="4.5"/><path d="M13.5 13.5l-3.3-3.3M5 7h4"/>"#),
    Rotate = "rotate" => line!(r#"<path d="M3 8a5 5 0 1 0 1.5-3.6"/><path d="M3 2.5V5h2.5"/>"#),
    /// Fit to the window.
    Fit = "fit" => line!(r#"<rect x="5" y="5" width="6" height="6" rx="0.5"/><path d="M2.5 5V2.5H5M11 2.5h2.5V5M13.5 11v2.5H11M5 13.5H2.5V11"/>"#),

    /// Minimise the window.
    Minimize = "minimize" => line!(r#"<path d="M3.5 8h9"/>"#),
    /// Maximise the window, or maximise or restore it when its state is unknown.
    Maximize = "maximize" => line!(r#"<path d="M3 6V4a1 1 0 0 1 1-1h2M10 3h2a1 1 0 0 1 1 1v2M13 10v2a1 1 0 0 1-1 1h-2M6 13H4a1 1 0 0 1-1-1v-2"/>"#),
    /// Restore a maximised window.
    Restore = "restore" => line!(r#"<path d="M6 3v2a1 1 0 0 1-1 1H3M13 6h-2a1 1 0 0 1-1-1V3M3 10h2a1 1 0 0 1 1 1v2M10 13v-2a1 1 0 0 1 1-1h2"/>"#),
}
