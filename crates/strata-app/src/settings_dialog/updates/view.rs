// SPDX-License-Identifier: MIT

//! View builders for the Updates settings page.

use rune::{Color, Cursor, El, Tx, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant};
use strata_core::i18n::tr;
use strata_desktop::{
    build_info::{build_kind, installed_version},
    services::{BuildKind, Channel, ManagedInstall, ReleaseMetadata, UpdateMethod},
};
use strata_kit::{icons::IconName, look::Look};

use super::{
    UpdatePhase, UpdatesState,
    install::{
        FINALIZING_STATUS, InstallProgressState, aur_update_action_label, install_failure_message,
    },
    messages::{
        LATEST_RELEASES_URL, auto_update_description, available_status_localized,
        channel_description, failed_status_localized, installed_version_status,
        managed_channel_description, managed_install_summary, split_anchor,
    },
    on_action_press, on_auto_updates_toggled, on_channel_changed, open_uri,
};
use crate::{
    App, buttons,
    settings_dialog::controls::{
        ChoiceOption, ChoiceSpec, PageCtx, RowSpec, choice_menu, control_border, group_card,
        option_row, switch_row,
    },
    shell,
};

pub(super) const ACTION_BUTTON_KEY: &str = "settings-update-check-action";
pub(super) const STATUS_TITLE_KEY: &str = "settings-update-status-title";
pub(super) const STATUS_TEXT_KEY: &str = "settings-update-status-text";
pub(super) const STATUS_LINK_KEY: &str = "settings-update-status-link";
pub(super) const PROGRESS_BAR_KEY: &str = "settings-update-progress";
pub(super) const MANAGED_BANNER_KEY: &str = "settings-update-managed-banner";
pub(super) const NOTES_CARD_KEY: &str = "settings-update-release-notes";
const MAX_PREVIEW_NOTES_CHARS: usize = 600;

pub(in crate::settings_dialog) fn build(ctx: PageCtx<'_>, menu_layer: &mut Vec<El>) -> Vec<El> {
    let app = ctx.app;
    let prefs = shell::preferences(app);
    let updates = &app.settings_dialog.updates;
    let method = updates.effective_method();
    let managed = updates.managed();
    let channel = prefs.release_channel();
    let channel_disabled = !prefs.check_for_updates || managed.is_some() || updates.is_installing();
    let menu = release_channel_menu(ctx, channel, channel_disabled, menu_layer);

    let mut rows = Vec::new();
    if let Some(managed) = managed {
        rows.push(managed_banner_row(ctx, managed));
    }
    rows.push(check_status_row(ctx, updates, method, managed));
    rows.push(switch_row(
        ctx,
        RowSpec::new(
            "auto-updates",
            "Check for updates automatically",
            auto_update_description(method),
        ),
        prefs.check_for_updates,
        |app, on, tx| {
            shell::saved(app).change(|s| s.check_for_updates = on);
            on_auto_updates_toggled(app, on, tx);
        },
    ));
    rows.push(channel_row(ctx, managed, (channel, channel_disabled), menu));

    group_card(&app.look, rows).into_iter().collect()
}

fn managed_banner_row(ctx: PageCtx<'_>, managed: &ManagedInstall) -> Option<El> {
    let app = ctx.app;
    if !ctx.search.shows_row(app, "check") {
        return None;
    }
    let look = &app.look;
    let kit = &look.kit;
    Some(
        col()
            .key(MANAGED_BANNER_KEY)
            .w_full()
            .min_w(0.0)
            .px(18.0)
            .py(16.0)
            .gap(4.0)
            .child(
                text(tr("Package-managed installation"))
                    .font(kit.ty.ui_medium)
                    .color(kit.c.text_base),
            )
            .child(
                text(managed_install_summary(managed))
                    .font(kit.ty.small)
                    .color(kit.c.text_muted)
                    .selectable(true)
                    .wrap(),
            ),
    )
}

