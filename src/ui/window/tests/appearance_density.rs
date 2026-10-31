// SPDX-License-Identifier: MIT

//! Appearance menu density sliders: each changes only its own area, snap labels select
//! their preset, and no interaction closes the menu.

use std::rc::Rc;
use std::time::{Duration, Instant};

use super::super::*;
use crate::services::{
    LoadHandle, Preview, PreviewContent, PreviewEvent, PreviewProvider, PreviewRequest,
};
use crate::test_support::gtk_test;
use crate::ui::browser_modes::{BrowserDensity, DensityScale, FrameDensity};
use crate::ui::preview::PreviewDrawer;

struct NoPreview;

impl PreviewProvider for NoPreview {
    fn load(&self, request: PreviewRequest, emit: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
        glib::idle_add_local_once(move || {
            emit(PreviewEvent::Ready(Preview {
                request_id: request.id,
                entry: request.entry,
                content_type: "text/plain".into(),
                content: PreviewContent::Text {
                    content: String::new(),
                    truncated: false,
                },
            }))
        });
        LoadHandle::new(|| {})
    }
}

fn settles(condition: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        if Instant::now() >= deadline {
            return false;
        }
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
    true
}

fn descendants(widget: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut widgets = vec![widget.clone()];
    let mut child = widget.first_child();
    while let Some(current) = child {
        widgets.extend(descendants(&current));
        child = current.next_sibling();
    }
    widgets
}

fn density_scales(popover: &gtk::Popover) -> [gtk::Scale; 2] {
    let scales: Vec<gtk::Scale> = descendants(popover.upcast_ref())
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Scale>().ok())
        .filter(|scale| scale.has_css_class("appearance-density-scale"))
        .collect();
    scales.try_into().expect("Frame and Content sliders")
}

fn snap_label(scale: &gtk::Scale, label: &str) -> gtk::Button {
    descendants(&scale.next_sibling().expect("snap labels"))
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
        .find(|button| button.label().as_deref() == Some(label))
        .unwrap_or_else(|| panic!("no {label} snap label"))
}

fn value_text(scale: &gtk::Scale) -> String {
    scale
        .prev_sibling()
        .and_then(|heading| heading.last_child())
        .and_downcast::<gtk::Label>()
        .expect("density value label")
        .text()
        .into()
}

fn open_appearance_menu(preferences: Rc<PreferenceManager>) -> (gtk::Window, gtk::Popover) {
    let view = browser_for_window();
    let preview = PreviewDrawer::new(Rc::new(NoPreview), false);
    let menu = build_appearance_menu(&view, &view.browser(), preferences, &preview);
    let window = gtk::Window::builder().child(&menu).build();
    window.present();
    menu.popup();
    let popover = menu.popover().expect("appearance popover");
    assert!(
        settles(|| popover.is_visible()),
        "the Appearance menu did not open"
    );
    (window, popover)
}

#[test]
fn dragging_a_density_slider_changes_only_its_area_and_keeps_the_menu_open() {
    gtk_test(
        "ui::window::tests::appearance_density::dragging_a_density_slider_changes_only_its_area_and_keeps_the_menu_open",
        || {
            PreferenceManager::seed_saved_preferences_for_test();
            let preferences = PreferenceManager::shared();
            preferences.set_browser_density(BrowserDensity::Compact);
            preferences.set_frame_density(FrameDensity::Compact);
            let (window, popover) = open_appearance_menu(preferences.clone());
            let [frame, content] = density_scales(&popover);

            content.set_value(6.0);
            assert_eq!(preferences.browser_density(), BrowserDensity::Custom(6));
            assert_eq!(preferences.frame_density(), FrameDensity::Compact);
            assert_eq!(value_text(&content), "Custom · 6 px");

            frame.set_value(f64::from(FrameDensity::AIRY_PX));
            assert_eq!(preferences.frame_density(), FrameDensity::Airy);
            assert_eq!(preferences.browser_density(), BrowserDensity::Custom(6));
            assert_eq!(value_text(&frame), "Airy · 4 px");
            assert!(popover.is_visible(), "sliding never closes the menu");

            window.destroy();
        },
    );
}

#[test]
fn snap_labels_select_their_preset_and_follow_changes_from_other_windows() {
    gtk_test(
        "ui::window::tests::appearance_density::snap_labels_select_their_preset_and_follow_changes_from_other_windows",
        || {
            PreferenceManager::seed_saved_preferences_for_test();
            let preferences = PreferenceManager::shared();
            preferences.set_browser_density(BrowserDensity::from_px(11));
            let (window, popover) = open_appearance_menu(preferences.clone());
            let [_, content] = density_scales(&popover);
            let compact = snap_label(&content, "Compact");
            let airy = snap_label(&content, "Airy");
            assert_eq!(content.value(), 11.0);
            assert!(!compact.has_css_class("selected") && !airy.has_css_class("selected"));

            airy.emit_clicked();
            assert_eq!(preferences.browser_density(), BrowserDensity::Airy);
            assert_eq!(content.value(), f64::from(BrowserDensity::AIRY_PX));
            assert!(airy.has_css_class("selected"));
            assert!(popover.is_visible(), "snapping never closes the menu");

            preferences.set_browser_density(BrowserDensity::Compact);
            assert_eq!(content.value(), f64::from(BrowserDensity::COMPACT_PX));
            assert!(compact.has_css_class("selected") && !airy.has_css_class("selected"));
            assert_eq!(value_text(&content), "Compact · 4 px");

            window.destroy();
        },
    );
}
