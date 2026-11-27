// SPDX-License-Identifier: MIT

use std::rc::Rc;

use rune::{Color, Cursor, El, Icon, Pixels, SurfaceFit, Tx, col, el, icon, row, text};
use strata_components::dialog::keep_words_whole;
use strata_core::i18n::{integer, tr};
use strata_desktop::services::integration_catalog::Category;
use strata_kit::{icons::IconName, look::Look};

use super::{catalog, installed, open_folder, refresh};
use crate::{
    App, buttons,
    settings_dialog::controls::{PageCtx, PickHandler, group_card, panel_border},
};

const HEADER_DESCRIPTION: &str = "Optional integrations bundled with Strata connect the context menu to other apps and services, such as sending files to a phone. They run with your permissions, like any program you install.";

pub(in crate::settings_dialog) fn build(ctx: PageCtx<'_>) -> Vec<El> {
    let app = ctx.app;
    if !ctx.search.shows_row(app, "integrations") {
        return Vec::new();
    }
    let state = &app.settings_dialog.integrations;
    let mut items = Vec::new();
    if let Some(header) = header_card(ctx) {
        items.push(header);
    }
    if let Some(section) = installed::section(app, state, ctx.layout.compact_content) {
        items.push(section);
    }
    items.extend(catalog::section(app, state, ctx.layout));
    items
}

fn header_card(ctx: PageCtx<'_>) -> Option<El> {
    let app = ctx.app;
    let look = &app.look;
    let kit = &look.kit;
    let c = &kit.c;
    let description = tr(HEADER_DESCRIPTION);
    let mut actions = row()
        .w_full()
        .min_w(0.0)
        .pt(8.0)
        .gap(8.0)
        .flex_wrap()
        .justify_end()
        .items_center()
        .child(icon_label_button(
            look,
            IconLabelButton {
                key: "integrations-refresh".to_owned(),
                icon: IconName::Refresh,
                label: tr("Refresh"),
                accessible: tr("Refresh"),
                disabled: false,
                on_press: Rc::new(|app, _| refresh(app)),
            },
        ));
    if app.settings_dialog.integrations.source.root.is_some() {
        actions = actions.child(icon_label_button(
            look,
            IconLabelButton {
                key: "integrations-open-folder".to_owned(),
                icon: IconName::FolderOpen,
                label: tr("Open folder"),
                accessible: tr("Open folder"),
                disabled: false,
                on_press: Rc::new(|app, _| open_folder(app)),
            },
        ));
    }
    let content = col()
        .w_full()
        .min_w(0.0)
        .px(18.0)
        .py(16.0)
        .gap(3.0)
        .child(
            ctx.search
                .title(app, look, &tr("Integrations"), kit.ty.ui_medium),
        )
        .child(
            text(keep_words_whole(&description))
                .label(description)
                .font(kit.ty.small)
                .color(c.text_muted)
                .wrap(),
        )
        .child(actions);
    let wrapped = ctx.search.row(app, look, "integrations", content);
    group_card(look, vec![wrapped])
}

pub(super) fn counted_heading(look: &Look, heading: &str, count: usize) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    row()
        .w_full()
        .pt(24.0)
        .pb(10.0)
        .px(4.0)
        .gap(8.0)
        .items_center()
        .child(text(tr(heading)).font(look.ty.heading).color(c.text_muted))
        .child(
            el().px(7.0)
                .py(1.0)
                .radius(look.round(8.0))
                .fill(kit.alpha(c.std_border, c.std_border.a * 0.35))
                .child(
                    text(integer(count as u64))
                        .font(kit.ty.small)
                        .color(c.text_muted),
                ),
        )
}

pub(super) fn logo_tile(look: &Look, pixels: Option<&Pixels>, category: Category) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    let artwork = match pixels {
        Some(px) => el()
            .size(20.0, 20.0)
            .pixels_fit(px, SurfaceFit::Fit)
            .a11y_hidden(),
        None => icon(category_icon(look, category), 20.0)
            .color(c.icon_accent)
            .a11y_hidden(),
    };
    el().size(36.0, 36.0)
        .center()
        .radius(look.round(8.0))
        .fill(kit.alpha(c.bg_base, 0.55))
        .border(1.0, panel_border(look))
        .child(artwork)
}

pub(super) fn category_icon(look: &Look, category: Category) -> &'static Icon {
    let (asset, fallback) = match category {
        Category::Cloud => ("strata-cloud", IconName::Network),
        Category::Devices => ("strata-smartphone", IconName::Phone),
        Category::VersionControl => ("strata-git-branch", IconName::Route),
        Category::Other => ("strata-blocks", IconName::Properties),
    };
    look.icon_named(asset)
        .unwrap_or_else(|| look.icon(fallback))
}

pub(super) struct IconLabelButton {
    pub(super) key: String,
    pub(super) icon: IconName,
    pub(super) label: String,
    pub(super) accessible: String,
    pub(super) disabled: bool,
    pub(super) on_press: PickHandler,
}

pub(super) fn icon_label_button(look: &Look, spec: IconLabelButton) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    let (fill, border, ink) = if spec.disabled {
        (c.surface_disabled, c.border_disabled, c.text_muted)
    } else {
        (
            Color::TRANSPARENT,
            kit.alpha(c.std_border, c.std_border.a * 0.54),
            c.text_base,
        )
    };
    let mut btn = row()
        .role("button")
        .label(spec.accessible)
        .min_h(28.0)
        .px(11.0)
        .py(4.0)
        .gap(6.0)
        .center()
        .radius(look.round(5.0))
        .fill(fill)
        .border(1.0, border)
        .focusable(!spec.disabled)
        .disabled(spec.disabled)
        .child(icon(look.icon(spec.icon), 14.0).color(ink).a11y_hidden())
        .child(text(spec.label).font(kit.ty.ui).color(ink).a11y_hidden());
    if spec.disabled {
        return btn.key(spec.key).layer_opacity(0.6);
    }
    let on_press = spec.on_press;
    btn = btn
        .cursor(Cursor::Pointer)
        .hover(kit.alpha(c.bg_accent, 0.15))
        .press(kit.alpha(c.bg_accent, 0.22))
        .focus_ring(2.0, 1.0, kit.alpha(c.border_focus, 0.50))
        .on_press_tx(move |app: &mut App, tx: &mut Tx| on_press(app, tx));
    buttons::on_return(spec.key, btn)
}

pub(super) fn note_text(look: &Look, message: String, error: bool) -> El {
    let color = if error {
        look.kit.c.danger.fg
    } else {
        look.kit.c.text_muted
    };
    text(keep_words_whole(&message))
        .label(message)
        .font(look.kit.ty.small)
        .color(color)
        .wrap()
}
