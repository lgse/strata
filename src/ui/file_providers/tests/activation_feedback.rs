// SPDX-License-Identifier: MIT
use super::*;

fn overlay_layers(overlay: &gtk::Overlay) -> Vec<gtk::Widget> {
    std::iter::successors(overlay.first_child(), gtk::Widget::next_sibling)
        .filter(|widget| overlay.child().as_ref() != Some(widget))
        .collect()
}

fn feedback_overlay() -> gtk::Overlay {
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&gtk::Label::new(None)));
    overlay
}

/// Sends the activation `action` for `/file`, owned by `overlay`.
fn activate(overlay: &gtk::Overlay, action: &str) {
    let owner: gtk::Widget = overlay.clone().upcast();
    with_hub(|hub| {
        let mut activation = request("activate", vec!["/file".into()], false);
        activation.action = Some(action.into());
        hub.send(0, activation, Pending::Activate(0, owner.downgrade()));
    });
}

/// The overlay shows `notice` notices in its operation docks and `dialog` other layers.
fn assert_feedback(overlay: &gtk::Overlay, action: &str, notice: bool, dialog: bool) {
    let (docks, dialogs): (Vec<_>, Vec<_>) = overlay_layers(overlay)
        .into_iter()
        .partition(|layer| layer.has_css_class("file-operation-dock"));
    let notices = docks
        .iter()
        .filter_map(|dock| dock.first_child()?.first_child()?.first_child())
        .filter(|row| row.has_css_class("overlay-notice"))
        .count();
    assert_eq!(notices, usize::from(notice), "{action} notice");
    assert_eq!(dialogs.len(), usize::from(dialog), "{action} dialog");
}

const REPLYING_PROVIDER: &str = r#"import json,sys
replies={
 'notice':{'outcome':{'status':'accepted','accepted':1,'total':1},'feedback':'notice'},
 'quiet':{'outcome':{'status':'accepted','accepted':1,'total':1},'feedback':'none'},
 'legacy':{},
 'rejected':{'outcome':{'status':'rejected','accepted':0,'total':1},'feedback':'notice'},
}
for line in sys.stdin:
 r=json.loads(line)
 print(json.dumps({'version':1,'id':r['id'],'message':'Link copied',**replies[r['action']]}),flush=True)
"#;

#[test]
fn activation_feedback_routes_success_to_notice_and_problems_to_dialog() {
    crate::test_support::gtk_test(
        "ui::file_providers::tests::activation_feedback::activation_feedback_routes_success_to_notice_and_problems_to_dialog",
        || {
            install_hub();
            with_hub(|hub| start_scripted_provider(hub, REPLYING_PROVIDER));
            for (action, notice, dialog) in [
                ("notice", true, false),
                ("quiet", false, false),
                ("legacy", false, true),
                ("rejected", false, true),
            ] {
                let overlay = feedback_overlay();
                activate(&overlay, action);
                let deadline = Deadline::start();
                while with_hub(|hub| !hub.pending.is_empty()) {
                    deadline.check(&format!("{action} reply was not handled"));
                    with_hub(Hub::tick);
                    nap();
                }
                assert_feedback(&overlay, action, notice, dialog);
            }
        },
    );
}

/// Holds every request until `release` exists; `exit` then ends the provider,
/// anything else gets an accepted notice.
fn held_provider_script(release: &Path) -> String {
    format!(
        r#"import json,os,sys,time
for line in sys.stdin:
 r=json.loads(line)
 while not os.path.exists({release:?}): time.sleep(0.01)
 if r['action']=='exit': sys.exit()
 print(json.dumps({{'version':1,'id':r['id'],'message':'Link copied','outcome':{{'status':'accepted','accepted':1,'total':1}},'feedback':'notice'}}),flush=True)
"#
    )
}

fn reload_to_no_providers() {
    let (sender, discovery) = mpsc::channel();
    with_hub(|hub| hub.reload(discovery));
    sender.send(Vec::new()).expect("discovery result");
}

#[test]
fn a_reload_still_shows_the_outcome_of_a_pending_activation() {
    crate::test_support::gtk_test(
        "ui::file_providers::tests::activation_feedback::a_reload_still_shows_the_outcome_of_a_pending_activation",
        || {
            for (action, notice, dialog) in [("reply", true, false), ("exit", false, true)] {
                install_hub();
                let fixture = tempfile::tempdir().expect("fixture");
                let release = fixture.path().join("release");
                with_hub(|hub| start_scripted_provider(hub, &held_provider_script(&release)));
                let overlay = feedback_overlay();
                activate(&overlay, action);
                reload_to_no_providers();
                std::fs::write(&release, "go").expect("release provider");

                let deadline = Deadline::start();
                while tick_hub() == glib::ControlFlow::Continue {
                    deadline.check(&format!(
                        "{action}: the pending activation was not settled after a reload"
                    ));
                    nap();
                }
                assert_feedback(&overlay, action, notice, dialog);
                clear_hub();
            }
        },
    );
}
