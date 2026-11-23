// SPDX-License-Identifier: MIT

use rune::Tx;
use strata_core::model::Location;

use super::{
    OnEmpty, PasteInto, TransferRequest, begin_transfer_request, paste_into, resolve_paste_location,
};
use crate::{App, clipboard, commands::On, conflict_dialog::ConflictFocus, shell};

pub(crate) fn paste_preferring(
    app: &mut App,
    focus: ConflictFocus,
    on_empty: OnEmpty,
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
