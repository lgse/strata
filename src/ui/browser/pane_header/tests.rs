// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn retiring_a_sort_button_releases_its_popover() {
    crate::test_support::gtk_test(
        "ui::browser::pane_header::tests::retiring_a_sort_button_releases_its_popover",
        || {
            let browser = Browser::new(Rc::new(crate::adapters::LocalFileSource));
            let button = column_sort_menu(&browser, 0);
            let popover = button.popover().expect("sort popover").downgrade();
            drop(button);
            assert!(
                popover.upgrade().is_none(),
                "retired columns must not retain sort popovers and their themed icons",
            );
        },
    );
}
