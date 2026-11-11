//! The sidebar's drop zones: one per pin gap, one per row that takes files, and one over the
//! whole sidebar that only notices a drag is there. All attach through the host's `Dnd`.

use std::rc::Rc;

use rune::Mods;

use super::data::ItemId;
use super::intent::{DropSite, Gap, SidebarIntent};
use super::state::{PinVerdict, SidebarState};
use super::{ActionFn, Emit};
use crate::contract::dnd::{DropAction, DropActions, DropHover, DropOffer, DropZone};

/// The action a drop into a pin gap takes, or `None` when the gap refuses it.
///
/// - A drag the host refused (a file, a trashed folder, a link) is refused.
/// - A drag that offers a copy takes it. A pin moves and copies nothing, so a modifier
///   doesn't matter: Shift on a drag from Strata still only pins.
/// - A move is taken only from this process, whose sources never delete on a move. A
///   Move-only drag from another app is refused, since completing it would ask that app to
///   delete the originals.
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

/// A pin gap's zone. `verdict` is the host's verdict on the drag over the gaps as the view
/// was built, and `check` the host's checks: under [`super::Unchecked::Wait`] the gap refuses
/// a drop the host hasn't confirmed ([`super::PinCheck::takes`]). A refused drag is stopped
/// here, showing "can't drop", rather than passed to the row around it.
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

/// The zone over the whole sidebar. It takes nothing; it only hears that a drag is over the
/// sidebar, so an empty PINNED section can show its gap and rows drop their hover fill.
pub(crate) fn background<S: 'static>(access: fn(&mut S) -> &mut SidebarState) -> DropZone<S> {
    DropZone::new(|_: &mut S, _, _| {})
        .mime_types(&[])
        // `DropZone::new` takes drags from this process; this zone takes none, so a local
        // drag over the background is refused too, not reported to its source as a copy.
        .local(false)
        .on_hover(move |s: &mut S, hover, _: &DropOffer, _| {
            access(s).set_drop_hover(DropSite::Background, hover)
        })
        .on_reject(move |s: &mut S, _, _| access(s).end_drag())
}
