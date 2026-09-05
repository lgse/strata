// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use std::time::{Duration, Instant};

fn pixels(window: &gtk::Window, widget: &gtk::Widget) -> Vec<u8> {
    let snapshot = gtk::Snapshot::new();
    gtk::WidgetPaintable::new(Some(widget)).snapshot(
        &snapshot,
        widget.width() as f64,
        widget.height() as f64,
    );
    let texture = window.renderer().expect("renderer").render_texture(
        snapshot.to_node().expect("render node"),
        Some(&gtk::graphene::Rect::new(
            0.0,
            0.0,
            widget.width() as f32,
            widget.height() as f32,
        )),
    );
    let mut pixels = vec![0; widget.width() as usize * widget.height() as usize * 4];
    texture.download(&mut pixels, widget.width() as usize * 4);
    pixels
}

#[test]
#[ignore = "requires X11, xdotool, and isolated XDG directories; run this test alone"]
fn options_share_a_compact_row_and_wrap_in_narrow_windows() {
    gtk::init().expect("GTK display");
    crate::ui::prepare_portal_ui();
    let context = glib::MainContext::default();
    let _owner = context.acquire().expect("exclusive GTK context");
    for (width, wraps) in [(900, false), (320, true)] {
        let options = chooser_options();
        options.set_valign(gtk::Align::Start);
        let filter = ChooserDropdown::new(&["Text files", "Images"], 0);
        append_option(
            &options,
            &labeled_row("Filter", Some(filter.button.upcast_ref())),
        );
        let choices = build_choices(
            &[
                Choice::new("encoding", "Encoding", "utf8")
                    .insert("utf8", "UTF-8")
                    .insert("latin1", "Latin-1"),
                Choice::boolean("compress", "Compress files", false),
            ],
            &options,
        );
        let window = gtk::Window::builder()
            .title("Strata keyboard regression")
            .default_width(width)
            .default_height(160)
            .child(&options)
            .build();
        window.present();
        super::keyboard::focus_window();
        let first = options.first_child().expect("filter group");
        let second = first.next_sibling().expect("encoding group");
        let third = second.next_sibling().expect("compression group");
        let deadline = Instant::now() + Duration::from_secs(5);
        while first.width() == 0 || third.height() == 0 {
            while context.pending() {
                context.iteration(false);
            }
            assert!(Instant::now() < deadline, "options are allocated");
            std::thread::sleep(Duration::from_millis(5));
        }
        let bounds = [&first, &second, &third]
            .map(|child| child.compute_bounds(&options).expect("option bounds"));
        if wraps {
            assert!(
                bounds[2].y() > bounds[0].y() + bounds[0].height(),
                "options wrap instead of clipping"
            );
        } else {
            let centers = bounds.map(|bounds| bounds.y() + bounds.height() / 2.0);
            assert!(
                centers
                    .iter()
                    .all(|center| (*center - centers[0]).abs() <= 1.0),
                "all options share one row"
            );
            assert!(
                options.height() < 42,
                "the options row is shorter than a standard form field"
            );
        }
        assert_eq!(choices[0].value(), ("encoding".into(), "utf8".into()));
        assert_eq!(choices[1].value(), ("compress".into(), "false".into()));
        for child in [&first, &second, &third] {
            assert!(
                !child.is_focusable(),
                "Tab targets controls, not layout wrappers"
            );
        }
        let ChoiceControl::Select {
            dropdown: encoding, ..
        } = &choices[0]
        else {
            panic!("encoding select")
        };
        let ChoiceControl::Boolean { check, .. } = &choices[1] else {
            panic!("compression check")
        };
        window.set_focus_visible(true);
        for (wrapper, dropdown) in [(&first, &filter), (&second, encoding)] {
            check.grab_focus();
            super::keyboard::settle();
            let before = pixels(&window, options.upcast_ref());
            gtk::prelude::GtkWindowExt::set_focus(&window, None::<&gtk::Widget>);
            for _ in 0..8 {
                if gtk::prelude::RootExt::focus(&window)
                    .is_some_and(|focus| focus.is_ancestor(&dropdown.button))
                {
                    break;
                }
                super::keyboard::key("Tab");
            }
            assert!(
                gtk::prelude::RootExt::focus(&window)
                    .is_some_and(|focus| focus.is_ancestor(&dropdown.button))
            );
            super::keyboard::settle();
            let after = pixels(&window, options.upcast_ref());
            let bounds = dropdown
                .button
                .compute_bounds(wrapper)
                .expect("select bounds");
            let label_width = (bounds.x() / 2.0) as usize;
            assert!(label_width > 0);
            let group = wrapper.compute_bounds(&options).expect("group bounds");
            let stride = options.width() as usize * 4;
            for y in group.y() as usize..(group.y() + group.height()) as usize {
                let start = y * stride + group.x() as usize * 4;
                assert_eq!(
                    &before[start..start + label_width * 4],
                    &after[start..start + label_width * 4],
                    "focus does not decorate the layout wrapper or label",
                );
            }
            let select = dropdown
                .button
                .compute_bounds(&options)
                .expect("select bounds");
            assert!(
                (select.y() as usize..(select.y() + select.height()) as usize).any(|y| {
                    let start = y * stride + select.x() as usize * 4;
                    let end = start + select.width() as usize * 4;
                    before[start..end] != after[start..end]
                }),
                "the select retains its own visible focus indicator"
            );
        }
        window.destroy();
    }
}
