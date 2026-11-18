// SPDX-License-Identifier: MIT

//! Conversions between W1 (`strata_components::contract::dnd`) and P1 (`strata_platform::dnd`),
//! plus the Linux file-manager drag formats and drag badge snapshot (`X2` adapter).

use std::{cell::RefCell, sync::Arc};

use rune::{Cx, El, Mods, PointerEvent, Tx, col, text};
use strata_components::contract::dnd::{
    DragPayload, DropAction, DropActions, DropData, DropHover, DropItem, DropOffer, DropZone,
    Dropped, ReadFailure, URI_LIST, parse_uri_list,
};
use strata_kit::{icons::IconName, look::Look};
use strata_platform::{
    dnd::{
        Actions, DragAction, DragIcon, DragItem, DropData as P1Data, DropOffer as P1Offer,
        DropPoint, DropTarget, Hover,
    },
    transfer::ReadError,
};

use super::content::{self, Got, Reads};

/// GNOME's file-list type (`copy\nfile:///...`).
pub(crate) const GNOME_COPIED_FILES: &str = "x-special/gnome-copied-files";
/// KDE's cut flag beside `text/uri-list`: `0` for a copy.
pub(crate) const KDE_CUT_SELECTION: &str = "application/x-kde-cutselection";

/// `drag_actions_for_modifiers` in `src/ui/browser/clipboard.rs`.
pub(crate) fn actions_for_mods(mods: Mods, can_move: bool) -> DropActions {
    let allowed = if can_move {
        DropActions::COPY | DropActions::MOVE
    } else {
        DropActions::COPY
    };
    if mods.ctrl {
        DropActions::COPY
    } else if mods.shift && can_move {
        DropActions::MOVE
    } else {
        allowed
    }
}

pub(crate) fn encode_uri_list(uris: &[String]) -> Vec<u8> {
    let mut out = Vec::new();
    for uri in uris {
        out.extend_from_slice(uri.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    out
}

pub(crate) fn encode_gnome_copied_files(uris: &[String]) -> Vec<u8> {
    let mut out = b"copy".to_vec();
    for uri in uris {
        out.push(b'\n');
        out.extend_from_slice(uri.as_bytes());
    }
    out
}

pub(crate) fn drag_item(payload: &DragPayload, icon: Option<DragIcon>) -> DragItem {
    let uris: Arc<[String]> = payload.uris.clone().into();
    let (a, b) = (uris.clone(), uris);
    let mut item = DragItem::new()
        .lazy(URI_LIST, move || encode_uri_list(&a))
        .lazy(GNOME_COPIED_FILES, move || encode_gnome_copied_files(&b))
        .data(KDE_CUT_SELECTION, b"0".to_vec())
        .actions(actions_to_p1(payload.actions))
        .local(payload.clone());
    if let Some(icon) = icon {
        item = item.icon(icon);
    }
    item
}

pub(crate) fn drag_icon(count: usize, folders: bool, scale: f32) -> DragIcon {
    #[derive(Clone)]
    struct Badge {
        count: usize,
        folders: bool,
    }
    fn build(badge: &Badge, _: &mut Cx<Badge>) -> El {
        let look = Look::fallback(false);
        let name = if badge.folders {
            IconName::Folder
        } else {
            IconName::File
        };
        let glyph = rune::icon(look.icon(name), 32.0).color(look.kit.c.icon_base);
        let mut el = col().size(48.0, 48.0).center().child(glyph);
        if badge.count > 1 {
            el = el.child(
                text(badge.count.to_string())
                    .font(look.kit.ty.small)
                    .color(look.kit.c.submit_fg)
                    .fill(look.kit.c.bg_accent)
                    .radius(8.0)
                    .px(4.0)
                    .pin(None, Some(0.0), Some(0.0), None),
            );
        }
        el
    }
    DragIcon::snapshot(Badge { count, folders }, build, (48.0, 48.0), scale, false)
}

/// Each action as W1 and P1 name it: as a set's member on both sides, and as P1's one action.
const ACTIONS: [(DropActions, Actions, DragAction); 3] = [
    (DropActions::COPY, Actions::COPY, DragAction::Copy),
    (DropActions::MOVE, Actions::MOVE, DragAction::Move),
    (DropActions::LINK, Actions::LINK, DragAction::Link),
];

pub(crate) fn actions_to_p1(actions: DropActions) -> Actions {
    ACTIONS
        .into_iter()
        .filter(|(w1, ..)| actions.contains(*w1))
        .fold(Actions::NONE, |out, (_, p1, _)| out | p1)
}

pub(crate) fn actions_of_p1(actions: Actions) -> DropActions {
    ACTIONS
        .into_iter()
        .filter(|(.., one)| actions.has(*one))
        .fold(DropActions::NONE, |out, (w1, ..)| out | w1)
}

pub(crate) fn action_to_p1(action: DropAction) -> DragAction {
    match action {
        DropAction::Copy | DropAction::Ask => DragAction::Copy,
        DropAction::Move => DragAction::Move,
        DropAction::Link => DragAction::Link,
    }
}

pub(crate) fn hover_of_p1(hover: Hover) -> DropHover {
    match hover {
        Hover::Left => DropHover::Left,
        Hover::Accepting(_) => DropHover::Accepting,
        Hover::Rejecting => DropHover::Rejecting,
    }
}

thread_local! {
    static RUNNING_PAYLOAD: RefCell<Option<DragPayload>> = const { RefCell::new(None) };
}

pub(crate) fn set_running(payload: Option<DragPayload>) {
    RUNNING_PAYLOAD.with(|slot| *slot.borrow_mut() = payload);
}

pub(crate) fn running() -> Option<DragPayload> {
    RUNNING_PAYLOAD.with(|slot| slot.borrow().clone())
}

pub(crate) fn offer_of_p1(offer: &P1Offer) -> DropOffer {
    let mimes: Vec<std::borrow::Cow<'static, str>> =
        offer.mimes.iter().map(|m| m.clone().into()).collect();
    let mut w1 = DropOffer::new(mimes, actions_of_p1(offer.actions))
        .local(offer.local)
        .serial(offer.serial);
    let uris = match running().filter(|_| offer.local) {
        Some(payload) => Some(payload.uris),
        None => offer.peeked(URI_LIST).map(parse_uri_list),
    };
    if let Some(uris) = uris {
        w1 = w1.uris(uris);
    }
    w1
}

pub(crate) fn point_of_p1(point: DropPoint) -> PointerEvent {
    let mut event = PointerEvent::default();
    event.x = point.x;
    event.y = point.y;
    event.window_x = point.window_x;
    event.window_y = point.window_y;
    event
}

pub(crate) fn dropped_of_p1(data: &P1Data) -> Dropped {
    let mut offer = offer_of_p1(&data.offer);
    let items: Vec<DropItem> = data
        .items
        .iter()
        .map(|(mime, bytes)| DropItem::new(mime.clone(), bytes.clone()))
        .collect();
    let mut body = DropData::new(items).failed(failures_of_p1(&data.failed));
    if let Some(payload) = data.local::<DragPayload>() {
        offer = offer.uris(payload.uris.clone());
        body = body.local(payload.clone());
    } else if offer.uris.is_none() {
        let uris = body.uris();
        if !uris.is_empty() {
            offer = offer.uris(uris);
        }
    }
    Dropped::new(offer, body, data.offer.mods.unwrap_or_default())
        .point(data.point.map(point_of_p1))
}

/// Why a read failed, as W1 says it: a read past P1's bound is too large.
pub(crate) fn failure_of_p1(error: &ReadError) -> ReadFailure {
    match error {
        ReadError::TooLarge => ReadFailure::TooLarge,
        ReadError::TimedOut => ReadFailure::TimedOut,
        other => ReadFailure::Failed(other.to_string()),
    }
}

fn failures_of_p1(failed: &[(String, ReadError)]) -> Vec<(String, ReadFailure)> {
    failed
        .iter()
        .map(|(mime, error)| (mime.clone(), failure_of_p1(error)))
        .collect()
}

/// A drop being read by P1, as the content policy sees it: the offered types a content zone
/// takes, as the walk after the drop sees them (`content_zone_takes`).
struct Reading<'a> {
    data: &'a P1Data,
}

impl Reads for Reading<'_> {
    fn offered(&self) -> Vec<&str> {
        let types = self.data.offer.mimes.iter().map(String::as_str);
        types
            .filter(|mime| content::content_zone_takes(mime))
            .collect()
    }

    fn got(&self, mime: &str) -> Got<'_> {
        if let Some((_, bytes)) = self.data.items.iter().find(|(m, _)| m == mime) {
            return Got::Bytes(bytes);
        }
        match self.data.failed.iter().find(|(m, _)| m == mime) {
            Some((_, error)) => Got::Failed(failure_of_p1(error)),
            None => Got::Unread,
        }
    }

    fn local(&self) -> bool {
        self.data.offer.local
    }
}

