// SPDX-License-Identifier: MIT

use super::*;
use crate::ui::browser::BrowserView;
use crate::ui::browser_modes::BrowserMode;
use crate::ui::preferences::PreferenceManager;

/// Call after creating a `BrowserView`: its exhaustive preference fixture seeds
/// `reduce_motion = true`.
fn animations_on() {
    crate::ui::motion::set_reduce_motion(false);
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_enable_animations(true);
    }
}

fn animations_off() {
    crate::ui::motion::set_reduce_motion(true);
}

fn pump_until(condition: impl Fn() -> bool, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn pump_for(duration: Duration) {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
}

struct Columns {
    view: BrowserView,
    window: gtk::Window,
    _root: tempfile::TempDir,
}

impl Columns {
    fn new(depths: usize, window_width: i32) -> Self {
        PreferenceManager::seed_saved_preferences_for_test();
        let root = tempfile::tempdir().expect("columns fixture");
        let mut path = root.path().to_path_buf();
        let mut locations = Vec::new();
        for depth in 0..depths {
            path = path.join(format!("level-{depth}"));
            std::fs::create_dir_all(&path).expect("column directory");
            std::fs::write(path.join("note.txt"), b"note").expect("column entry");
            locations.push(Location::local(&path));
        }
        let view = BrowserView::new(
            Rc::new(crate::adapters::LocalFileSource),
            crate::ui::browser::PeekBehavior::default(),
        );
        view.set_view_mode(BrowserMode::Columns);
        let window = gtk::Window::builder()
            .default_width(window_width)
            .default_height(650)
            .child(&view.widget())
            .build();
        window.present();
        view.browser().navigate(Location::local(root.path()));
        pump_until(|| column_loaded(&view, 0), "root column load");
        for (depth, location) in locations.iter().take(depths.saturating_sub(1)).enumerate() {
            view.browser().descend(depth, location.clone());
            pump_until(|| column_loaded(&view, depth + 1), "descended column load");
        }
        pump_for(Duration::from_millis(40));
        Self {
            view,
            window,
            _root: root,
        }
    }

    fn shell(&self, depth: usize) -> gtk::Box {
        self.view
            .state
            .columns
            .borrow()
            .get(depth)
            .map(|column| column.shell.clone())
            .expect("column shell")
    }

    fn scroller(&self) -> gtk::ScrolledWindow {
        self.view.state.scroller.clone()
    }

    fn adjustment(&self) -> gtk::Adjustment {
        self.view.state.scroller.hadjustment()
    }
}

impl Drop for Columns {
    fn drop(&mut self) {
        PreferenceManager::shared().release_bindings_within(&self.view.widget());
        self.view.browser().clear_observer();
        self.window.destroy();
    }
}

fn column_loaded(view: &BrowserView, depth: usize) -> bool {
    view.browser()
        .column_snapshot(depth)
        .is_some_and(|column| !column.loading)
}

fn resize_gesture(scroller: &gtk::ScrolledWindow) -> gtk::GestureDrag {
    let controllers = scroller.observe_controllers();
    (0..controllers.n_items())
        .filter_map(|index| {
            controllers
                .item(index)
                .and_then(|controller| controller.downcast::<gtk::GestureDrag>().ok())
        })
        .find(|gesture| gesture.name().as_deref() == Some("column-resize"))
        .expect("column-resize gesture")
}

fn shell_edge(scroller: &gtk::ScrolledWindow, shell: &gtk::Box) -> (f64, f64) {
    pump_until(
        || {
            shell
                .compute_bounds(scroller)
                .is_some_and(|bounds| bounds.width() > 0.0)
        },
        "column allocation",
    );
    let bounds = shell.compute_bounds(scroller).expect("column bounds");
    (
        f64::from(bounds.x() + bounds.width()) - 0.5,
        f64::from(bounds.y()) + 4.0,
    )
}

fn column_pane(shell: &gtk::Box) -> gtk::Widget {
    shell
        .first_child()
        .and_downcast::<gtk::Overlay>()
        .and_then(|overlay| overlay.child())
        .expect("column pane")
}

#[test]
fn reveal_column_does_not_move_an_already_visible_active_column() {
    crate::test_support::gtk_test(
        "ui::browser::columns::tests::reveal_column_does_not_move_an_already_visible_active_column",
        || {
            let fixture = Columns::new(6, 700);
            animations_off();
            fixture.view.browser().set_active_column(5);
            let adjustment = fixture.adjustment();
            pump_until(|| adjustment.page_size() > 0.0, "viewport sizing");
            adjustment.set_value(adjustment.upper() - adjustment.page_size());
            pump_for(Duration::from_millis(40));
            let before = adjustment.value();
            fixture.view.state.reveal_column(fixture.shell(5));
            pump_for(Duration::from_millis(200));
            assert_eq!(
                adjustment.value(),
                before,
                "an already-visible active column must not be dragged"
            );
        },
    );
}

#[test]
fn a_resize_during_a_column_entry_does_not_strand_its_animation() {
    crate::test_support::gtk_test(
        "ui::browser::columns::tests::a_resize_during_a_column_entry_does_not_strand_its_animation",
        || {
            PreferenceManager::seed_saved_preferences_for_test();
            let root = tempfile::tempdir().expect("entry fixture");
            let view = BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                crate::ui::browser::PeekBehavior::default(),
            );
            view.set_view_mode(BrowserMode::Columns);
            animations_on();
            let window = gtk::Window::builder()
                .default_width(900)
                .default_height(650)
                .child(&view.widget())
                .build();
            window.present();
            view.browser().navigate(Location::local(root.path()));
            let shell = view
                .state
                .columns
                .borrow()
                .first()
                .map(|column| column.shell.clone())
                .expect("column shell");
            let pane = column_pane(&shell);
            let scroller = view.state.scroller.clone();
            let edge = shell_edge(&scroller, &shell);
            assert!(pane.has_css_class("column-entering"));
            let gesture = resize_gesture(&scroller);
            gesture.emit_by_name::<()>("drag-begin", &[&edge.0, &edge.1]);
            gesture.emit_by_name::<()>("drag-end", &[&0.0f64, &0.0f64]);
            pump_until(
                || !pane.has_css_class("column-entering"),
                "the entry animation to end despite the resize",
            );
            window.destroy();
            view.browser().clear_observer();
        },
    );
}

