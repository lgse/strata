use gtk::prelude::*;

use super::super::{
    BrowserDensity, ListColumnLayout, configure_icons_view_density, icons_card_extent,
    icons_loading_skeleton, list_loading_skeleton,
};
use crate::ui::loading_skeleton;

fn children(widget: &impl IsA<gtk::Widget>) -> Vec<gtk::Widget> {
    let mut children = Vec::new();
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        children.push(widget);
    }
    children
}

fn icons(skeleton: &gtk::Box) -> gtk::GridView {
    skeleton
        .first_child()
        .and_downcast::<gtk::ScrolledWindow>()
        .expect("skeleton scroll")
        .child()
        .and_downcast::<gtk::GridView>()
        .expect("skeleton icons")
}

#[test]
fn mode_specific_structure() {
    const CHILD: &str = "STRATA_SKELETON_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let status = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "ui::browser_modes::tests::skeletons::mode_specific_structure",
            ])
            .env(CHILD, "1")
            .status()
            .expect("isolated GTK test should start");
        assert!(status.success());
        return;
    }
    if gtk::init().is_err() {
        return;
    }

    let miller = loading_skeleton::miller();
    assert!(!miller.can_target());
    assert!(!miller.is_focusable());
    let scroll = miller
        .first_child()
        .and_downcast::<gtk::ScrolledWindow>()
        .expect("Miller scroll");
    let rows = scroll
        .child()
        .and_downcast::<gtk::Viewport>()
        .expect("Miller viewport")
        .child()
        .expect("Miller rows");
    assert_eq!(children(&rows).len(), loading_skeleton::ROW_COUNT as usize);
    for row in children(&rows) {
        assert!(row.has_css_class("file-row"));
        assert_eq!(children(&row).len(), 4);
    }

    let columns = ListColumnLayout::new();
    let list = list_loading_skeleton(&columns);
    assert!(!list.can_target());
    let scroll = list
        .first_child()
        .and_downcast::<gtk::ScrolledWindow>()
        .expect("List scroll");
    let table = scroll
        .child()
        .and_downcast::<gtk::Viewport>()
        .expect("List viewport")
        .child()
        .expect("List table");
    let rows = children(&table);
    assert_eq!(rows.len(), loading_skeleton::ROW_COUNT as usize + 1);
    assert!(rows[0].has_css_class("list-headings"));
    for row in &rows {
        assert_eq!(children(row).len(), 5);
    }
    for (index, width) in super::super::LIST_COLUMN_WIDTHS.into_iter().enumerate() {
        for row in &rows {
            assert_eq!(children(row)[index].width_request(), width);
        }
    }

    super::super::set_list_column_width(&columns, 0, 200);
    super::super::set_list_column_width(&columns, 1, 80);
    for row in &rows {
        let cells = children(row);
        assert_eq!(cells[0].width_request(), 200);
        assert!(!cells[0].hexpands());
        assert_eq!(cells[1].width_request(), 80);
    }

    for size in [32, 64, 128, 256] {
        let skeleton = icons_loading_skeleton(size, BrowserDensity::Compact);
        let icons = icons(&skeleton);
        let model = icons.model().expect("placeholder model");
        assert!(model.is::<gtk::NoSelection>());
        assert_eq!(model.n_items(), 60);
        assert_eq!(icons.min_columns(), 1);
        assert_eq!(icons.max_columns(), 20);
        configure_icons_view_density(&icons, BrowserDensity::Airy);
        assert_eq!(icons.max_columns(), 16);
        assert!(icons_card_extent(size).1 > size);
    }
}

fn old_skeleton() -> gtk::Box {
    let skeleton = gtk::Box::new(gtk::Orientation::Vertical, 9);
    skeleton.add_css_class("old-loading-skeleton");
    for width in [168, 124, 192, 148, 176, 112] {
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        bar.add_css_class("old-skeleton-row");
        bar.set_size_request(width, 10);
        bar.set_halign(gtk::Align::Start);
        skeleton.append(&bar);
    }
    skeleton
}