pub(crate) fn target_of<S: 'static>(zone: DropZone<S>) -> DropTarget<S> {
    let dropped_zone = zone.clone();
    let target = DropTarget::new(move |state: &mut S, data: P1Data, tx: &mut Tx| {
        dropped_zone.deliver(state, dropped_of_p1(&data), tx);
    })
    .mime_types(zone.preferred_mime_types());
    // A zone that takes content reads one type at a time, as GTK reads only the one it uses;
    // a zone of files reads the file list.
    let mut target = if content::zone_takes(&zone) {
        // The walks read and judge a drop by the policy's list (`content_zone_takes`), not the
        // zone's: a zone listing other types would be read by the wrong one.
        debug_assert!(
            content::lists_the_policy(&zone),
            "a content zone lists the file list and `CONTENT_TYPES`, and nothing else: {:?}",
            zone.preferred_mime_types()
        );
        target.read_next(|data| content::next_read(&Reading { data }))
    } else {
        target.read_all()
    };
    if zone.accepts_local() {
        target = target.local::<DragPayload>();
    }
    if zone.hears_hover() {
        if zone.reads(URI_LIST) {
            target = target.peek(&[URI_LIST]);
        }
        let hover_zone = zone.clone();
        target = target.on_hover(move |state: &mut S, hover, offer: &P1Offer, tx: &mut Tx| {
            hover_zone.hover(state, hover_of_p1(hover), &offer_of_p1(offer), tx);
        });
    }
    if zone.follows_moves() {
        let move_zone = zone.clone();
        target = target.on_move(move |state: &mut S, point, tx: &mut Tx| {
            move_zone.moved(state, point_of_p1(point), tx);
        });
    }
    if zone.rejects() {
        let reject_zone = zone.clone();
        target = target.on_reject(move |state: &mut S, offer: &P1Offer, tx: &mut Tx| {
            reject_zone.reject(state, offer_of_p1(offer), tx);
        });
    }
    target.action(move |state: &S, offer: &P1Offer, mods: Option<Mods>| {
        zone.choose(state, &offer_of_p1(offer), mods.unwrap_or_default())
            .map(action_to_p1)
    })
}
