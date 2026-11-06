// SPDX-License-Identifier: MIT

use super::*;

const ALT: ModifierType = ModifierType::ALT_MASK;

fn strip(browser: &crate::app::Browser) -> Vec<Location> {
    (0..)
        .map_while(|depth| browser.location_at(depth))
        .collect()
}

#[test]
fn alt_down_makes_the_focused_column_first_only_in_columns() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::first_column::alt_down_makes_the_focused_column_first_only_in_columns",
        || {
            let fixture = KeyboardFixture::new();
            let browser = fixture.view.browser();
            let root = fixture._directory.path().to_path_buf();
            std::fs::create_dir_all(root.join("alpha/sub")).expect("fixture folders");
            std::fs::write(root.join("alpha/sub/x.txt"), b"x").expect("fixture file");
            browser.refresh_all();
            wait_until(|| rendered_name(&fixture.view.widget(), "alpha"));
            for (depth, folder) in ["alpha", "alpha/sub"].into_iter().enumerate() {
                browser.descend(depth, Location::local(root.join(folder)));
                wait_loaded(&browser, depth + 1);
            }
            let whole = strip(&browser);
            assert_eq!(whole.len(), 3, "{whole:?}");
            browser.set_active_column(1);
            focus_files(&fixture);
            wait_until(|| focused_widget_shows(&fixture.window, "sub"));

            assert!(fixture.press(Key::Down, ALT));
            wait_loaded(&browser, 0);
            wait_loaded(&browser, 1);
            wait_until_or(
                || focused_depth(&browser) == Some(0) && focused_name(&browser) == "sub",
                || {
                    format!(
                        "cursor at {:?} on {:?}",
                        focused_depth(&browser),
                        focused_name(&browser)
                    )
                },
            );

            assert_eq!(strip(&browser), whole[1..]);

            assert!(fixture.press(Key::Left, ALT));
            wait_loaded(&browser, 2);
            assert_eq!(strip(&browser), whole);

            browser.set_active_column(1);
            fixture.view.set_view_mode(BrowserMode::List);
            focus_files(&fixture);
            fixture.press(Key::Down, ALT);
            pump(200);
            assert_eq!(strip(&browser), whole, "List keeps the strip");
        },
    );
}