fn gallery(before: bool, density: BrowserDensity, thumbnail_size: i32) -> gtk::Box {
    let gallery = gtk::Box::new(gtk::Orientation::Vertical, 12);
    gallery.set_margin_top(16);
    gallery.set_margin_bottom(16);
    gallery.set_margin_start(16);
    gallery.set_margin_end(16);
    for (mode, root_class) in [
        ("Icons", "mode-icons"),
        ("List", "mode-list"),
        ("Column (Miller)", "miller-columns"),
    ] {
        let label = gtk::Label::new(Some(mode));
        label.set_xalign(0.0);
        gallery.append(&label);
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        root.add_css_class(root_class);
        root.add_css_class(match density {
            BrowserDensity::Compact => "density-compact",
            BrowserDensity::Airy => "density-airy",
        });
        root.set_homogeneous(true);
        root.set_vexpand(true);
        let count = if mode == "Column (Miller)" { 3 } else { 1 };
        for index in 0..count {
            let loading = if before {
                old_skeleton()
            } else {
                match mode {
                    "Icons" => icons_loading_skeleton(thumbnail_size, density),
                    "List" => list_loading_skeleton(&ListColumnLayout::new()),
                    _ => loading_skeleton::miller(),
                }
            };
            let (shell, ..) = super::super::pane_base(
                ["Home", "Documents", "Projects"][index],
                if mode == "Icons" {
                    "icons-pane"
                } else {
                    "list-pane"
                },
                &loading,
                None,
                None,
            );
            root.append(&shell);
        }
        gallery.append(&root);
    }
    gallery
}

#[test]
#[ignore = "renders GTK comparison images; requires a display and STRATA_SKELETON_VISUALS"]
fn capture_comparison() {
    gtk::init().expect("visual capture requires a display");
    let output = std::path::PathBuf::from(
        std::env::var_os("STRATA_SKELETON_VISUALS").expect("visual output directory"),
    );
    std::fs::create_dir_all(&output).expect("create visual output directory");
    let provider = gtk::CssProvider::new();
    provider.load_from_string(&format!(
        "{}\n.old-loading-skeleton {{ padding: 8px; }}\n.old-skeleton-row {{ background: alpha(@theme_border, 0.28); border-radius: 4px; min-height: 10px; }}",
        include_str!("../../../style.css")
    ));
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().expect("GTK display"),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let themes = crate::ui::theme::ThemeManager::shared();
    for (name, before, density, size, theme_id) in [
        ("before", true, BrowserDensity::Compact, 64, "azure-glow"),
        ("after", false, BrowserDensity::Compact, 64, "azure-glow"),
        (
            "airy-light",
            false,
            BrowserDensity::Airy,
            128,
            "atelier-cave-light",
        ),
    ] {
        let theme = themes
            .themes()
            .into_iter()
            .find(|theme| theme.id == theme_id)
            .expect("built-in comparison theme");
        themes.preview(&theme.tokens);
        let gallery = gallery(before, density, size);
        let window = gtk::Window::builder()
            .title("Strata loading skeleton comparison")
            .default_width(960)
            .default_height(1020)
            .child(&gallery)
            .build();
        window.set_size_request(960, 1100);
        window.present();
        let context = gtk::glib::MainContext::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(600);
        while std::time::Instant::now() < deadline {
            while context.pending() {
                context.iteration(false);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let paintable = gtk::WidgetPaintable::new(Some(&window));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(
            &snapshot,
            f64::from(window.width()),
            f64::from(window.height()),
        );
        let node = snapshot.to_node().expect("rendered gallery");
        let texture = window
            .renderer()
            .expect("window renderer")
            .render_texture(&node, None);
        texture
            .save_to_png(output.join(format!("{name}.png")))
            .expect("save comparison image");
        window.close();
    }
}