#[test]
fn a_closing_column_leaves_after_its_animation_and_takes_no_input() {
    crate::test_support::gtk_test(
        "ui::browser::columns::tests::a_closing_column_leaves_after_its_animation_and_takes_no_input",
        || {
            let fixture = Columns::new(3, 900);
            animations_on();
            let exiting = fixture.shell(1);
            fixture.view.browser().close_column(1);
            assert!(
                exiting.parent().is_some(),
                "the column stays while its exit animation plays"
            );
            let scroller = fixture.scroller();
            let bounds = exiting.compute_bounds(&scroller).expect("exiting bounds");
            let left = bounds.x().max(0.0);
            let right = (bounds.x() + bounds.width()).min(scroller.width() as f32);
            assert!(
                right - left > 2.0,
                "part of the closing column is on screen"
            );
            let picked = scroller.pick(
                f64::from((left + right) / 2.0),
                f64::from(bounds.y() + bounds.height() / 2.0),
                gtk::PickFlags::DEFAULT,
            );
            assert!(
                picked.is_none_or(|picked| !picked.is_ancestor(&exiting) && picked != exiting),
                "a closing column takes no clicks or drops meant for the column now at its depth"
            );
            pump_until(
                || exiting.parent().is_none(),
                "the column to leave once its animation ends",
            );
        },
    );
}

#[test]
fn switching_to_a_sibling_closes_the_old_child_without_an_exit_animation() {
    crate::test_support::gtk_test(
        "ui::browser::columns::tests::switching_to_a_sibling_closes_the_old_child_without_an_exit_animation",
        || {
            let fixture = Columns::new(3, 900);
            animations_on();
            let old_child = fixture.shell(1);
            let old_grandchild = fixture.shell(2);
            let sibling = fixture._root.path().join("sibling-0");
            std::fs::create_dir_all(&sibling).expect("sibling directory");
            fixture.view.browser().descend(0, Location::local(&sibling));
            pump_until(|| column_loaded(&fixture.view, 1), "sibling column load");
            assert!(
                old_child.parent().is_none(),
                "the replaced child goes at once so its replacement takes its place"
            );
            assert!(
                old_grandchild.parent().is_some(),
                "deeper columns of the replaced branch shrink away so the strip slides"
            );
            pump_until(
                || old_grandchild.parent().is_none(),
                "the deeper column's exit animation to finish",
            );
        },
    );
}

