// SPDX-License-Identifier: MIT

use std::rc::Rc;

use rune::Mods;

use super::data::ItemId;
use super::intent::{DropSite, Gap, SidebarIntent};
use super::state::{PinVerdict, SidebarState};
use super::{ActionFn, Emit};
use crate::contract::dnd::{DropAction, DropActions, DropHover, DropOffer, DropZone};

pub fn pin_action(offer: &DropOffer, verdict: PinVerdict) -> Option<DropAction> {
    if verdict == PinVerdict::Refused {
        return None;
    }
    if offer.actions.contains(DropActions::COPY) {
        Some(DropAction::Copy)
    } else if offer.local && offer.actions.contains(DropActions::MOVE) {
        Some(DropAction::Move)
    } else {
        None
    }
}

pub(crate) fn gap<S: 'static>(
    access: fn(&mut S) -> &mut SidebarState,
    emit: &Emit<S>,
    gap: Gap,
    verdict: PinVerdict,
    check: &super::PinCheck,
) -> DropZone<S> {
    let takes = check.takes(verdict);
    let index = gap.index;
    let on_drop = emit.clone();
    let on_hover = emit.clone();
    let zone = DropZone::new(move |s: &mut S, dropped, tx| {
        access(s).end_drag();
        on_drop(
            s,
            SidebarIntent::PinDrop {
                gap: gap.clone(),
                dropped,
            },
            tx,
        );
    })
    .on_hover(move |s: &mut S, hover, offer: &DropOffer, tx| {
        let st = access(s);
        st.set_drop_hover(DropSite::Gap(index), hover);
        if hover == DropHover::Accepting {
            st.note_drag(offer);
            on_hover(
                s,
                SidebarIntent::PinHover {
                    serial: offer.serial,
                    uris: offer.uris.clone(),
                    local: offer.local,
                },
                tx,
            );
        }
        on_hover(
            s,
            SidebarIntent::DropHover {
                site: DropSite::Gap(index),
                hover,
            },
            tx,
        );
    })
    .action(move |_: &S, offer, _: Mods| {
        if takes {
            pin_action(offer, verdict)
        } else {
            None
        }
    });
    if verdict == PinVerdict::Refused {
        zone.on_reject(move |s: &mut S, _, _| access(s).end_drag())
    } else {
        zone
    }
}

/// A row's zone: files dropped on it go into its location, as the host's `action` decides.
pub(crate) fn row<S: 'static>(
    access: fn(&mut S) -> &mut SidebarState,
    emit: &Emit<S>,
    action: Option<&ActionFn<S>>,
    item: ItemId,
) -> DropZone<S> {
    let on_drop = emit.clone();
    let on_hover = emit.clone();
    let dropped_item = item.clone();
    let mut zone = DropZone::new(move |s: &mut S, dropped, tx| {
        access(s).end_drag();
        on_drop(
            s,
            SidebarIntent::DropInto {
                item: dropped_item.clone(),
                dropped,
            },
            tx,
        );
    })
    .on_hover({
        let item = item.clone();
        move |s: &mut S, hover, _: &DropOffer, tx| {
            let site = DropSite::Item(item.clone());
            access(s).set_drop_hover(site.clone(), hover);
            on_hover(s, SidebarIntent::DropHover { site, hover }, tx);
        }
    })
    .on_reject(move |s: &mut S, _, _| access(s).end_drag());
    if let Some(action) = action {
        let action = Rc::clone(action);
        zone = zone.action(move |s: &S, offer, mods| action(s, &item, offer, mods));
    }
    zone
}

pub(crate) fn background<S: 'static>(access: fn(&mut S) -> &mut SidebarState) -> DropZone<S> {
    DropZone::new(|_: &mut S, _, _| {})
        .mime_types(&[])
        .local(false)
        .on_hover(move |s: &mut S, hover, _: &DropOffer, _| {
            access(s).set_drop_hover(DropSite::Background, hover)
        })
        .on_reject(move |s: &mut S, _, _| access(s).end_drag())
}
