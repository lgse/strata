// SPDX-License-Identifier: MIT

use rune::{Env, Runtime, el};
use strata_kit::look::Look;

use super::*;

const R: DrawerRules = DrawerRules::STRATA;

fn columns(navigated: f32, keep: f32) -> Arrangement {
    Arrangement::Columns(ColumnsFit::new(&R, navigated, keep))
}

#[test]
fn in_columns_the_automatic_width_fills_the_space_right_of_the_focused_column() {
    // 1 point of divider line, then 600 of columns: the rest is the preview's.
    let s = solve(&R, &columns(600.0, 300.0), 1201.0, None);
    assert_eq!(s.width, 600.0);
    assert!(s.fits);
}

#[test]
fn the_columns_minimum_is_one_standard_column_and_yields_to_the_focused_column() {
    // Only 200 free beside the navigated columns: the preview takes its 300 minimum.
    let s = solve(&R, &columns(800.0, 300.0), 1001.0, None);
    assert_eq!(s.width, 300.0);
    // The focused column needs 750 to stay whole: the minimum yields down to what is left.
    let s = solve(&R, &columns(800.0, 750.0), 1001.0, None);
    assert_eq!(s.width, 250.0);
}

#[test]
fn a_dragged_width_in_columns_is_a_session_minimum_the_preview_still_fills_past() {
    let fit = columns(400.0, 300.0);
    // Wider free space than the manual width: the preview fills it.
    assert_eq!(solve(&R, &fit, 1201.0, Some(500.0)).width, 800.0);
    // Less free space: the columns yield down to the manual width.
    assert_eq!(solve(&R, &fit, 801.0, Some(500.0)).width, 500.0);
}

#[test]
fn in_icons_and_list_the_automatic_width_is_ninety_percent_of_half_within_its_bounds() {
    for a in [Arrangement::Icons, Arrangement::List] {
        assert_eq!(solve(&R, &a, 1001.0, None).width, 450.0);
        // The 240 floor, while the files keep one standard column.
        assert_eq!(solve(&R, &a, 541.0, None).width, 240.0);
        // The 3000 ceiling.
        assert_eq!(solve(&R, &a, 10_001.0, None).width, 3000.0);
    }
}

#[test]
fn in_icons_and_list_a_dragged_width_is_the_width_and_a_window_resize_keeps_it() {
    for available in [900.0, 1200.0, 1600.0] {
        assert_eq!(
            solve(&R, &Arrangement::List, available, Some(420.0)).width,
            420.0
        );
    }
}

#[test]
fn the_content_hides_when_less_than_240_would_remain_beside_the_focused_column() {
    // Columns: 1 + 300 standard + 239 left over.
    let fit = Arrangement::Columns(ColumnsFit::new(&R, 300.0, 300.0));
    assert!(!solve(&R, &fit, 540.0, None).fits);
    assert!(solve(&R, &fit, 541.0, None).fits);
    // Icons and List keep one standard column for the files.
    assert!(!solve(&R, &Arrangement::Icons, 540.0, None).fits);
    assert!(solve(&R, &Arrangement::Icons, 541.0, None).fits);
    // The narrow test counts the focused column at most one standard column.
    let wide = Arrangement::Columns(ColumnsFit::new(&R, 500.0, 500.0).keep_focused_standard(300.0));
    assert!(solve(&R, &wide, 600.0, None).fits);
}

#[test]
fn the_reserved_slot_lends_trailing_columns_its_width_and_shrinks_to_zero() {
    let fit = ColumnsFit::new(&R, 400.0, 300.0).trailing(250.0);
    let s = solve(&R, &Arrangement::Columns(fit), 1001.0, None);
    // The 600 point preview and the divider's line, less the 250 lent.
    assert_eq!(s.empty_slot, 351.0);
    // A narrow window: the slot shrinks to nothing instead of going.
    let s = solve(
        &R,
        &Arrangement::Columns(ColumnsFit::new(&R, 400.0, 400.0)),
        401.0,
        None,
    );
    assert_eq!(s.empty_slot, 0.0);
}

#[test]
fn a_manual_width_is_at_least_one_standard_column() {
    let mut st = DrawerState::new();
    st.set_manual_width(&R, 120.0);
    assert_eq!(st.manual_width(), Some(300.0));
    assert_eq!(st.demand(&R), 300.0);
    assert_eq!(DrawerState::new().demand(&R), 240.0);
}

