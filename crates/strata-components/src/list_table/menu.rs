//! The columns menu (Strata's `column_options.rs`): COLUMNS, a check item per column, a
//! separator, then "Reset to default columns". It stays open while its checks change; Escape or
//! a press outside closes it.

use std::rc::Rc;

use rune::{El, PointerPress, Tx};
use rune_kit::{IconButton, Menu, MenuItem, Placement, context_menu};
use strata_kit::icons::IconName;

use super::Shared;
use super::layout::{ColumnLayout, LayoutChange, LayoutWhy};
use super::state::MenuAt;

/// The heading strip's menu's key, under the table's key.
pub(super) fn headings_menu_key(key: &str) -> String {
    format!("{key}-headings-menu")
}

/// The Columns button's menu's key.
pub(super) fn button_menu_key(key: &str) -> String {
    format!("{key}-columns-menu")
}

/// The Columns button's key.
pub(super) fn button_key(key: &str) -> String {
    format!("{key}-columns-button")
}

/// Wide enough for the primary column's description to show whole.
const MENU_WIDTH: f32 = 260.0;

fn items<S: 'static>(shared: &Rc<Shared<S>>, s: &S, mut menu: Menu) -> Menu {
    let layout = (shared.parts)(s).layout;
    menu = menu.min_width(MENU_WIDTH).group_label("COLUMNS");
    for column in shared.columns.iter() {
        let mut item = MenuItem::new(column.title.clone()).checked(layout.is_visible(&column.id));
        if column.is_primary() {
            item = item
                .disabled(true)
                .description(format!("The {} column cannot be hidden", column.title));
        } else {
            let toggle = shared.clone();
            let id = column.id.clone();
            item = item.on_select_tx(move |s: &mut S, tx: &mut Tx| {
                let layout = (toggle.parts)(s).layout;
                let next = layout.with_visible(&id, !layout.is_visible(&id));
                if let Some(f) = &toggle.on_layout {
                    f(
                        s,
                        LayoutChange {
                            layout: next,
                            why: LayoutWhy::Visibility,
                        },
                        tx,
                    );
                }
            });
        }
        menu = menu.item(item);
    }
    let reset = shared.clone();
    menu.separator()
        .item(MenuItem::new("Reset to default columns").on_select_tx(
            move |s: &mut S, tx: &mut Tx| {
                let layout = ColumnLayout::new(reset.columns.iter().map(|c| c.id.clone()));
                if let Some(f) = &reset.on_layout {
                    f(
                        s,
                        LayoutChange {
                            layout,
                            why: LayoutWhy::Reset,
                        },
                        tx,
                    );
                }
            },
        ))
}

fn closing<S: 'static>(shared: &Rc<Shared<S>>, menu: Menu) -> Menu {
    let close = shared.clone();
    menu.on_close(move |s: &mut S| (close.table_mut)(s).menu = None)
}

/// The heading strip, wrapped by its menu: open at the pointer after a right-click, or under
/// the strip's right end for "Customize columns…".
pub(super) fn headings<S: 'static>(
    shared: &Rc<Shared<S>>,
    s: &S,
    strip: El,
    layer: &mut Vec<El>,
) -> El {
    let parts = (shared.parts)(s);
    let key = headings_menu_key(&shared.key);
    // A context menu's trigger is where it opens, not a button that opens it, so the strip
    // carries no expanded state.
    let menu = match parts.table.menu {
        Some(MenuAt::Headings(Some((x, y)))) => {
            context_menu(key, strip, Some(PointerPress { x, y }))
        }
        Some(MenuAt::Headings(None)) => context_menu(key, strip, None)
            .open(true)
            .placement(Placement::BottomEnd),
        _ => context_menu(key, strip, None),
    };
    let open = matches!(parts.table.menu, Some(MenuAt::Headings(_)));
    let menu = if open {
        closing(shared, items(shared, s, menu))
    } else {
        menu
    };
    // The menu's trigger wrapper takes the strip's full width.
    menu.build(&parts.look.kit, layer).w_full()
}

/// The Columns button for the pane header (Strata's "Columns" menu button), and its menu.
pub(super) fn button<S: 'static>(shared: &Rc<Shared<S>>, s: &S, layer: &mut Vec<El>) -> El {
    let parts = (shared.parts)(s);
    let look = parts.look;
    let open = shared.clone();
    let trigger = IconButton::new(look.icon(IconName::ViewColumns))
        .label("Columns")
        .help("Columns")
        .on_press(move |s: &mut S| (open.table_mut)(s).menu = Some(MenuAt::Button))
        .build(&look.kit)
        .key(button_key(&shared.key));
    let is_open = parts.table.menu == Some(MenuAt::Button);
    let menu = Menu::new(button_menu_key(&shared.key))
        .trigger(trigger)
        .placement(Placement::BottomEnd)
        .open(is_open);
    let menu = if is_open {
        closing(shared, items(shared, s, menu))
    } else {
        menu
    };
    menu.build(&look.kit, layer)
}
