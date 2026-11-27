// SPDX-License-Identifier: MIT

use super::super::IconName;
use super::catalog::index as i;

pub(super) const fn index(name: IconName) -> usize {
    match name {
        // Entries.
        IconName::Folder => i::STRATA_FOLDER,
        IconName::FolderFilled => i::STRATA_KIT_FOLDER_FILLED,
        IconName::FolderOpen => i::STRATA_FOLDER_OPEN,
        // Strata draws a file of no known type as its document icon.
        IconName::File => i::STRATA_FILE_TEXT,
        IconName::FileText => i::STRATA_FILE_TEXT,
        IconName::FileCode => i::STRATA_FILE_CODE,
        IconName::FileImage => i::STRATA_IMAGE,
        IconName::Symlink => i::STRATA_KIT_LINK,
        IconName::Lock => i::STRATA_LOCK,
        IconName::Archive => i::STRATA_FILE_ARCHIVE,
        IconName::Unlock => i::STRATA_LOCK_OPEN,

        // Places.
        IconName::Home => i::STRATA_HOUSE,
        IconName::Desktop => i::STRATA_FOLDER,
        IconName::Documents => i::STRATA_FILE_TEXT,
        IconName::Downloads => i::STRATA_DOWNLOAD,
        IconName::Music => i::STRATA_MUSIC_2,
        IconName::Pictures => i::STRATA_IMAGE,
        IconName::Videos => i::STRATA_VIDEO,
        IconName::Recent => i::STRATA_CLOCK,
        IconName::Network => i::STRATA_NETWORK,
        IconName::Computer => i::STRATA_HARD_DRIVE,

        IconName::Drive => i::STRATA_HARD_DRIVE,
        IconName::Usb => i::STRATA_HARD_DRIVE,
        IconName::Optical => i::STRATA_HARD_DRIVE,
        IconName::Phone => i::STRATA_HARD_DRIVE,
        IconName::Eject => i::STRATA_EJECT,

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
        IconName::Up => i::STRATA_ARROW_UP,
        IconName::Down => i::STRATA_ARROW_DOWN,
        IconName::Refresh => i::STRATA_REFRESH,
        IconName::More => i::STRATA_KIT_ELLIPSIS,
        IconName::Menu => i::STRATA_KIT_MENU,

        // Actions.
        IconName::Plus => i::STRATA_PLUS,
        IconName::Minus => i::STRATA_MINUS,
        IconName::Close => i::STRATA_X,
        IconName::ClosePanel => i::STRATA_PANEL_RIGHT_CLOSE,
        IconName::Search => i::STRATA_SEARCH,
        IconName::Filter => i::STRATA_FUNNEL,
        IconName::Check => i::STRATA_CHECK,
        IconName::Copy => i::STRATA_COPY,
        IconName::Cut => i::STRATA_SCISSORS,
        IconName::Paste => i::STRATA_CLIPBOARD_PASTE,
        IconName::FolderPlus => i::STRATA_FOLDER_PLUS,
        IconName::FilePlus => i::STRATA_FILE_PLUS,
        IconName::AppWindow => i::STRATA_APP_WINDOW,
        IconName::CopyPlus => i::STRATA_COPY_PLUS,
        IconName::FolderInput => i::STRATA_FOLDER_INPUT,
        IconName::FolderOutput => i::STRATA_FOLDER_OUTPUT,
        IconName::Send => i::STRATA_SEND_HORIZONTAL,
        IconName::PackagePlus => i::STRATA_PACKAGE_PLUS,
        IconName::Extract => i::STRATA_PACKAGE_OPEN,
        IconName::FolderArchive => i::STRATA_FOLDER_ARCHIVE,
        IconName::Palette => i::STRATA_PALETTE,
        IconName::Route => i::STRATA_ROUTE,
        IconName::FileType => i::STRATA_FILE_TYPE,
        IconName::CircleX => i::STRATA_CIRCLE_X,
        IconName::Undo => i::STRATA_UNDO_2,
        IconName::Terminal => i::STRATA_TERMINAL,
        IconName::Properties => i::STRATA_INFO,
        IconName::Rename => i::STRATA_PENCIL,
        IconName::Open => i::STRATA_EXTERNAL_LINK,
        IconName::Wrap => i::STRATA_WRAP_TEXT,
        IconName::Source => i::STRATA_FILE_CODE,
        IconName::Rendered => i::STRATA_FILE_TEXT,
        IconName::Eye => i::STRATA_EYE,
        IconName::EyeOff => i::STRATA_EYE_OFF,

        // Views.
        IconName::ViewList => i::STRATA_LIST,
        IconName::ViewColumns => i::STRATA_COLUMNS,
        IconName::ViewIcons => i::STRATA_ICONS,
        IconName::ViewSettings => i::STRATA_SETTINGS_2,
        IconName::Sidebar => i::STRATA_PANEL_LEFT_SYMBOLIC,
        IconName::SortAscending => i::STRATA_ARROW_UP_NARROW_WIDE,
        IconName::SortDescending => i::STRATA_ARROW_DOWN_WIDE_NARROW,

        // Status.
        IconName::Info => i::STRATA_INFO,
        IconName::Warning => i::STRATA_TRIANGLE_ALERT,
        IconName::Error => i::STRATA_X,
        IconName::Success => i::STRATA_CHECK,
        IconName::ListChecks => i::STRATA_LIST_CHECKS,
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

        IconName::Minimize => i::STRATA_MINUS,
        IconName::Maximize => i::STRATA_MAXIMIZE,
        IconName::Restore => i::STRATA_MINIMIZE,
    }
}
