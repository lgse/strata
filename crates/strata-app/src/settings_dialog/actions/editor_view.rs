// SPDX-License-Identifier: MIT

//! Rendering of the 3-tab Custom Action editor modal (`General` / `Script` / `Behavior`),
//! built on C15's [`StrataDialog`], C20c's [`TabbedForm`], C10b's [`CodeEditor`], and
//! Strata's `Look`-styled controls.

use std::rc::Rc;

use rune::{Color, Cursor, El, KeyEvent, LiveRegion, ScrollSpec, TextEdit, Tx, col, row, text};
use rune_kit::{Checkbox, TextInput};
use strata_components::{
    code_view::editor::CodeEditor,
    controls::{
        switch::Switch,
        tabbed_form::{FormLabels, FormPage, Mounting, TabbedForm},
    },
    dialog::{StrataDialog, keep_words_whole, text_area},
};
use strata_core::{
    i18n::tr,
    model::{ActionRuntime, ErrorPolicy, ExecutionMode, MenuPlacement, WorkingDirectory},
};
use strata_kit::look::Look;

use super::{
    close_editor, confirm_editor,
    editor::{
        BEHAVIOR_TAB, EDITOR_ARGUMENTS_KEY, EDITOR_DESCRIPTION_KEY, EDITOR_ENTRYPOINT_KEY,
        EDITOR_EXTENSIONS_KEY, EDITOR_FILES_KEY, EDITOR_ID_KEY, EDITOR_MAX_ITEMS_KEY,
        EDITOR_NAME_KEY, EDITOR_PROGRAM_KEY, EditorMode, EditorSession, GENERAL_TAB, SCRIPT_TAB,
        default_entrypoint,
    },
    examples, icon_chooser, readiness,
};
use crate::{App, buttons, custom_actions::action_icon};

const EDITOR_DIALOG_WIDTH: f32 = 760.0;
const EDITOR_BODY_HEIGHT: f32 = 480.0;

type PickHandler = Rc<dyn Fn(&mut App, &mut Tx)>;
type SegmentOption = (String, bool, PickHandler);

pub(super) fn tabbed_form() -> TabbedForm<App> {
    TabbedForm::new(
        "settings-action-editor-tabs",
        |app: &App| {
            &app.settings_dialog
                .actions
                .editor
                .as_ref()
                .expect("editor open")
                .tabs
        },
        |app: &mut App| {
            &mut app
                .settings_dialog
                .actions
                .editor
                .as_mut()
                .expect("editor open")
                .tabs
        },
    )
    .mounting(Mounting::Rebuilt)
}

pub(super) fn build_dialog(app: &App, layer: &mut Vec<El>) {
    let Some(session) = app.settings_dialog.actions.editor.as_ref() else {
        return;
    };
    let look = &app.look;
    let (title_key, confirm_key) = match session.mode {
        EditorMode::Create => ("New action", "Create action"),
        EditorMode::Edit => ("Edit action", "Save changes"),
    };
    let title = tr(title_key);
    let pages = [
        FormPage::new("general", tr("General")).errors(session.error_tab == Some(GENERAL_TAB)),
        FormPage::new("script", tr("Script")).errors(session.error_tab == Some(SCRIPT_TAB)),
        FormPage::new("behavior", tr("Behavior")).errors(session.error_tab == Some(BEHAVIOR_TAB)),
    ];
    let form = tabbed_form().labels(FormLabels {
        tabs: title.clone(),
        has_errors: session.error.clone().unwrap_or_default(),
    });
    let mut body = col()
        .w_full()
        .min_h(EDITOR_BODY_HEIGHT)
        .on_key(|app: &mut App, ev: &KeyEvent, tx: &mut Tx| {
            tabbed_form().window_key(app, 3, ev, tx)
        })
        .child(form.build(app, look, &pages, build_tab));
    if let Some(error) = &session.error {
        body = body.child(error_banner(look, error));
    }
    StrataDialog::new(
        format!("settings-action-editor-{}", session.generation),
        title,
        close_editor,
    )
    .open(true)
    .stacked(true)
    .blocking(true)
    .width(EDITOR_DIALOG_WIDTH)
    .icon(action_icon(session.selected_icon.as_deref()))
    .subtitle(tr(
        "Saved as action.toml under ~/.config/strata/actions/<id>",
    ))
    .child(body)
    .secondary(tr("Cancel"), close_editor)
    .primary_enabled(!session.saving)
    .primary_tx(tr(confirm_key), confirm_editor)
    .close_label(tr("Close dialog"))
    .build(look, layer);

    icon_chooser::build_popover(look, session, layer);
    examples::build_popover(look, session, layer);
}

