// SPDX-License-Identifier: MIT

use super::super::targets;

#[test]
fn a_native_toggle_reports_its_own_state_over_a_recorded_one() {
    crate::test_support::gtk_test(
        "qa::tests::bridge::a_native_toggle_reports_its_own_state_over_a_recorded_one",
        || {
            use gtk::prelude::*;
            let toggle = gtk::ToggleButton::new();
            toggle.set_active(true);
            crate::ui::accessibility_qa::record(&toggle, |naming| naming.pressed = Some(false));
            assert!(targets::states(toggle.upcast_ref()).pressed);

            let plain = gtk::Button::new();
            crate::ui::accessibility_qa::record(&plain, |naming| {
                naming.pressed = Some(true);
                naming.checked = Some(true);
                naming.busy = Some(true);
            });
            let states = targets::states(plain.upcast_ref());
            assert!(states.pressed && states.checked && states.busy);

            let switch = gtk::Switch::new();
            switch.set_active(true);
            assert!(targets::states(switch.upcast_ref()).checked);

            let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            row.append(&content);
            assert!(!targets::states(row.upcast_ref()).marked);
            content.add_css_class(crate::ui::ENTER_TARGET_CLASS);
            assert!(
                targets::states(row.upcast_ref()).marked,
                "an entry row reports the Enter mark its content carries"
            );
        },
    );
}

#[test]
fn notebook_tabs_and_scrolled_text_report_what_the_bridge_does() {
    crate::test_support::gtk_test(
        "qa::tests::bridge::notebook_tabs_and_scrolled_text_report_what_the_bridge_does",
        || {
            use gtk::prelude::*;
            let notebook = gtk::Notebook::new();
            for title in ["General", "Script"] {
                notebook.append_page(
                    &gtk::Box::new(gtk::Orientation::Vertical, 0),
                    Some(&gtk::Label::new(Some(title))),
                );
            }
            notebook.set_current_page(Some(1));
            let selected: Vec<(String, bool)> = targets::presented(notebook.upcast_ref())
                .iter()
                .filter(|widget| targets::role_name(widget) == "page tab")
                .map(|tab| (targets::accessible_name(tab), targets::states(tab).selected))
                .collect();
            assert_eq!(
                selected,
                [("General".to_owned(), false), ("Script".to_owned(), true)]
            );

            let view = gtk::TextView::new();
            let scroll = gtk::ScrolledWindow::builder().child(&view).build();
            crate::ui::accessibility_qa::record(&scroll, |naming| {
                naming.label = Some("Arguments".to_owned())
            });
            assert_eq!(targets::accessible_name(view.upcast_ref()), "Arguments");
            assert_eq!(targets::accessible_name(scroll.upcast_ref()), "");
            assert!(targets::states(view.upcast_ref()).editable);
            let plain = gtk::TextView::new();
            let _unlabelled = gtk::ScrolledWindow::builder().child(&plain).build();
            assert_eq!(targets::accessible_name(plain.upcast_ref()), "");
        },
    );
}

#[test]
fn buttons_alerts_passwords_and_list_rows_report_what_the_bridge_does() {
    crate::test_support::gtk_test(
        "qa::tests::bridge::buttons_alerts_passwords_and_list_rows_report_what_the_bridge_does",
        || {
            use gtk::prelude::*;
            let name = |widget: &gtk::Widget| targets::accessible_name(widget);
            let labelled = gtk::Button::with_label("Home directory");
            crate::ui::accessibility_qa::record(&labelled, |naming| {
                naming.label = Some("Default directory".to_owned());
            });
            assert_eq!(name(labelled.upcast_ref()), "Home directory");
            let reset = gtk::Button::with_label("r");
            crate::ui::accessibility_qa::record_labelled_by_reset(&reset);
            crate::ui::accessibility_qa::record(&reset, |naming| {
                naming.label = Some("Owner read".to_owned());
            });
            assert_eq!(name(reset.upcast_ref()), "Owner read");
            let icon = gtk::Button::from_icon_name("edit-find");
            crate::ui::accessibility_qa::record(&icon, |naming| {
                naming.label = Some("Search".to_owned());
            });
            assert_eq!(name(icon.upcast_ref()), "Search");

            let alert = gtk::Label::builder()
                .accessible_role(gtk::AccessibleRole::Alert)
                .build();
            assert_eq!(targets::role_name(alert.upcast_ref()), "alert");
            let password = gtk::PasswordEntry::new();
            assert_eq!(targets::role_name(password.upcast_ref()), "password text");

            let row = gtk::Box::builder()
                .accessible_role(gtk::AccessibleRole::ListItem)
                .build();
            assert!(!targets::states(row.upcast_ref()).selected);
            row.set_state_flags(gtk::StateFlags::SELECTED, false);
            assert!(targets::states(row.upcast_ref()).selected);
            let plain = gtk::Box::new(gtk::Orientation::Vertical, 0);
            plain.set_state_flags(gtk::StateFlags::SELECTED, false);
            assert!(!targets::states(plain.upcast_ref()).selected);
        },
    );
}