/// Runs `f` on a drawer in a handler, as a host does.
fn in_handler(
    st: DrawerState,
    f: impl FnOnce(&mut DrawerState, &mut rune::Tx) + 'static,
) -> DrawerState {
    let mut rt = Runtime::new(
        st,
        |_: &DrawerState, _: &mut rune::Cx<DrawerState>| el(),
        (10.0, 10.0),
        Env::default(),
    );
    rt.frame(0.0);
    rt.update(f);
    rt.state.clone()
}

#[test]
fn closing_keeps_the_manual_width_and_ends_find() {
    let mut st = DrawerState::new();
    st.set_manual_width(&R, 480.0);
    let st = in_handler(st, |st, tx| {
        let look = Look::fallback(false);
        st.open(&look, tx);
        st.open_find();
        st.find.query.replace_text("needle");
        st.close(&look, tx);
    });
    assert!(!st.is_open());
    assert_eq!(
        st.manual_width(),
        Some(480.0),
        "reopening restores the width"
    );
    assert!(!st.is_finding());
    assert_eq!(st.find_query(), "", "closing the drawer clears the query");
}

#[test]
fn reopening_find_keeps_the_query_selected() {
    let mut st = DrawerState::new();
    st.open_find();
    st.find.query.replace_text("needle");
    st.find.open = false;
    st.open_find();
    assert!(st.find.query.focused);
    assert_eq!(st.find.query.selected(), "needle");
}

#[test]
fn a_closed_drawer_is_never_suspended_and_reopening_reconciles_with_the_host() {
    let mut st = DrawerState::new();
    st.open_at_once();
    st.suspended = true;
    assert!(st.is_suspended());
    let st = in_handler(st, |st, tx| st.close(&Look::fallback(false), tx));
    assert!(!st.is_suspended(), "nothing to pause while closed");
    // The host last heard Suspend(true); the next open's fit marker owes it Suspend(false).
    assert!(st.suspended);
}

#[test]
fn the_right_pane_tells_c2_what_the_empty_slot_keeps() {
    let fit = ColumnsFit::new(&R, 400.0, 300.0).trailing(250.0);
    let pane = right_pane(&DrawerState::new(), &R, &fit, 1001.0);
    assert_eq!(
        (pane.width, pane.empty_slot, pane.preview_shown),
        (600.0, 351.0, false)
    );
}

#[test]
fn in_columns_a_wider_standard_column_raises_the_manual_floor() {
    let fit = Arrangement::Columns(ColumnsFit::new(&R, 400.0, 300.0).standard_column(340.0));
    let s = solve(&R, &fit, 701.0, Some(300.0));
    assert_eq!((s.width, s.min), (340.0, 340.0));
}

#[test]
fn return_on_a_drawer_button_fires_its_intent() {
    use rune::{Key, KeyEvent};

    struct Host {
        drawer: DrawerState,
        closed: bool,
    }

    fn view(host: &Host, cx: &mut rune::Cx<Host>) -> El {
        let look = Look::fallback(false);
        PreviewDrawer::new(&host.drawer, |h: &mut Host| &mut h.drawer)
            .placeholder("Ready")
            .on_intent(|h, intent, _| {
                if intent == DrawerIntent::Closed {
                    h.closed = true;
                }
            })
            .build(el().key("files").focusable(true), &look, cx)
    }

    let mut drawer = DrawerState::new();
    drawer.open_at_once();
    let mut rt = Runtime::new(
        Host {
            drawer,
            closed: false,
        },
        view,
        (800.0, 400.0),
        Env::default(),
    );
    rt.frame(0.0);
    rt.settle_all();
    rt.frame(0.0);
    let open = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("preview-open"))
        .expect("Open button stays visible on placeholder");
    assert!(!open.focusable, "Open button is disabled on placeholder");
    rt.update(|_, tx| tx.focus("preview-close", true));
    rt.frame(0.0);
    rt.key(KeyEvent {
        key: Key::Enter,
        mods: rune::Mods::default(),
    });
    // The sent Space lands on the next frame.
    rt.frame(1.0 / 60.0);
    rt.frame(1.0 / 60.0);
    assert!(
        rt.state.closed,
        "Return on Close fires DrawerIntent::Closed"
    );
}

struct FindHost {
    drawer: DrawerState,
    doc: crate::document_view::DocumentState,
    look: Look,
}