fn channel_row(
    ctx: PageCtx<'_>,
    managed: Option<&ManagedInstall>,
    (channel, disabled): (Channel, bool),
    menu: El,
) -> Option<El> {
    let Some(managed) = managed else {
        return option_row(
            ctx,
            RowSpec::new("channel", "Release channel", channel_description(channel))
                .disabled(disabled),
            menu,
        );
    };
    let app = ctx.app;
    if !ctx.search.shows_row(app, "channel") {
        return None;
    }
    let look = &app.look;
    let kit = &look.kit;
    let copy = col()
        .min_w(0.0)
        .gap(3.0)
        .child(
            ctx.search
                .title(app, look, &tr("Release channel"), kit.ty.ui_medium),
        )
        .child(
            text(managed_channel_description(managed))
                .font(kit.ty.small)
                .color(kit.c.text_muted)
                .wrap(),
        );
    let mut line = if ctx.layout.compact_content {
        col()
            .w_full()
            .min_w(0.0)
            .px(18.0)
            .py(16.0)
            .gap(look.space.sm)
            .child(copy.w_full())
            .child(col().w_full().min_w(0.0).child(menu))
    } else {
        row()
            .w_full()
            .min_w(0.0)
            .px(18.0)
            .py(16.0)
            .gap(look.space.xl)
            .items_center()
            .child(copy.grow(1.0))
            .child(menu)
    };
    if disabled {
        line = line.layer_opacity(0.45);
    }
    ctx.search.row(app, look, "channel", line)
}

fn check_status_row(
    ctx: PageCtx<'_>,
    updates: &UpdatesState,
    method: UpdateMethod,
    managed: Option<&ManagedInstall>,
) -> Option<El> {
    let app = ctx.app;
    if !ctx.search.shows_row(app, "check") {
        return None;
    }
    let look = &app.look;
    let presentation = status_presentation(updates, managed);
    let heading = row()
        .gap(8.0)
        .items_center()
        .child(
            icon(look.icon(presentation.icon), 18.0)
                .color(presentation.icon_color(look))
                .a11y_hidden(),
        )
        .child(
            text(presentation.title)
                .key(STATUS_TITLE_KEY)
                .font(look.kit.ty.ui_medium)
                .color(look.kit.c.text_base),
        );

    let mut copy = col()
        .min_w(0.0)
        .gap(6.0)
        .child(heading)
        .child(status_detail(look, updates, method, managed));

    if let Some(progress) = progress_bar(look, updates) {
        copy = copy.child(progress);
    }

    let action = Button::new(presentation.button_label)
        .variant(presentation.button_variant)
        .disabled(presentation.button_disabled)
        .on_press_tx(on_action_press)
        .build(&look.kit);
    let summary = status_summary_row(ctx, copy, buttons::on_return(ACTION_BUTTON_KEY, action));

    let mut content = col()
        .w_full()
        .min_w(0.0)
        .px(18.0)
        .py(16.0)
        .gap(look.space.md)
        .child(summary);

    if let UpdatePhase::Available { release, .. } = &updates.phase
        && !release.notes.trim().is_empty()
    {
        content = content.child(release_notes_preview(look, release));
    }

    ctx.search.row(app, look, "check", content)
}

fn status_summary_row(ctx: PageCtx<'_>, copy: El, action_btn: El) -> El {
    let look = &ctx.app.look;
    if ctx.layout.compact_content {
        col()
            .w_full()
            .min_w(0.0)
            .gap(look.space.md)
            .child(copy.w_full())
            .child(action_btn)
    } else {
        row()
            .w_full()
            .min_w(0.0)
            .gap(look.space.xl)
            .items_center()
            .child(copy.grow(1.0))
            .child(action_btn)
    }
}

struct StatusPresentation {
    icon: IconName,
    danger_icon: bool,
    title: String,
    button_label: String,
    button_variant: ButtonVariant,
    button_disabled: bool,
}