fn build_tab(app: &App, index: usize) -> El {
    let look = &app.look;
    let session = app
        .settings_dialog
        .actions
        .editor
        .as_ref()
        .expect("editor open");
    let content = match index {
        GENERAL_TAB => general_tab(look, session),
        SCRIPT_TAB => script_tab(app, look, session),
        _ => behavior_tab(look, session),
    };
    col()
        .w_full()
        .max_h(EDITOR_BODY_HEIGHT - 44.0)
        .pt(16.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(content)
}

fn general_tab(look: &Look, session: &EditorSession) -> El {
    let description_input = single_line_input(
        look,
        InputSpec {
            key: EDITOR_DESCRIPTION_KEY,
            label: tr("Description"),
            placeholder: tr("Describes this action to assistive technology"),
            description: tr("Optional accessible description"),
            disabled: false,
            width: None,
            edit: &session.description,
            on_edit: |app| &mut editor_mut(app).description,
        },
    );
    let icon_field = field_block(
        look,
        "Icon",
        "Bundled Lucide icon",
        icon_chooser::build_row(look, session),
    );
    let enabled_switch = Switch::new(session.enabled)
        .label(tr("Enabled"))
        .description(tr("Show this action in menus"))
        .on_toggle(|app: &mut App| {
            let editor = editor_mut(app);
            editor.enabled = !editor.enabled;
        })
        .build(look)
        .key("settings-action-editor-enabled");
    let enabled_row = col()
        .w_full()
        .border_sides([1.0, 0.0, 0.0, 0.0], option_divider(look))
        .child(option_row(
            look,
            OptionRow {
                title: "Enabled",
                description: "Show this action in menus",
                control: enabled_switch,
                disabled: false,
            },
        ));
    col()
        .w_full()
        .gap(18.0)
        .child(identity_row(look, session))
        .child(field_block(
            look,
            "Description",
            "Optional accessible description",
            description_input,
        ))
        .child(icon_field)
        .child(enabled_row)
}

fn identity_row(look: &Look, session: &EditorSession) -> El {
    let name_input = single_line_input(
        look,
        InputSpec {
            key: EDITOR_NAME_KEY,
            label: tr("Name"),
            placeholder: tr("Batch rename"),
            description: tr("Shown in the context menu"),
            disabled: false,
            width: None,
            edit: &session.name,
            on_edit: |app| &mut editor_mut(app).name,
        },
    );
    let id_desc = tr(if session.mode == EditorMode::Create {
        "Folder name: lowercase letters, digits, and dashes. Leave blank to use the name."
    } else {
        "The action id cannot change; duplicate it to create a variant."
    });
    let id_input = single_line_input(
        look,
        InputSpec {
            key: EDITOR_ID_KEY,
            label: tr("Id"),
            placeholder: session.id_placeholder(),
            description: id_desc,
            disabled: session.mode == EditorMode::Edit,
            width: None,
            edit: &session.id,
            on_edit: |app| &mut editor_mut(app).id,
        },
    );
    row()
        .w_full()
        .gap(18.0)
        .child(field_block(look, "Name", "Shown in the context menu", name_input).grow(1.0))
        .child(field_block(look, "Id", "Folder name", id_input).w(240.0))
}

fn script_tab(app: &App, look: &Look, session: &EditorSession) -> El {
    let runtime_bar = segmented_choice(
        look,
        SegmentSpec {
            key_prefix: "settings-action-editor-runtime",
            group_label: Some(tr("Runtime")),
            disabled: false,
            choices: vec![
                (
                    tr("Python"),
                    session.runtime == ActionRuntime::Python,
                    Rc::new(|app, _| set_runtime(app, ActionRuntime::Python)),
                ),
                (
                    tr("Bash"),
                    session.runtime == ActionRuntime::Bash,
                    Rc::new(|app, _| set_runtime(app, ActionRuntime::Bash)),
                ),
                (
                    tr("Command"),
                    session.runtime == ActionRuntime::Command,
                    Rc::new(|app, _| set_runtime(app, ActionRuntime::Command)),
                ),
            ],
        },
    );
    let is_script = session.runtime != ActionRuntime::Command;
    let secondary_field = launch_target_field(look, session, is_script);
    let top_row = row()
        .w_full()
        .gap(18.0)
        .child(field_block(look, "Runtime", "How it starts", runtime_bar))
        .child(secondary_field.grow(1.0));
    let body_section = if is_script {
        script_editor_section(app, look, session)
    } else {
        arguments_section(look, session)
    };
    col().w_full().gap(18.0).child(top_row).child(body_section)
}

fn launch_target_field(look: &Look, session: &EditorSession, is_script: bool) -> El {
    if is_script {
        let entry = single_line_input(
            look,
            InputSpec {
                key: EDITOR_ENTRYPOINT_KEY,
                label: tr("Script file"),
                placeholder: default_entrypoint(session.runtime).to_owned(),
                description: tr("Saved beside action.toml"),
                disabled: false,
                width: None,
                edit: &session.entrypoint,
                on_edit: |app| &mut editor_mut(app).entrypoint,
            },
        );
        field_block(look, "Script file", "Saved beside action.toml", entry)
    } else {
        let program = single_line_input(
            look,
            InputSpec {
                key: EDITOR_PROGRAM_KEY,
                label: tr("Program"),
                placeholder: tr("Installed program, for example make"),
                description: tr("Installed executable"),
                disabled: false,
                width: None,
                edit: &session.program,
                on_edit: |app| &mut editor_mut(app).program,
            },
        );
        field_block(look, "Program", "Installed executable", program)
    }
}

fn script_editor_section(app: &App, look: &Look, session: &EditorSession) -> El {
    let mut heading = row().w_full().gap(12.0).items_center().child(
        text(tr("Script"))
            .font(look.kit.ty.ui_medium)
            .color(look.kit.c.text_base)
            .grow(1.0),
    );
    if let Some(status) = readiness::indicator(look, &session.readiness) {
        heading = heading.child(status);
    }
    heading = heading.child(examples::library_button(look, session.examples_open));
    let code = CodeEditor::new(
        |app: &App| {
            app.settings_dialog
                .actions
                .editor
                .as_ref()
                .expect("editor open")
                .active_script_editor()
        },
        |app: &mut App| {
            let later = app.settings_dialog.actions.later.downgrade();
            let editor = editor_mut(app);
            editor.refresh_readiness(later, false);
            editor.active_script_editor_mut()
        },
    )
    .label(tr("Script"))
    .rows(12)
    .build(app, look);
    col()
        .w_full()
        .gap(8.0)
        .child(heading)
        .child(col().w_full().h(260.0).child(code))
}

fn arguments_section(look: &Look, session: &EditorSession) -> El {
    let kit = &look.kit;
    let heading = row()
        .w_full()
        .gap(12.0)
        .items_center()
        .child(
            text(tr("Arguments"))
                .font(kit.ty.ui_medium)
                .color(kit.c.text_base)
                .grow(1.0),
        )
        .child(
            text(tr("One per line · {path}, {paths}, or {parent}"))
                .font(kit.ty.small)
                .color(kit.c.text_muted),
        )
        .child(examples::library_button(look, session.examples_open));
    let area = text_area(&session.arguments, |app: &mut App| {
        &mut editor_mut(app).arguments
    })
    .label(tr("Arguments"))
    .rows(8)
    .build(kit)
    .key(EDITOR_ARGUMENTS_KEY)
    .description(tr("One argument per line; {path}, {paths}, or {parent}"));
    col().w_full().gap(8.0).child(heading).child(area)
}

fn behavior_tab(look: &Look, session: &EditorSession) -> El {
    let mut rows = run_choice_rows(look, session);
    rows.extend(condition_rows(look, session));
    let divider = option_divider(look);
    let mut stack = col().w_full();
    for (idx, item) in rows.into_iter().enumerate() {
        let wrapped = if idx > 0 {
            col()
                .w_full()
                .border_sides([1.0, 0.0, 0.0, 0.0], divider)
                .child(item)
        } else {
            item
        };
        stack = stack.child(wrapped);
    }
    stack
}

fn run_choice_rows(look: &Look, session: &EditorSession) -> Vec<El> {
    let per_item = session.mode_choice == ExecutionMode::PerItem;
    let mode = segmented_choice(
        look,
        SegmentSpec {
            key_prefix: "settings-action-editor-mode",
            group_label: Some(tr("Run")),
            disabled: false,
            choices: vec![
                (
                    tr("Whole selection"),
                    session.mode_choice == ExecutionMode::WholeSelection,
                    Rc::new(|app, _| editor_mut(app).mode_choice = ExecutionMode::WholeSelection),
                ),
                (
                    tr("Per item"),
                    per_item,
                    Rc::new(|app, _| editor_mut(app).mode_choice = ExecutionMode::PerItem),
                ),
            ],
        },
    );
    let policy = segmented_choice(
        look,
        SegmentSpec {
            key_prefix: "settings-action-editor-policy",
            group_label: Some(tr("On failure")),
            disabled: !per_item,
            choices: vec![
                (
                    tr("Continue"),
                    session.policy == ErrorPolicy::Continue,
                    Rc::new(|app, _| editor_mut(app).policy = ErrorPolicy::Continue),
                ),
                (
                    tr("Stop"),
                    session.policy == ErrorPolicy::Stop,
                    Rc::new(|app, _| editor_mut(app).policy = ErrorPolicy::Stop),
                ),
            ],
        },
    );
    let working_dir = segmented_choice(
        look,
        SegmentSpec {
            key_prefix: "settings-action-editor-directory",
            group_label: Some(tr("Working folder")),
            disabled: false,
            choices: vec![
                (
                    tr("Invoking"),
                    session.working_directory == WorkingDirectory::Parent,
                    Rc::new(|app, _| editor_mut(app).working_directory = WorkingDirectory::Parent),
                ),
                (
                    tr("Home"),
                    session.working_directory == WorkingDirectory::Home,
                    Rc::new(|app, _| editor_mut(app).working_directory = WorkingDirectory::Home),
                ),
                (
                    tr("Action"),
                    session.working_directory == WorkingDirectory::Action,
                    Rc::new(|app, _| editor_mut(app).working_directory = WorkingDirectory::Action),
                ),
            ],
        },
    );
    let placement = segmented_choice(
        look,
        SegmentSpec {
            key_prefix: "settings-action-editor-placement",
            group_label: Some(tr("Placement")),
            disabled: false,
            choices: vec![
                (
                    tr("Menu item"),
                    session.placement == MenuPlacement::Top,
                    Rc::new(|app, _| editor_mut(app).placement = MenuPlacement::Top),
                ),
                (
                    tr("Actions submenu"),
                    session.placement == MenuPlacement::Submenu,
                    Rc::new(|app, _| editor_mut(app).placement = MenuPlacement::Submenu),
                ),
            ],
        },
    );
    vec![
        option_row(
            look,
            OptionRow {
                title: "Run",
                description: "Once, or once per selected item",
                control: mode,
                disabled: false,
            },
        ),
        option_row(
            look,
            OptionRow {
                title: "On failure",
                description: "Only used when running per item",
                control: policy,
                disabled: !per_item,
            },
        ),
        option_row(
            look,
            OptionRow {
                title: "Working folder",
                description: "Where the process starts",
                control: working_dir,
                disabled: false,
            },
        ),
        option_row(
            look,
            OptionRow {
                title: "Placement",
                description: "Where the action appears",
                control: placement,
                disabled: false,
            },
        ),
    ]
}

fn condition_rows(look: &Look, session: &EditorSession) -> Vec<El> {
    let kind_row = row()
        .gap(12.0)
        .items_center()
        .child(
            Checkbox::new(session.files)
                .label(tr("Files"))
                .on_toggle(|app: &mut App| {
                    let editor = editor_mut(app);
                    editor.files = !editor.files;
                })
                .build(&look.kit)
                .key(EDITOR_FILES_KEY),
        )
        .child(
            Checkbox::new(session.folders)
                .label(tr("Folders"))
                .on_toggle(|app: &mut App| {
                    let editor = editor_mut(app);
                    editor.folders = !editor.folders;
                })
                .build(&look.kit)
                .key("settings-action-editor-folders"),
        );
    let extensions = single_line_input(
        look,
        InputSpec {
            key: EDITOR_EXTENSIONS_KEY,
            label: tr("Extensions"),
            placeholder: tr("Any"),
            description: tr("Comma separated, without dots"),
            disabled: false,
            width: Some(180.0),
            edit: &session.extensions,
            on_edit: |app| &mut editor_mut(app).extensions,
        },
    );
    let max_items = single_line_input(
        look,
        InputSpec {
            key: EDITOR_MAX_ITEMS_KEY,
            label: tr("Maximum items"),
            placeholder: tr("No limit"),
            description: tr("Optional selection limit"),
            disabled: false,
            width: Some(180.0),
            edit: &session.max_items,
            on_edit: |app| &mut editor_mut(app).max_items,
        },
    );
    let confirm_switch = Switch::new(session.confirm)
        .label(tr("Confirm"))
        .description(tr("Ask before running this action"))
        .on_toggle(|app: &mut App| {
            let editor = editor_mut(app);
            editor.confirm = !editor.confirm;
        })
        .build(look)
        .key("settings-action-editor-confirm");
    vec![
        option_row(
            look,
            OptionRow {
                title: "Applies to",
                description: "Which selected entries offer this action",
                control: kind_row,
                disabled: false,
            },
        ),
        option_row(
            look,
            OptionRow {
                title: "Extensions",
                description: "Comma separated, without dots",
                control: extensions,
                disabled: false,
            },
        ),
        option_row(
            look,
            OptionRow {
                title: "Maximum items",
                description: "Optional selection limit",
                control: max_items,
                disabled: false,
            },
        ),
        option_row(
            look,
            OptionRow {
                title: "Confirm",
                description: "Ask before running this action",
                control: confirm_switch,
                disabled: false,
            },
        ),
    ]
}

struct InputSpec<'a> {
    key: &'static str,
    label: String,
    placeholder: String,
    description: String,
    disabled: bool,
    width: Option<f32>,
    edit: &'a TextEdit,
    on_edit: fn(&mut App) -> &mut TextEdit,
}