fn find_host_view(host: &FindHost, cx: &mut rune::Cx<FindHost>) -> El {
    let body = crate::document_view::DocumentView::new(|h: &FindHost| &h.doc, |h| &mut h.doc)
        .build(host, &host.look, cx);
    PreviewDrawer::new(&host.drawer, |h: &mut FindHost| &mut h.drawer)
        .hosted(&host.doc, |h: &mut FindHost| &mut h.doc)
        .content(body)
        .build(el().key("files").focusable(true), &host.look, cx)
}

fn find_host_poll(host: &mut FindHost, clock: f64, tx: &mut rune::Tx) -> bool {
    host.doc.tick(clock) | host.doc.flush(tx)
}

fn assert_find_close_keeps_selection(by_button: bool) {
    use std::sync::Arc;

    use rune::{Key, KeyEvent, Mods, clipboard};

    use crate::document_view::{Document, DocumentState, Unit, UnitKind};

    let mut drawer = DrawerState::new();
    drawer.open_at_once();
    let doc = DocumentState::new(Arc::new(Document::new(vec![
        Unit::new(UnitKind::Paragraph, "First needle here."),
        Unit::new(UnitKind::Paragraph, "Second NEEDLE here."),
    ])))
    .key("preview-document");
    let state = FindHost {
        drawer,
        doc,
        look: Look::fallback(false),
    };
    let mut rt = Runtime::new(state, find_host_view, (900.0, 500.0), Env::default());
    rt.poll = Some(find_host_poll);
    let clip = clipboard::memory();
    rt.use_clipboard(&clip);
    rt.frame(0.0);
    rt.settle_all();

    rt.update(|h, _| {
        h.drawer.start_find("needle");
        h.doc.find("needle");
        h.doc.find_step(true);
    });
    rt.frame(1.0 / 60.0);
    rt.settle_all();
    assert_eq!(rt.selected_text().as_deref(), Some("NEEDLE"));

    if by_button {
        let rect = rt
            .elements()
            .into_iter()
            .find(|e| e.path.ends_with("/preview-find-close"))
            .expect("close button")
            .rect;
        let (x, y) = (rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0);
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
    } else {
        rt.key(KeyEvent {
            key: Key::Escape,
            mods: Mods::default(),
        });
    }
    rt.frame(1.0 / 60.0);
    rt.settle_all();
    assert_eq!(rt.selected_text().as_deref(), Some("NEEDLE"));

    let copy = KeyEvent {
        key: Key::Command('c'),
        mods: Mods {
            cmd: true,
            ctrl: true,
            ..Mods::default()
        },
    };
    rt.key(copy);
    rt.frame(1.0 / 60.0);
    assert_eq!(clip.text().as_deref().map(String::as_str), Some("NEEDLE"));
}

#[test]
fn closing_find_keeps_the_matched_selection_for_copy() {
    assert_find_close_keeps_selection(false);
    assert_find_close_keeps_selection(true);
}

#[test]
fn focus_content_when_shown_lands_when_the_viewer_mounts() {
    struct DeferredHost {
        drawer: DrawerState,
        loaded: bool,
        look: Look,
    }

    fn deferred_view(host: &DeferredHost, cx: &mut rune::Cx<DeferredHost>) -> El {
        let body = if host.loaded {
            el().key("preview-viewer").focusable(true).w_full().h_full()
        } else {
            el().key("preview-awaiting").w_full().h_full()
        };
        PreviewDrawer::new(&host.drawer, |h: &mut DeferredHost| &mut h.drawer)
            .content(body)
            .build(
                el().key("files").focusable(true).w_full().h_full(),
                &host.look,
                cx,
            )
    }

    let mut drawer = DrawerState::new();
    drawer.open_at_once();
    let mut rt = Runtime::new(
        DeferredHost {
            drawer,
            loaded: false,
            look: Look::fallback(false),
        },
        deferred_view,
        (900.0, 500.0),
        Env::default(),
    );
    rt.frame(0.0);
    rt.update(|h, tx| {
        tx.focus("files", true);
        let _ = h;
    });
    rt.frame(1.0 / 60.0);

    // Ask to focus the preview body while only the non-focusable placeholder is mounted.
    rt.update(|h, tx| h.drawer.focus_content_when_shown(tx));
    rt.frame(1.0 / 60.0);
    let focused = |rt: &mut Runtime<DeferredHost>| {
        rt.elements()
            .into_iter()
            .find(|e| e.focused && !e.exiting)
            .and_then(|e| e.key)
    };
    assert_eq!(focused(&mut rt).as_deref(), Some("files"));

    // Once the focusable viewer mounts, the owed focus lands on it automatically.
    rt.update(|h, _| h.loaded = true);
    rt.frame(1.0 / 60.0);
    assert_eq!(focused(&mut rt).as_deref(), Some("preview-viewer"));
}

