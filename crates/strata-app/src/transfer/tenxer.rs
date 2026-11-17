// SPDX-License-Identifier: MIT

//! 10xer's transfers: `p` and `P` paste with Keep Both or Replace focused on a conflict, and
//! `m:` and `c:` move or copy into a picked folder without going there
//! (`src/ui/browser/file_commands.rs`, `src/ui/browser/transfer.rs`).

use std::rc::Rc;

use rune::Tx;
use strata_core::model::Location;

use super::{
    PasteInto, TransferRequest, begin_transfer_request, paste_into, resolve_paste_location,
};
use crate::{App, clipboard, commands::On, conflict_dialog::ConflictFocus, shell};

/// `BrowserView::paste_preferring`: pastes with `focus` as the default conflict button, calling
/// `on_empty` when the clipboard offers no files.
pub(crate) fn paste_preferring(
    app: &mut App,
    focus: ConflictFocus,
    on_empty: Rc<dyn Fn(&mut App, &mut Tx)>,
    tx: &mut Tx,
) {
    shell::publish(app);
    if !clipboard::has_files(app) {
        on_empty(app, tx);
        return;
    }
    if let Some(destination) = resolve_paste_location(app, &On::Focus) {
        let into = PasteInto {
            destination,
            focus,
            on_empty: Some(on_empty),
        };
        paste_into(app, into, tx);
    }
}

/// `ViewState::start_transfer_in_place`: moves or copies `sources` into `destination` without
/// going there, with Keep Both focused on a conflict.
pub(crate) fn start_in_place_transfer(
    app: &mut App,
    destination: Location,
    sources: Vec<Location>,
    move_sources: bool,
) {
    let request = TransferRequest::new(destination, sources, move_sources);
    begin_transfer_request(
        app,
        request.revealing(false).focusing(ConflictFocus::KeepBoth),
    );
}
