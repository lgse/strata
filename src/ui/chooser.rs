// SPDX-License-Identifier: MIT

mod destination;
mod download;
mod geometry;
mod image_conversion;
mod layout;
mod options;
mod shortcuts;
mod source;
mod state;

#[cfg(test)]
mod tests;

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    ffi::{OsStr, OsString},
    path::{Component, Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::TryRecvError,
    },
    time::Duration,
};

use ashpd::{
    PortalError, WindowIdentifierType,
    desktop::file_chooser::{Choice, FileFilter, SelectedFiles},
};
use gtk::{gdk::ModifierType, gio, glib, prelude::*};

use crate::{
    adapters::{LocalFileSource, LocalOperationProvider, LocalPreviewProvider, probes},
    app::BrowserEvent,
    model::{EntryKind, FileEntry, Location, MetadataValue},
    portal::{
        ChooserKind, ChooserRequest, DestinationCheck, check_destinations, local_uri,
        open_selection, safe_filename, writable_from_read_only,
    },
    services::{
        DirectoryChange, DirectoryEvent, DirectoryRequest, FileSource, ItemIdentity, LoadHandle,
        LocationValidationError, MetadataRequest, RemoteDownload, StampAnswer, WatchRequest,
        Watching, download_remote, remote_file_url,
    },
};

use super::{
    blur::BlurBin,
    browser::{BrowserView, dismiss_modal_layer, modal_layer},
    browser_modes::{BrowserMode, ThumbnailResize},
    controls::{
        ModalTone, focus_button, form_check_button, form_entry, form_label, menu_option,
        message_dialog_layout,
    },
    marquee::OriginClick,
    modal::close_on_escape,
    preferences::PreferenceManager,
    preview::{PreviewDrawer, preview_target},
    shortcut_footer::ShortcutFooter,
    top_bar_navigation::TopBarNavigation,
    window::{
        ChooserKeys, ChooserPolicy, MIN_SIDEBAR_WIDTH, SidebarView, build_appearance_menu,
        build_sidebar, edit_focused_text, home_directory, install_modal_focus_trap,
        visible_modal_layer,
    },
};
use super::{
    focus_navigation::{
        FOCUS_MODIFIERS, arrow_direction, plain_tab_direction, vim_focus_direction,
    },
    keys::{self, KeyPress, Map, SHORTCUT_MODIFIERS, bindings::*},
};

use destination::DestinationHost;
pub(crate) use destination::{DestinationRequest, present_destination_chooser};
use geometry::{chooser_initial_dimensions, detect_monitor_geometry};
use layout::build_chooser_hosted;
use options::{ChoiceControl, ChooserDropdown, PortalFilter};
use source::ChooserFileSource;
#[cfg(test)]
use source::OUTSIDE_ROOT;
use state::ChooserState;

type Completion = Box<dyn FnOnce(ashpd::backend::Result<SelectedFiles>)>;
type DestinationValidator =
    Rc<dyn Fn(&Path) -> futures_lite::future::BoxedLocal<Result<PathBuf, String>>>;

thread_local! {
    static CHOOSERS: RefCell<HashMap<String, glib::WeakRef<gtk::Window>>> = RefCell::new(HashMap::new());
    static DESTINATION_PARENTS: RefCell<Vec<glib::WeakRef<gtk::Window>>> = const { RefCell::new(Vec::new()) };
}

#[cfg(test)]
thread_local! {
    static DESTINATION_STATES: RefCell<Vec<std::rc::Weak<ChooserState>>> = const { RefCell::new(Vec::new()) };
}

#[cfg(test)]
pub(crate) fn destination_chooser_for(parent: &gtk::Window) -> Option<(gtk::Window, BrowserView)> {
    DESTINATION_STATES.with(|states| {
        states
            .borrow()
            .iter()
            .filter_map(std::rc::Weak::upgrade)
            .find(|state| {
                state.window.is_visible() && state.window.transient_for().as_ref() == Some(parent)
            })
            .map(|state| (state.window.clone(), state.view.clone()))
    })
}

pub(crate) fn present_chooser(
    request: ChooserRequest,
    cancelled: Arc<AtomicBool>,
    completion: impl FnOnce(ashpd::backend::Result<SelectedFiles>) + 'static,
) {
    let _ = build_chooser(request, cancelled, completion);
}

fn build_chooser(
    request: ChooserRequest,
    cancelled: Arc<AtomicBool>,
    completion: impl FnOnce(ashpd::backend::Result<SelectedFiles>) + 'static,
) -> Option<Rc<ChooserState>> {
    build_chooser_with_source(request, cancelled, completion, ChooserFileSource::new())
}

fn build_chooser_with_source(
    request: ChooserRequest,
    cancelled: Arc<AtomicBool>,
    completion: impl FnOnce(ashpd::backend::Result<SelectedFiles>) + 'static,
    source: Rc<ChooserFileSource>,
) -> Option<Rc<ChooserState>> {
    build_chooser_hosted(request, cancelled, completion, source, None)
}

pub(crate) fn cancel_chooser(token: &str) {
    CHOOSERS.with(|choosers| {
        let window = {
            choosers
                .borrow()
                .get(token)
                .and_then(glib::WeakRef::upgrade)
        };
        if let Some(window) = window {
            window.close();
        }
    });
}