#[test]
fn overlay_above_divider_takes_pointer_hits_while_divider_stays_draggable() {
    struct OverlayHost {
        drawer: DrawerState,
        look: Look,
        overlay_open: bool,
        overlay_hits: u32,
    }

    fn overlay_view(host: &OverlayHost, cx: &mut rune::Cx<OverlayHost>) -> El {
        let base = PreviewDrawer::new(&host.drawer, |h: &mut OverlayHost| &mut h.drawer)
            .placeholder("Ready")
            .build(
                el().key("files").focusable(true).w_full().h_full(),
                &host.look,
                cx,
            );
        let overlay = host.overlay_open.then(|| {
            el().key("menu-overlay")
                .role("button")
                .absolute(0.0, 0.0)
                .w(900.0)
                .h(500.0)
                .fill(host.look.kit.c.surface_raised)
                .on_press(|h: &mut OverlayHost| h.overlay_hits += 1)
        });
        rune_kit::root(base, overlay)
    }

    let mut drawer = DrawerState::new();
    drawer.open_at_once();
    let mut rt = Runtime::new(
        OverlayHost {
            drawer,
            look: Look::fallback(false),
            overlay_open: true,
            overlay_hits: 0,
        },
        overlay_view,
        (900.0, 500.0),
        Env::default(),
    );
    rt.frame(0.0);
    rt.settle_all();

    let grip = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("preview-grip"))
        .expect("preview divider grip")
        .rect;
    let (x, y) = (grip[0] + grip[2] / 2.0, grip[1] + grip[3] / 2.0);

    // An overlay in rune_kit::root's layer must paint and hit-test above the divider.
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(1.0 / 60.0);
    assert_eq!(rt.state.overlay_hits, 1);
    assert_eq!(rt.state.drawer.manual_width(), None);

    // Once the overlay closes, dragging the divider at the same coordinates resizes the drawer.
    rt.update(|h, _| h.overlay_open = false);
    rt.frame(1.0 / 60.0);
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_move(x - 40.0, y);
    rt.pointer_up(x - 40.0, y);
    rt.frame(1.0 / 60.0);
    assert!(
        rt.state.drawer.manual_width().is_some(),
        "divider must still resize the drawer when no overlay covers it"
    );
}

#[test]
fn sliding_open_closing_and_reversing_keep_single_bg_base_fill_across_slot() {
    use rune::Color;
    use strata_kit::{shape::LookParams, transparency::TransparencyParams};

    struct SlideHost {
        drawer: DrawerState,
        look: Look,
    }

    fn slide_view(host: &SlideHost, cx: &mut rune::Cx<SlideHost>) -> El {
        PreviewDrawer::new(&host.drawer, |h: &mut SlideHost| &mut h.drawer)
            .placeholder("Ready")
            .build(
                el().key("files")
                    .w_full()
                    .h_full()
                    .fill(host.look.kit.c.bg_base),
                &host.look,
                cx,
            )
    }

    let look = Look::fallback(false).with_params(&LookParams {
        transparency: TransparencyParams {
            enabled: true,
            background_percent: 30,
            raised_percent: 40,
            ..TransparencyParams::default()
        },
        ..LookParams::default()
    });
    assert!(look.kit.c.bg_base.a < 0.99);

    let mut rt = Runtime::new(
        SlideHost {
            drawer: DrawerState::new(),
            look: look.clone(),
        },
        slide_view,
        (1000.0, 500.0),
        Env::default(),
    );
    rt.frame(0.0);
    rt.settle_all();

    let check = |rt: &Runtime<SlideHost>, stage: &str| {
        let bg = rt.state.look.kit.c.bg_base;
        let els = rt.elements();
        let files = els
            .iter()
            .find(|e| !e.exiting && e.key.as_deref() == Some("preview-files"))
            .expect("preview-files");
        let slot = els
            .iter()
            .find(|e| !e.exiting && e.key.as_deref() == Some("preview-slot"))
            .expect("preview-slot");
        assert_eq!(slot.fill, bg, "{stage}: preview-slot owns bg_base");
        for drawer in els
            .iter()
            .filter(|e| e.key.as_deref() == Some("preview-drawer"))
        {
            assert_eq!(
                drawer.fill,
                Color::TRANSPARENT,
                "{stage}: preview-drawer must not double-paint bg_base"
            );
        }
        let seam = (files.visible[0] + files.visible[2] - slot.visible[0]).abs();
        assert!(
            seam <= 0.5,
            "{stage}: files right ({}) meets slot left ({})",
            files.visible[0] + files.visible[2],
            slot.visible[0]
        );
    };

    // Open slide.
    rt.update(|h, tx| h.drawer.open(&h.look, tx));
    for dt in [0.0, 1.0 / 60.0, 0.05, 0.08, 0.08, 0.1] {
        rt.frame(dt);
        check(&rt, "open");
    }

    // Close slide, then reverse back open mid-slide, then close fully.
    rt.update(|h, tx| h.drawer.close(&h.look, tx));
    for dt in [0.0, 1.0 / 60.0, 0.06] {
        rt.frame(dt);
        check(&rt, "close mid");
    }
    rt.update(|h, tx| h.drawer.open(&h.look, tx));
    for dt in [0.0, 1.0 / 60.0, 0.08, 0.12, 0.12] {
        rt.frame(dt);
        check(&rt, "reopen mid-close");
    }
    rt.update(|h, tx| h.drawer.close(&h.look, tx));
    for dt in [0.0, 1.0 / 60.0, 0.05, 0.08, 0.08, 0.1] {
        rt.frame(dt);
        check(&rt, "close full");
    }
}

