//! The hierarchy menu: the current folder and its ancestors, current first and checked,
//! opened under the bar where a secondary press landed.

use rune::{El, Tx, el};
use rune_kit::{Menu, MenuItem};

use super::build::Ctx;
use super::data::{CRUMB_CHARS, elide_middle};
use super::intent::{LocationIntent, Via};

pub(super) fn view<S: 'static>(cx: &Ctx<'_, S>, layer: &mut Vec<El>) -> El {
    let Some((x, y)) = cx.state.menu() else {
        return el();
    };
    let access = cx.access;
    let mut menu = Menu::new(format!("{}-menu", cx.key))
        .trigger(el().size(0.0, 0.0))
        .open(true)
        .at(x, y)
        .on_close(move |s: &mut S| access(s).close_menu());
    for (i, crumb) in cx.trail.crumbs.iter().rev().enumerate() {
        let (id, emit) = (crumb.id.clone(), cx.emit.clone());
        menu = menu.item(
            MenuItem::new(elide_middle(&crumb.label, CRUMB_CHARS).into_owned())
                .icon(cx.look.icon(crumb.icon))
                .checked(i == 0)
                .on_select_tx(move |s: &mut S, tx: &mut Tx| {
                    access(s).close_menu();
                    let id = id.clone();
                    emit(s, LocationIntent::Open { id, via: Via::Menu }, tx);
                }),
        );
    }
    menu.build(&cx.look.kit, layer).absolute(0.0, 0.0)
}