impl StatusPresentation {
    fn new(icon: IconName, title: &str, button_label: &str) -> Self {
        Self {
            icon,
            danger_icon: false,
            title: tr(title),
            button_label: tr(button_label),
            button_variant: ButtonVariant::Neutral,
            button_disabled: false,
        }
    }

    fn danger(mut self) -> Self {
        self.danger_icon = true;
        self
    }

    fn submit(mut self) -> Self {
        self.button_variant = ButtonVariant::Submit;
        self
    }

    fn disabled(mut self, disabled: bool) -> Self {
        self.button_disabled = disabled;
        self
    }

    fn icon_color(&self, look: &Look) -> Color {
        if self.danger_icon {
            look.kit.c.danger.fg
        } else {
            look.kit.c.icon_accent
        }
    }
}

fn status_presentation(
    updates: &UpdatesState,
    managed: Option<&ManagedInstall>,
) -> StatusPresentation {
    match &updates.phase {
        UpdatePhase::Idle => {
            StatusPresentation::new(IconName::Info, "Check for updates", "Check now")
        }
        UpdatePhase::Checking { .. } => {
            StatusPresentation::new(IconName::Info, "Checking for updates…", "Check now")
                .disabled(true)
        }
        UpdatePhase::UpToDate => {
            StatusPresentation::new(IconName::Success, "Strata is up to date", "Check now")
        }
        UpdatePhase::Available {
            method,
            returns_to_stable,
            ..
        } => available_presentation(*method, *returns_to_stable, managed),
        UpdatePhase::Installing {
            cancelling,
            finalizing,
            ..
        } => StatusPresentation::new(IconName::Downloads, "An update is available", "Cancel")
            .disabled(*cancelling || *finalizing),
        UpdatePhase::Installed { .. } => {
            StatusPresentation::new(IconName::Success, "An update is available", "Restart now")
                .submit()
        }
        UpdatePhase::InstallCancelled => {
            StatusPresentation::new(IconName::Info, "An update is available", "Check now")
        }
        UpdatePhase::InstallFailed { .. } => {
            StatusPresentation::new(IconName::Warning, "An update is available", "Check now")
                .danger()
        }
        UpdatePhase::CheckFailed { .. } => {
            StatusPresentation::new(IconName::Warning, "Couldn't check for updates", "Check now")
                .danger()
        }
    }
}

fn available_presentation(
    method: UpdateMethod,
    returns_to_stable: bool,
    managed: Option<&ManagedInstall>,
) -> StatusPresentation {
    let label = match method {
        UpdateMethod::InPlace if returns_to_stable => "Return to stable",
        UpdateMethod::InPlace => "Install update",
        UpdateMethod::Pacman => "Check again",
        UpdateMethod::Aur => aur_update_action_label(managed),
        UpdateMethod::Omarchy => "Open Omarchy Update",
    };
    let presentation =
        StatusPresentation::new(IconName::Downloads, "An update is available", label);
    if method == UpdateMethod::InPlace {
        presentation.submit()
    } else {
        presentation
    }
}

fn status_detail(
    look: &Look,
    updates: &UpdatesState,
    method: UpdateMethod,
    managed: Option<&ManagedInstall>,
) -> El {
    let kit = &look.kit;
    match &updates.phase {
        UpdatePhase::Idle => plain_status(
            look,
            installed_version_status(&installed_version(), build_kind(), method, managed),
        ),
        UpdatePhase::Checking { .. } => plain_status(look, tr("Checking for updates…")),
        UpdatePhase::UpToDate => plain_status(
            look,
            rust_i18n::t!(
                "Up to date — version %{value1}",
                value1 = installed_version()
            )
            .into_owned(),
        ),
        UpdatePhase::Available {
            release,
            method: offer_method,
            returns_to_stable,
            managed_status,
            install_busy,
            ..
        } => available_detail(
            look,
            release,
            (*offer_method, *returns_to_stable, *install_busy),
            (managed, managed_status.as_deref()),
        ),
        UpdatePhase::Installing {
            progress,
            cancelling,
            finalizing,
            ..
        } => installing_detail(look, progress, *cancelling, *finalizing),
        UpdatePhase::Installed { returns_to_stable } => plain_status(
            look,
            tr(if *returns_to_stable {
                "Stable release installed — restart to apply"
            } else {
                "Update installed — restart to apply"
            }),
        ),
        UpdatePhase::InstallCancelled => plain_status(look, tr("Update cancelled")),
        UpdatePhase::InstallFailed { message } => text(install_failure_message(message.clone()))
            .key(STATUS_TEXT_KEY)
            .font(kit.ty.small)
            .color(kit.c.danger.fg)
            .wrap(),
        UpdatePhase::CheckFailed { message } => {
            let localized = failed_status_localized(message);
            status_with_link(look, &localized, LATEST_RELEASES_URL)
        }
    }
}