fn preview_doc(units: Vec<crate::document_view::Unit>) -> crate::document_view::DocumentState {
    use std::sync::Arc;

    crate::document_view::DocumentState::new(Arc::new(crate::document_view::Document::new(units)))
        .key("preview-document")
}

fn assert_preview_body_settled(rt: &Runtime<FindHost>, animating: bool) {
    let els = rt.elements();
    let body_prefix = format!(
        "{}/",
        els.iter()
            .find(|e| !e.exiting && e.key.as_deref() == Some(BODY_KEY))
            .expect("preview-body")
            .path
    );
    let list_segment = format!("/{}/", rt.state.doc.list_key());
    let doc_rows: Vec<_> = els
        .iter()
        .filter(|e| e.path.starts_with(&body_prefix) && e.path.contains(&list_segment))
        .collect();
    assert!(
        !els.iter()
            .any(|e| e.path.starts_with(&body_prefix) && e.exiting)
    );
    assert!(!doc_rows.is_empty());
    assert!(doc_rows.iter().all(|e| (e.opacity - 1.0).abs() < 1e-4));
    assert!(!animating, "preview body updates must land immediately");
}

#[test]
fn preview_content_changes_land_immediately_while_drawer_slide_still_animates() {
    use crate::document_view::{Unit, UnitKind};

    let mut drawer = DrawerState::new();
    drawer.open_at_once();
    let look = Look::fallback(false);
    assert!(look.animations, "test requires animations enabled");

    let doc = preview_doc(vec![
        Unit::new(UnitKind::Heading(1), "First heading"),
        Unit::new(UnitKind::Paragraph, "First paragraph body."),
    ]);
    let mut rt = Runtime::new(
        FindHost { drawer, doc, look },
        find_host_view,
        (900.0, 500.0),
        Env::default(),
    );
    rt.poll = Some(find_host_poll);
    rt.frame(0.0);
    rt.settle_all();

    rt.update(|h, tx| {
        tx.animated();
        h.doc = preview_doc(vec![
            Unit::new(UnitKind::Heading(1), "Second heading"),
            Unit::new(UnitKind::Paragraph, "Second paragraph line one."),
            Unit::new(UnitKind::Paragraph, "Second paragraph line two."),
        ]);
    });
    let animating_after_swap = rt.frame(1.0 / 60.0);
    assert_preview_body_settled(&rt, animating_after_swap);

    rt.update(|h, tx| h.drawer.close(&h.look, tx));
    assert!(
        rt.frame(1.0 / 60.0),
        "outer drawer close must still animate"
    );
    assert!(
        rt.elements()
            .iter()
            .any(|e| e.key.as_deref() == Some("preview-drawer") && e.exiting),
        "preview-drawer must stay mounted during its exit slide"
    );
}
