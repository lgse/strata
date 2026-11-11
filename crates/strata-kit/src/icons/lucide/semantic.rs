//! Which icon each semantic name draws, the same in every catalog: Strata's own choice where
//! Strata has one (read at lgse/strata `upstream/main` 35ee677ab: its context menus, sidebar
//! and `src/ui/browser/entry.rs`; W2d's names and the device and root rulings at fb7eb0c4e,
//! whose icons are the same), else the closest Lucide icon.
//!
//! The match is exhaustive, so a name W2 adds fails to build until it has an icon here, and a
//! target that leaves `icons/` fails to build too.

use super::super::IconName;
use super::catalog::index as i;

pub(super) const fn index(name: IconName) -> usize {
    match name {
        // Entries.
        IconName::Folder => i::STRATA_FOLDER,
        IconName::FolderOpen => i::STRATA_FOLDER_OPEN,
        // Strata draws a file of no known type as its document icon.
        IconName::File => i::STRATA_FILE_TEXT,
        IconName::FileText => i::STRATA_FILE_TEXT,
        IconName::FileCode => i::STRATA_FILE_CODE,
        IconName::FileImage => i::STRATA_IMAGE,
        IconName::Symlink => i::STRATA_KIT_LINK,
        IconName::Lock => i::STRATA_LOCK,
        IconName::Archive => i::STRATA_FILE_ARCHIVE,
        // Strata draws a volume's current state, not the action: C8's Unlock action (on a
        // locked volume) shows `Lock`, and its Lock action `Unlock`, as its Unpin shows the pin.
        IconName::Unlock => i::STRATA_LOCK_OPEN,

        // Places.
        IconName::Home => i::STRATA_HOUSE,
        // Strata's Desktop place is a plain folder (`src/ui/window/places.rs`).
        IconName::Desktop => i::STRATA_FOLDER,
        IconName::Documents => i::STRATA_FILE_TEXT,
        IconName::Downloads => i::STRATA_DOWNLOAD,
        IconName::Music => i::STRATA_MUSIC_2,
        IconName::Pictures => i::STRATA_IMAGE,
        IconName::Videos => i::STRATA_VIDEO,
        IconName::Recent => i::STRATA_CLOCK,
        IconName::Network => i::STRATA_NETWORK,
        // Strata's root crumb (`src/ui/browser/location/breadcrumbs.rs`); its monitor is the
        // Desktop chip in Settings and a custom action icon, found by name.
        IconName::Computer => i::STRATA_HARD_DRIVE,

        // Devices: Strata draws every device row (mount, volume or drive, a USB stick, a disc
        // or a phone alike) as a hard drive (`src/ui/window/sidebar_devices.rs`). Its disc is
        // for disk-image files and its smartphone for file-provider integrations.
        IconName::Drive => i::STRATA_HARD_DRIVE,
        IconName::Usb => i::STRATA_HARD_DRIVE,
        IconName::Optical => i::STRATA_HARD_DRIVE,
        IconName::Phone => i::STRATA_HARD_DRIVE,
        IconName::Eject => i::STRATA_EJECT,

        // Pins and the trash: Strata's Unpin shows the pin (the current state), and its trash
        // has one icon.
        IconName::Pin => i::STRATA_PIN,
        IconName::Unpin => i::STRATA_PIN,
        IconName::Trash => i::STRATA_TRASH,
        IconName::TrashFull => i::STRATA_TRASH,
        IconName::Grip => i::STRATA_KIT_GRIP_VERTICAL,

        // Navigation.
        IconName::ChevronLeft => i::STRATA_KIT_CHEVRON_LEFT,
        IconName::ChevronRight => i::STRATA_CHEVRON_RIGHT,
        IconName::ChevronUp => i::STRATA_KIT_CHEVRON_UP,
        IconName::ChevronDown => i::STRATA_CHEVRON_DOWN,
        IconName::Back => i::STRATA_ARROW_LEFT,
        IconName::Forward => i::STRATA_ARROW_RIGHT,
        // Also C9's find bar, as Strata's `src/ui/preview/find.rs` draws it.
        IconName::Up => i::STRATA_ARROW_UP,
        IconName::Down => i::STRATA_ARROW_DOWN,
        IconName::Refresh => i::STRATA_REFRESH,
        IconName::More => i::STRATA_KIT_ELLIPSIS,
        IconName::Menu => i::STRATA_KIT_MENU,

        // Actions.
        IconName::Plus => i::STRATA_PLUS,
        // Strata's spin buttons draw `strata-minus-symbolic`, the same shape filled for GTK's
        // recolouring; Rune tints the stroked one.
        IconName::Minus => i::STRATA_MINUS,
        IconName::Close => i::STRATA_X,
        // `src/ui/preview/drawer/header.rs`.
        IconName::ClosePanel => i::STRATA_PANEL_RIGHT_CLOSE,
        IconName::Search => i::STRATA_SEARCH,
        IconName::Filter => i::STRATA_FUNNEL,
        IconName::Check => i::STRATA_CHECK,
        IconName::Copy => i::STRATA_COPY,
        IconName::Cut => i::STRATA_SCISSORS,
        IconName::Paste => i::STRATA_CLIPBOARD_PASTE,
        IconName::FolderPlus => i::STRATA_FOLDER_PLUS,
        IconName::Extract => i::STRATA_PACKAGE_OPEN,
        IconName::Terminal => i::STRATA_TERMINAL,
        IconName::Properties => i::STRATA_INFO,
        IconName::Rename => i::STRATA_PENCIL,
        IconName::Open => i::STRATA_EXTERNAL_LINK,
        IconName::Wrap => i::STRATA_WRAP_TEXT,
        // C9's toggle, as Strata's `src/ui/preview/document.rs` draws it.
        IconName::Source => i::STRATA_FILE_CODE,
        IconName::Rendered => i::STRATA_FILE_TEXT,
        IconName::Eye => i::STRATA_EYE,
        IconName::EyeOff => i::STRATA_EYE_OFF,

        // Views.
        IconName::ViewList => i::STRATA_LIST,
        IconName::ViewColumns => i::STRATA_COLUMNS,
        IconName::ViewIcons => i::STRATA_ICONS,
        IconName::Sidebar => i::STRATA_PANEL_LEFT_SYMBOLIC,
        // List headings, as Strata's `src/ui/browser_modes/list_headings.rs` draws them.
        IconName::SortAscending => i::STRATA_ARROW_UP,
        IconName::SortDescending => i::STRATA_ARROW_DOWN,

        // Status.
        IconName::Info => i::STRATA_INFO,
        IconName::Warning => i::STRATA_TRIANGLE_ALERT,
        // Strata's dialogs (C15, `src/ui/modal/messages.rs`) and jobs (C16, `src/ui/jobs.rs`).
        // Its circled check and cross are Settings-row status icons, not these.
        IconName::Error => i::STRATA_X,
        IconName::Success => i::STRATA_CHECK,
        // `src/ui/jobs.rs`'s header bezel.
        IconName::ListChecks => i::STRATA_LIST_CHECKS,
        // `src/ui/search/panel.rs`'s open hint.
        IconName::Enter => i::STRATA_CORNER_DOWN_LEFT,
        IconName::Keyboard => i::STRATA_KEYBOARD,
        IconName::Help => i::STRATA_KIT_CIRCLE_QUESTION_MARK,
        IconName::Clipboard => i::STRATA_KIT_CLIPBOARD,
        IconName::Settings => i::STRATA_SETTINGS,

        // Media.
        IconName::Play => i::STRATA_PLAY,
        IconName::Pause => i::STRATA_PAUSE,
        IconName::SkipBack => i::STRATA_SKIP_BACK,
        IconName::SkipForward => i::STRATA_SKIP_FORWARD,
        IconName::Volume => i::STRATA_VOLUME_2,
        IconName::VolumeMute => i::STRATA_VOLUME_X,
        IconName::Repeat => i::STRATA_KIT_REPEAT,
        IconName::Fullscreen => i::STRATA_MAXIMIZE,

        // Viewers.
        IconName::ZoomIn => i::STRATA_KIT_ZOOM_IN,
        IconName::ZoomOut => i::STRATA_KIT_ZOOM_OUT,
        IconName::Rotate => i::STRATA_KIT_ROTATE_CW,
        IconName::Fit => i::STRATA_KIT_SCAN,

        // Window buttons: Strata's header (`src/ui/window/composition/layout.rs`), whose
        // Restore shows Lucide's minimize.
        IconName::Minimize => i::STRATA_MINUS,
        IconName::Maximize => i::STRATA_MAXIMIZE,
        IconName::Restore => i::STRATA_MINIMIZE,
    }
}