fn available_detail(
    look: &Look,
    release: &ReleaseMetadata,
    (offer_method, returns_to_stable, install_busy): (UpdateMethod, bool, bool),
    (managed, managed_status): (Option<&ManagedInstall>, Option<&str>),
) -> El {
    let kit = &look.kit;
    let localized = available_status_localized(
        &release.url,
        &release.version,
        offer_method,
        returns_to_stable,
    );
    let mut stack =
        col()
            .w_full()
            .min_w(0.0)
            .gap(4.0)
            .child(status_with_link(look, &localized, &release.url));
    if let Some(managed) = managed {
        stack = stack.child(
            text(managed.update_instruction())
                .font(kit.ty.small)
                .color(kit.c.text_muted)
                .wrap(),
        );
    }
    if install_busy {
        stack = stack.child(
            text(tr(
                "Another install is already running — try again shortly.",
            ))
            .font(kit.ty.small)
            .color(kit.c.danger.fg)
            .wrap(),
        );
    } else if let Some(extra) = managed_status {
        stack = stack.child(
            text(extra.to_owned())
                .font(kit.ty.small)
                .color(kit.c.text_muted)
                .wrap(),
        );
    }
    stack
}

fn installing_detail(
    look: &Look,
    progress: &InstallProgressState,
    cancelling: bool,
    finalizing: bool,
) -> El {
    let message = if finalizing {
        tr(FINALIZING_STATUS)
    } else if cancelling {
        tr("Cancelling…")
    } else {
        progress.status.clone()
    };
    plain_status(look, message)
}

fn plain_status(look: &Look, message: String) -> El {
    text(message)
        .key(STATUS_TEXT_KEY)
        .font(look.kit.ty.small)
        .color(look.kit.c.text_muted)
        .wrap()
}

fn status_with_link(look: &Look, localized: &str, url: &str) -> El {
    let kit = &look.kit;
    let Some((prefix, label, suffix)) = split_anchor(localized, url) else {
        return plain_status(look, localized.to_owned());
    };
    let mut line = row()
        .key(STATUS_TEXT_KEY)
        .w_full()
        .min_w(0.0)
        .flex_wrap()
        .items_center();
    if !prefix.is_empty() {
        line = line.child(
            text(prefix.to_owned())
                .font(kit.ty.small)
                .color(kit.c.text_muted),
        );
    }
    line = line.child(link_button(
        look,
        STATUS_LINK_KEY,
        label.to_owned(),
        url.to_owned(),
    ));
    if !suffix.is_empty() {
        line = line.child(
            text(suffix.to_owned())
                .font(kit.ty.small)
                .color(kit.c.text_muted),
        );
    }
    line
}

fn link_button(look: &Look, key: &'static str, label: String, url: String) -> El {
    let kit = &look.kit;
    let btn = el()
        .role("link")
        .label(label.clone())
        .px(2.0)
        .radius(look.round(4.0))
        .hover(kit.c.hover)
        .focusable(true)
        .focus_ring(2.0, 0.0, kit.c.border_focus)
        .cursor(Cursor::Pointer)
        .child(
            text(label)
                .font(kit.ty.small)
                .color(kit.c.text_accent)
                .a11y_hidden(),
        )
        .on_press_tx(move |app: &mut App, _: &mut Tx| {
            let _ = open_uri(app, &url);
        });
    buttons::on_return(key, btn)
}