#[test]
fn close_column_skips_exit_animation_when_animations_disabled() {
    crate::test_support::gtk_test(
        "ui::browser::columns::tests::close_column_skips_exit_animation_when_animations_disabled",
        || {
            let fixture = Columns::new(3, 900);
            animations_off();
            let exiting = fixture.shell(1);
            fixture.view.browser().close_column(1);
            assert!(
                exiting.parent().is_none(),
                "with animations disabled the column is removed immediately"
            );
        },
    );
}

#[test]
fn a_finished_edge_drag_resizes_every_open_column() {
    crate::test_support::gtk_test(
        "ui::browser::columns::tests::a_finished_edge_drag_resizes_every_open_column",
        || {
            let fixture = Columns::new(3, 1400);
            animations_off();
            let dragged = fixture.shell(1);
            let others = [fixture.shell(0), fixture.shell(2)];
            let before: Vec<_> = others.iter().map(gtk::Box::width_request).collect();
            let gesture = resize_gesture(&fixture.scroller());
            let edge = shell_edge(&fixture.scroller(), &dragged);
            gesture.emit_by_name::<()>("drag-begin", &[&edge.0, &edge.1]);
            gesture.emit_by_name::<()>("drag-update", &[&80.0f64, &0.0f64]);
            assert_eq!(
                others
                    .iter()
                    .map(gtk::Box::width_request)
                    .collect::<Vec<_>>(),
                before,
                "only the dragged column follows the pointer"
            );
            gesture.emit_by_name::<()>("drag-end", &[&80.0f64, &0.0f64]);
            for column in &others {
                assert_eq!(
                    column.width_request(),
                    dragged.width_request(),
                    "the other open columns take the dragged width once the drag ends"
                );
            }
        },
    );
}

#[test]
fn double_clicking_an_edge_autofits_the_column_and_saves_its_width() {
    crate::test_support::gtk_test(
        "ui::browser::columns::tests::double_clicking_an_edge_autofits_the_column_and_saves_its_width",
        || {
            for animated in [true, false] {
                let fixture = Columns::new(1, 900);
                if animated {
                    animations_on();
                } else {
                    animations_off();
                }
                let shell = fixture.shell(0);
                let before = shell.width_request();
                let gesture = resize_gesture(&fixture.scroller());
                let edge = shell_edge(&fixture.scroller(), &shell);
                gesture.emit_by_name::<()>("drag-begin", &[&edge.0, &edge.1]);
                gesture.emit_by_name::<()>("drag-begin", &[&edge.0, &edge.1]);
                if animated {
                    pump_for(COLUMN_TRANSITION + Duration::from_millis(60));
                }
                let fitted = shell.width_request();
                assert_ne!(fitted, before, "the column fits its content");
                let scale = PreferenceManager::shared().interface_scale();
                assert_eq!(
                    PreferenceManager::shared().browser_column_width(),
                    Some(((f64::from(fitted) / scale).round() as i32).max(COLUMN_WIDTH)),
                    "the saved width is the one the column ends at"
                );
            }
        },
    );
}

#[test]
fn reveal_shows_a_fitting_column_whole_and_leaves_a_filling_one_alone() {
    let cases = [
        ((3210.0, 3510.0), 3212.0, 300.0, 3210.0),
        ((1000.0, 1702.0), 1001.0, 700.0, 1001.0),
        ((1000.0, 1702.0), 900.0, 700.0, 1000.0),
    ];
    for ((left, right), current, page_size, expected) in cases {
        let span = ColumnSpan {
            left,
            right,
            trailing: 0.0,
        };
        assert_eq!(
            span.reveal_target(current, page_size, 0.0, 4000.0),
            expected,
            "{left}..{right} seen from {current} in {page_size}"
        );
    }
}
