// SPDX-License-Identifier: MIT

use super::*;
use vte4::prelude::*;

fn terminal(pane: &crate::ui::window::composition::panes::Pane) -> vte4::Terminal {
    if pane.terminal.widget.is_visible() {
        wait_until(|| pane.terminal.is_running());
    }
    find_widget(pane.terminal.widget.upcast_ref(), &|widget| {
        widget.is::<vte4::Terminal>()
    })
    .expect("embedded terminal")
    .downcast()
    .expect("VTE")
}

fn command(terminal: &vte4::Terminal, command: &str) {
    terminal.feed_child(format!("{command}\n").as_bytes());
}

fn await_contents(path: impl AsRef<std::path::Path>, expected: &[u8]) {
    let path = path.as_ref();
    wait_until_or(
        || std::fs::read(path).is_ok_and(|bytes| bytes == expected),
        || {
            format!(
                "shell did not write the expected contents to {}",
                path.display()
            )
        },
    );
}

fn await_directory(path: &std::path::Path, expected: &std::path::Path) {
    use std::os::unix::ffi::OsStrExt;
    let mut bytes = expected.as_os_str().as_bytes().to_vec();
    bytes.push(b'\n');
    await_contents(path, &bytes);
}

#[test]
fn pane_shells_keep_their_directory_and_session_when_hidden() {
    gtk_test(
        "ui::window::composition::tabs::tests::panes::terminal::pane_shells_keep_their_directory_and_session_when_hidden",
        || {
            let split = Split::open();
            let content = split.content();
            let main = content.panes.main().clone();
            content.header.terminal.emit_clicked();
            let first = terminal(&main);
            let marker = split.root.path().join("left/terminal-started");
            command(&first, "export STRATA_SESSION=kept; pwd > terminal-started");
            await_directory(&marker, &split.root.path().join("left"));
            split.key(Key::F4, M::empty());
            assert!(!main.terminal.widget.is_visible());
            load(&split.main(), split.folder("right"));
            split.key(Key::F4, M::empty());
            command(
                &first,
                "printf '%s' \"$STRATA_SESSION\" > terminal-reopened",
            );
            let retained = split.root.path().join("left/terminal-reopened");
            await_contents(retained, b"kept");
            content.header.extra_pane.emit_clicked();
            let extra = content.panes.active();
            extra.terminal.button.emit_clicked();
            let second = terminal(&extra);
            command(&second, "pwd > second-started");
            let other = split.root.path().join("right/second-started");
            await_directory(&other, &split.root.path().join("right"));
            command(&first, "exit");
            wait_until(|| !main.terminal.widget.is_visible());
            assert!(
                second.has_focus(),
                "an exiting shell in the other pane must not steal focus"
            );
            split.window.destroy();
        },
    );
}

#[test]
fn hidden_terminal_jobs_block_closing_and_window_disposal_stops_the_shell() {
    gtk_test(
        "ui::window::composition::tabs::tests::panes::terminal::hidden_terminal_jobs_block_closing_and_window_disposal_stops_the_shell",
        || {
            let split = Split::open();
            let content = split.content();
            content.header.terminal.emit_clicked();
            let shell = terminal(content.panes.main());
            command(&shell, "echo $$ > shell-pid; sleep 120");
            let pid_file = split.root.path().join("left/shell-pid");
            wait_until(|| pid_file.exists() && content.has_terminal_jobs());
            content.header.terminal.emit_clicked();
            assert!(!content.panes.main().terminal.widget.is_visible());
            split.tabs.close(0);
            assert_eq!(
                split.tabs.tabs.borrow().len(),
                1,
                "closing must keep a tab with a running job"
            );
            let pid = std::fs::read_to_string(pid_file).expect("shell PID");
            let proc = std::path::PathBuf::from(format!("/proc/{}", pid.trim()));
            split.window.destroy();
            wait_until(|| !proc.exists());
        },
    );
}

#[test]
fn both_terminals_follow_selected_theme_changes_even_while_hidden() {
    gtk_test(
        "ui::window::composition::tabs::tests::panes::terminal::both_terminals_follow_selected_theme_changes_even_while_hidden",
        || {
            let themes = crate::ui::theme::ThemeManager::shared();
            themes.select_theme("azure-glow");
            let split = Split::open();
            split.key(Key::F3, M::empty());
            let content = split.content();
            let (other_window, other_tabs) =
                open_in(&split.window.application().expect("application"));
            let other_content = other_tabs.active_tab().content.clone();
            let terminals: Vec<_> = content
                .panes
                .all()
                .iter()
                .chain(other_content.panes.all().iter())
                .map(|pane| terminal(pane))
                .collect();
            let initial =
                gdk::RGBA::parse(themes.appearance_tokens().background).expect("theme color");
            for shell in &terminals {
                assert_eq!(shell.color_background_for_draw(), initial);
            }
            for theme in ["catppuccin-mocha", "azure-glow"] {
                themes.select_theme(theme);
                let expected =
                    gdk::RGBA::parse(themes.appearance_tokens().background).expect("theme color");
                wait_until(|| {
                    terminals
                        .iter()
                        .all(|shell| shell.color_background_for_draw() == expected)
                });
            }
            other_window.destroy();
            split.window.destroy();
        },
    );
}

#[test]
fn terminal_retries_a_missing_folder_and_accepts_native_path_bytes() {
    gtk_test(
        "ui::window::composition::tabs::tests::panes::terminal::terminal_retries_a_missing_folder_and_accepts_native_path_bytes",
        || {
            use std::os::unix::ffi::OsStringExt;
            let split = Split::open();
            let path = split
                .root
                .path()
                .join(std::ffi::OsString::from_vec(b"native-\xff".to_vec()));
            std::fs::create_dir(&path).expect("native folder");
            load(&split.main(), Location::local(&path));
            std::fs::remove_dir(&path).expect("remove before shell starts");
            let content = split.content();
            content.header.terminal.emit_clicked();
            let pane = content.panes.main();
            let retry =
                button_with_label(pane.terminal.widget.upcast_ref(), "Retry").expect("retry");
            wait_until(|| retry.is_visible());
            assert!(
                retry.is_visible(),
                "a missing starting folder must offer retry"
            );
            assert!(!pane.terminal.is_running());
            std::fs::create_dir(&path).expect("restore folder");
            retry.emit_clicked();
            let shell = terminal(pane);
            command(&shell, "printf native > native-ready");
            await_contents(path.join("native-ready"), b"native");
            content.header.terminal.emit_clicked();
            content.header.terminal.emit_clicked();
            command(&shell, "printf retained > native-reopened");
            await_contents(path.join("native-reopened"), b"retained");
            split.window.destroy();
        },
    );
}