fn single_line_input(look: &Look, spec: InputSpec<'_>) -> El {
    let mut input = TextInput::new(spec.edit)
        .label(spec.label)
        .placeholder(spec.placeholder)
        .width(spec.width)
        .disabled(spec.disabled)
        .on_edit(spec.on_edit)
        .build(&look.kit)
        .key(spec.key)
        .description(spec.description);
    if spec.disabled {
        input = input.disabled(true);
    }
    if spec.width.is_none() {
        input = input.w_full();
    }
    input
}

fn field_block(look: &Look, title: &str, hint: &str, control: El) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    let mut heading = row().w_full().gap(12.0).items_center().child(
        text(tr(title))
            .font(kit.ty.ui_medium)
            .color(c.text_base)
            .grow(1.0),
    );
    if !hint.is_empty() {
        heading = heading.child(text(tr(hint)).font(kit.ty.small).color(c.text_muted));
    }
    col().w_full().gap(8.0).child(heading).child(control)
}

struct OptionRow<'a> {
    title: &'a str,
    description: &'a str,
    control: El,
    disabled: bool,
}

fn option_row(look: &Look, spec: OptionRow<'_>) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    let copy = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(2.0)
        .child(
            text(tr(spec.title))
                .font(kit.ty.ui_medium)
                .color(c.text_base),
        )
        .child(
            text(tr(spec.description))
                .font(kit.ty.small)
                .color(c.text_muted)
                .wrap(),
        );
    let mut line = row()
        .w_full()
        .min_h(44.0)
        .py(8.0)
        .gap(16.0)
        .items_center()
        .child(copy)
        .child(spec.control);
    if spec.disabled {
        line = line.layer_opacity(0.45);
    }
    line
}

