// SPDX-License-Identifier: MIT

use rune::{El, LiveRegion, TextEdit, el, text_input};
use strata_qa::Backend as _;

use super::super::{backend::Rune, input_log::InputLog};
use crate::{
    App,
    slots::{Entry, Frame, Place},
    test_support::{hosted_over, idle_view, view::idle},
};

const MODULE: &str = "qa::stage::tests";

fn view_entries(view: fn(&App, &mut Frame<'_, App>) -> Option<El>) -> Vec<Entry<App>> {
    struct View;
    vec![Entry {
        name: "made-view",
        kind: std::any::TypeId::of::<View>,
        place: Place::View(idle_view("made-view", view, |_| true)),
        listen: |_| None,
        publish: |_| {},
        caught_up: |_| {},
        customized: |_| {},
        poll: |_, _, _| false,
        switched: |_, _| {},
        tab_hidden: |_, _| {},
        tab_shown: |_, _| {},
        commands: &[],
        keyboard: |_| None,
    }]
}

fn live_tree_parts(app: &App, _: &mut Frame<'_, App>) -> Option<El> {
    let mut edit = TextEdit::new("one\ntwo");
    edit.focused = true;
    edit.anchor = 4;
    edit.focus = 7;
    let notes = text_input(&edit, app.look.kit.ty.ui)
        .key("made-notes")
        .label("Notes")
        .w(200.0)
        .h(80.0);
    let busy_preview = el()
        .key("made-preview")
        .role("region")
        .label("Model preview")
        .busy(true)
        .w(200.0)
        .h(120.0);
    let footer_status = el()
        .key("made-status")
        .role("status")
        .label("Files on clipboard")
        .w(180.0)
        .h(24.0);
    Some(
        el().key("made-view")
            .child(busy_preview)
            .child(footer_status)
            .child(notes),
    )
}

fn live_tree_entries() -> Vec<Entry<App>> {
    view_entries(live_tree_parts)
}

#[test]
fn runtime_busy_multiline_selection_and_announcer_filtering_hold_in_stage() {
    let name = "runtime_busy_multiline_selection_and_announcer_filtering_hold_in_stage";
    hosted_over(MODULE, name, live_tree_entries, |mut host| {
        idle(&mut host);
        host.runtime().update(|_, tx| {
            tx.focus("made-notes", false);
            tx.announce("Copied", LiveRegion::Polite);
        });
        idle(&mut host);

        let backend = Rune::new(host, InputLog::from_environment().expect("input log"));
        let root = backend.window().expect("window");
        let mut stack = vec![root];
        let mut nodes = Vec::new();
        while let Some(node) = stack.pop() {
            stack.extend(backend.children(&node));
            nodes.push(node);
        }

        let statuses: Vec<_> = nodes
            .iter()
            .filter(|node| backend.role(node) == "status bar")
            .map(|node| backend.name(node))
            .collect();
        assert_eq!(statuses, ["Files on clipboard"]);

        let preview = nodes
            .iter()
            .find(|node| backend.name(node) == "Model preview")
            .expect("Model preview");
        assert!(backend.states(preview).busy);
        assert!(!backend.states(&nodes[0]).busy);

        let notes = nodes
            .iter()
            .find(|node| backend.name(node) == "Notes")
            .expect("Notes field");
        let field = backend.field(notes).expect("focused multiline field");
        assert_eq!(field.text, "one\ntwo");
        assert_eq!((field.selection, field.caret), (Some([4, 7]), 7));
    });
}