fn progress_bar(look: &Look, updates: &UpdatesState) -> Option<El> {
    let UpdatePhase::Installing {
        progress: InstallProgressState { fraction, status },
        ..
    } = &updates.phase
    else {
        return None;
    };
    let kit = &look.kit;
    let fill_frac = fraction.unwrap_or(0.25).clamp(0.0, 1.0);
    let fill = el()
        .h_full()
        .w_frac(fill_frac)
        .radius(look.round(3.0))
        .fill(kit.c.border_active);
    let track = el()
        .w_full()
        .h(6.0)
        .radius(look.round(3.0))
        .fill(kit.alpha(kit.c.bg_base, 0.45))
        .border(1.0, control_border(look))
        .clip_soft(0.0)
        .child(fill)
        .a11y_hidden();
    let mut bar = col()
        .key(PROGRESS_BAR_KEY)
        .role("progressbar")
        .label(tr("Install update"))
        .description(status.clone())
        .w_full()
        .child(track);
    if let Some(f) = fraction {
        bar = bar.range_value(f64::from(*f), 0.0, 1.0);
    }
    Some(bar)
}

fn release_notes_preview(look: &Look, release: &ReleaseMetadata) -> El {
    let kit = &look.kit;
    let mut header = row()
        .w_full()
        .min_w(0.0)
        .gap(look.space.sm)
        .items_center()
        .child(
            text(
                rust_i18n::t!("Available release · v%{version}", version = release.version)
                    .into_owned(),
            )
            .font(kit.ty.small_medium)
            .color(kit.c.text_base)
            .grow(1.0)
            .min_w(0.0),
        );
    if release.kind != BuildKind::Stable {
        header = header.child(
            text(release.kind.localized_label())
                .font(kit.ty.small)
                .color(kit.c.text_accent),
        );
    }
    let preview = truncated_notes(&release.notes);
    col()
        .key(NOTES_CARD_KEY)
        .w_full()
        .min_w(0.0)
        .px(12.0)
        .py(10.0)
        .gap(6.0)
        .radius(look.round(8.0))
        .fill(look.fill.muted)
        .border(1.0, control_border(look))
        .child(header)
        .child(
            text(preview)
                .font(kit.ty.small)
                .color(kit.c.text_muted)
                .wrap(),
        )
        .child(link_button(
            look,
            "settings-update-notes-github",
            tr("View on GitHub"),
            release.url.clone(),
        ))
}

fn truncated_notes(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.chars().count() <= MAX_PREVIEW_NOTES_CHARS {
        return trimmed.to_owned();
    }
    let cut: String = trimmed.chars().take(MAX_PREVIEW_NOTES_CHARS).collect();
    format!("{}…", cut.trim_end())
}

fn release_channel_menu(
    ctx: PageCtx<'_>,
    channel: Channel,
    disabled: bool,
    menu_layer: &mut Vec<El>,
) -> El {
    let choices = [
        ("Stable", Channel::Stable),
        ("release_channel.preview", Channel::Preview),
        ("Nightly", Channel::Nightly),
    ];
    let current = choices
        .iter()
        .find_map(|&(l, c)| (c == channel).then(|| tr(l)))
        .unwrap_or_else(|| tr("Stable"));
    choice_menu(
        ctx,
        ChoiceSpec {
            id: "release-channel",
            title: "Release channel",
            current,
            disabled,
            options: choices
                .into_iter()
                .map(|(label, ch)| {
                    ChoiceOption::new(tr(label), channel == ch, move |app, tx| {
                        let mut changed = false;
                        shell::saved(app).mutate(|s| {
                            changed = s.set_release_channel(ch);
                            changed
                        });
                        if changed {
                            shell::saved(app).commit();
                            on_channel_changed(app, tx);
                        }
                    })
                })
                .collect(),
        },
        menu_layer,
    )
}