struct SegmentSpec<'a> {
    key_prefix: &'a str,
    group_label: Option<String>,
    disabled: bool,
    choices: Vec<SegmentOption>,
}

fn segmented_choice(look: &Look, spec: SegmentSpec<'_>) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    let count = spec.choices.len().max(1);
    let items: Vec<El> = spec
        .choices
        .into_iter()
        .enumerate()
        .map(|(idx, choice)| segment_item(look, spec.key_prefix, idx, spec.disabled, choice))
        .collect();
    let border = kit.alpha(c.std_border, c.std_border.a * 0.45);
    let mut bar = row()
        .key(spec.key_prefix.to_owned())
        .equal_columns(count)
        .padding(2.0)
        .radius(kit.radius.md)
        .fill(kit.alpha(c.bg_base, 0.25))
        .border(1.0, border)
        .children(items);
    if let Some(label) = spec.group_label {
        bar = bar.role("group").label(label);
    }
    bar
}

fn segment_item(
    look: &Look,
    key_prefix: &str,
    idx: usize,
    disabled: bool,
    (label, active, on_pick): SegmentOption,
) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    let (fill, hover_fill, ink) = if active {
        (
            kit.alpha(c.bg_accent, 0.22),
            kit.alpha(c.bg_accent, 0.26),
            c.text_accent,
        )
    } else {
        (Color::TRANSPARENT, c.hover, c.text_muted)
    };
    let mut text_el = text(label.clone()).font(kit.ty.ui).color(ink).a11y_hidden();
    if !active && !disabled {
        text_el = text_el.group_color(c.text_base);
    }
    let mut btn = row()
        .grid_cell(idx)
        .role("button")
        .label(label)
        .checked(active)
        .min_h(28.0)
        .px(12.0)
        .py(3.0)
        .center()
        .radius(look.round(4.0))
        .fill(fill)
        .focusable(!disabled)
        .disabled(disabled)
        .child(text_el);
    if disabled {
        btn.key(format!("{key_prefix}-{idx}"))
    } else {
        btn = btn
            .hover_group()
            .hover(hover_fill)
            .press(kit.alpha(c.bg_accent, 0.28))
            .focus_ring(2.0, -2.0, c.border_focus)
            .cursor(Cursor::Pointer)
            .on_press_tx(move |app: &mut App, tx: &mut Tx| on_pick(app, tx));
        buttons::on_return(format!("{key_prefix}-{idx}"), btn)
    }
}

fn error_banner(look: &Look, message: &str) -> El {
    text(keep_words_whole(message))
        .key("settings-action-editor-error")
        .label(message.to_owned())
        .font(look.kit.ty.small)
        .color(look.kit.c.danger.fg)
        .pt(8.0)
        .wrap()
        .live_region(LiveRegion::Polite)
}

fn option_divider(look: &Look) -> Color {
    look.kit
        .alpha(look.kit.c.std_border, look.kit.c.std_border.a * 0.30)
}

fn set_runtime(app: &mut App, runtime: ActionRuntime) {
    let later = app.settings_dialog.actions.later.downgrade();
    if let Some(editor) = app.settings_dialog.actions.editor.as_mut() {
        editor.set_runtime(runtime, later);
    }
}

fn editor_mut(app: &mut App) -> &mut EditorSession {
    app.settings_dialog
        .actions
        .editor
        .as_mut()
        .expect("editor open")
}
