// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn install_guidance_uses_caller_package_mappings_or_falls_back() {
    let tools = [
        MissingTool {
            name: "mkfs.ntfs or mkntfs",
            packages: &[
                (PackageManager::Pacman, "ntfsprogs"),
                (PackageManager::Dnf, "ntfsprogs"),
            ],
        },
        MissingTool {
            name: "ntfslabel",
            packages: &[
                (PackageManager::Pacman, "ntfsprogs"),
                (PackageManager::Dnf, "ntfsprogs"),
            ],
        },
    ];
    assert_eq!(
        install_command(Some(PackageManager::Pacman), &tools).as_deref(),
        Some("sudo pacman -S --needed ntfsprogs")
    );
    assert_eq!(
        install_command(Some(PackageManager::Dnf), &tools).as_deref(),
        Some("sudo dnf install ntfsprogs")
    );
    assert!(install_command(Some(PackageManager::Apt), &tools).is_none());
    assert!(install_command(None, &tools).is_none());
}

fn descendants(root: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut widgets = Vec::new();
    let mut child = root.first_child();
    while let Some(widget) = child {
        widgets.extend(descendants(&widget));
        child = widget.next_sibling();
        widgets.push(widget);
    }
    widgets
}

#[test]
fn missing_tools_dialog_shows_caller_content_and_closes() {
    crate::test_support::gtk_test(
        "ui::missing_tools::tests::missing_tools_dialog_shows_caller_content_and_closes",
        || {
            let parent = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&parent));
            let window = gtk::Window::builder().child(&overlay).build();
            window.present();
            show_missing_tools(
                &parent,
                "Formatting requires a filesystem tool.",
                &[MissingTool {
                    name: "mkfs.fat",
                    packages: &[
                        (PackageManager::Pacman, "dosfstools"),
                        (PackageManager::Apt, "dosfstools"),
                    ],
                }],
            );
            let widgets = descendants(overlay.upcast_ref());
            for text in [
                "Missing tools",
                "Formatting requires a filesystem tool.",
                "Missing: mkfs.fat",
            ] {
                assert!(
                    widgets.iter().any(|widget| {
                        widget
                            .clone()
                            .downcast::<gtk::Label>()
                            .is_ok_and(|label| label.text() == text)
                    }),
                    "{text}"
                );
            }
            let close = widgets
                .into_iter()
                .find_map(|widget| {
                    widget
                        .downcast::<gtk::Button>()
                        .ok()
                        .filter(|button| button.label().as_deref() == Some("Close"))
                })
                .expect("close missing-tools dialog");
            let layer = close
                .ancestor(gtk::Box::static_type())
                .expect("missing-tools dialog action area");
            close.emit_clicked();
            let context = glib::MainContext::default();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while layer.root().is_some() && std::time::Instant::now() < deadline {
                context.iteration(true);
            }
            assert!(layer.root().is_none());
            window.close();
        },
    );
}
