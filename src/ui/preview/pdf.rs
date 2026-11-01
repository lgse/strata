// SPDX-License-Identifier: MIT

use super::*;

impl PreviewState {
    pub(super) fn render_pdf_viewer(
        self: &Rc<Self>,
        entry: FileEntry,
        initial_png: Vec<u8>,
        initial_page: i32,
        pages: i32,
        initial_text_layer: Option<Arc<crate::services::PdfTextLayer>>,
    ) {
        let page_count = pages.clamp(0, 10_000);
        let labels: Vec<_> = (1..=page_count).map(|page| page.to_string()).collect();
        let labels: Vec<_> = labels.iter().map(String::as_str).collect();
        let model = gtk::StringList::new(&labels);
        let selection = gtk::NoSelection::new(Some(model));
        let factory = gtk::SignalListItemFactory::new();
        let zoom = Rc::new(Cell::new(PDF_MIN_ZOOM));
        let page_width = Rc::new(Cell::new(0));
        let visible_pages = Rc::new(RefCell::new(HashMap::<
            i32,
            (gtk::Overlay, gtk::Picture, gtk::DrawingArea),
        >::new()));
        let text_layers = Rc::new(RefCell::new(HashMap::<i32, Arc<PdfTextLayer>>::new()));
        let pdf_ranges = Rc::new(RefCell::new(HashMap::<i32, (usize, usize)>::new()));
        let pdf_drag = Rc::new(Cell::new(PdfDrag::Idle));
        let pdf_anchor = Rc::new(Cell::new((-1i32, 0usize)));
        let pdf_granularity = Rc::new(Cell::new(1u8));
        let pdf_press = Rc::new(RefCell::new((
            Instant::now(),
            f64::MAX,
            f64::MAX,
            -1i32,
            0u8,
        )));
        if let Some(layer) = initial_text_layer {
            text_layers.borrow_mut().insert(initial_page, layer);
        }

        factory.connect_setup(|_, item| {
            let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let overlay = gtk::Overlay::new();
            let picture = gtk::Picture::new();
            picture.set_can_shrink(true);
            picture.set_content_fit(gtk::ContentFit::Contain);
            picture.set_hexpand(true);
            picture.set_vexpand(true);
            let text_area = gtk::DrawingArea::new();
            text_area.set_hexpand(true);
            text_area.set_vexpand(true);
            text_area.set_accessible_role(gtk::AccessibleRole::Img);
            super::super::accessibility::set_label(&text_area, &crate::i18n::tr("PDF page text"));
            let spinner = gtk::Spinner::new();
            spinner.set_halign(gtk::Align::Center);
            spinner.set_valign(gtk::Align::Center);
            overlay.set_child(Some(&picture));
            overlay.add_overlay(&text_area);
            overlay.add_overlay(&spinner);
            overlay.set_hexpand(true);
            overlay.set_halign(gtk::Align::Center);
            overlay.set_size_request(-1, 560);
            item.set_child(Some(&overlay));
        });

        let provider = self.provider.clone();
        let loads = self.pdf_loads.clone();
        let render_size = self.media_preview_size();
        let initial_page = Rc::new(RefCell::new(Some((initial_page, initial_png))));
        let next_request = Rc::new(Cell::new(self.next_request.get().saturating_add(10_000)));
        let entry_for_bind = entry.clone();
        let page_width_for_bind = page_width.clone();
        let visible_pages_for_bind = visible_pages.clone();
        let layers_for_bind = text_layers.clone();
        let ranges_for_bind = pdf_ranges.clone();
        factory.connect_bind(move |_, item| {
            let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let page_index = item.position() as i32;
            let Some(overlay) = item.child().and_downcast::<gtk::Overlay>() else {
                return;
            };
            let Some(picture) = overlay.child().and_downcast::<gtk::Picture>() else {
                return;
            };
            let Some(spinner) = overlay.last_child().and_downcast::<gtk::Spinner>() else {
                return;
            };
            let Some(text_area) = picture.next_sibling().and_downcast::<gtk::DrawingArea>() else {
                return;
            };
            let binding_name = format!("pdf-page-{page_index}");
            overlay.set_widget_name(&binding_name);
            crate::ui::accessibility::set_description(&overlay, None);
            let target_width = page_width_for_bind.get();
            overlay.set_size_request(if target_width > 0 { target_width } else { -1 }, 560);
            picture.set_paintable(gtk::gdk::Paintable::NONE);
            spinner.start();
            spinner.set_visible(true);
            let layers_for_draw = layers_for_bind.clone();
            let ranges_for_draw = ranges_for_bind.clone();
            text_area.set_draw_func(move |_, cr, width, height| {
                let Some(layer) = layers_for_draw.borrow().get(&page_index).cloned() else {
                    return;
                };
                let Some(&(start, end)) = ranges_for_draw.borrow().get(&page_index) else {
                    return;
                };
                if start == end {
                    return;
                }
                let Some(color) = pdf_selection_color() else {
                    return;
                };
                let (ox, oy, s) =
                    pdf_text::image_bounds(&layer, f64::from(width), f64::from(height));
                cr.set_source_rgba(
                    f64::from(color.red()),
                    f64::from(color.green()),
                    f64::from(color.blue()),
                    0.42,
                );
                for [x1, y1, x2, y2] in pdf_text::selection_runs(&layer, start, end) {
                    let (rx, ry) = (ox + f64::from(x1) * s, oy + f64::from(y1) * s);
                    let (rw, rh) = (f64::from(x2 - x1) * s, f64::from(y2 - y1) * s);
                    let pad = rh * 0.06;
                    rounded_rect(
                        cr,
                        rx - pad,
                        ry - pad,
                        rw + pad * 2.0,
                        rh + pad * 2.0,
                        (rh * 0.16).min(3.0),
                    );
                }
                let _ = cr.fill();
            });
            visible_pages_for_bind.borrow_mut().insert(
                page_index,
                (overlay.clone(), picture.clone(), text_area.clone()),
            );

            let is_initial_page = initial_page
                .borrow()
                .as_ref()
                .is_some_and(|(page, _)| *page == page_index);
            let cached = if is_initial_page {
                initial_page.borrow_mut().take()
            } else {
                None
            };
            if let Some((_, png)) = cached {
                set_pdf_page_texture(&overlay, &picture, png, page_width_for_bind.get());
                spinner.stop();
                spinner.set_visible(false);
                return;
            }

            let request_id = PreviewRequestId(next_request.get());
            next_request.set(next_request.get().saturating_add(1));
            let weak_overlay = overlay.downgrade();
            let weak_picture = picture.downgrade();
            let weak_spinner = spinner.downgrade();
            let weak_text_area = text_area.downgrade();
            let loads_for_event = loads.clone();
            let layers_for_event = layers_for_bind.clone();
            let page_width_for_event = page_width_for_bind.clone();
            let emit = Rc::new(move |event| {
                loads_for_event.borrow_mut().remove(&page_index);
                let Some(overlay) = weak_overlay
                    .upgrade()
                    .filter(|overlay| overlay.widget_name() == binding_name)
                else {
                    return;
                };
                match event {
                    PreviewEvent::Ready(Preview {
                        request_id: response_id,
                        content:
                            PreviewContent::Pdf {
                                png,
                                page,
                                text_layer,
                                ..
                            },
                        ..
                    }) if response_id == request_id && page == page_index => {
                        if let Some(layer) = text_layer {
                            layers_for_event.borrow_mut().insert(page_index, layer);
                            if let Some(area) = weak_text_area.upgrade() {
                                area.queue_draw();
                            }
                        }
                        if let Some(picture) = weak_picture.upgrade() {
                            set_pdf_page_texture(
                                &overlay,
                                &picture,
                                png,
                                page_width_for_event.get(),
                            );
                        }
                    }
                    PreviewEvent::Failed {
                        request_id: response_id,
                        ..
                    } if response_id == request_id => {
                        crate::ui::accessibility::set_description(
                            &overlay,
                            Some(&crate::i18n::tr("Unable to render this PDF page")),
                        );
                    }
                    PreviewEvent::Progress { .. }
                    | PreviewEvent::Ready(_)
                    | PreviewEvent::Failed { .. }
                    | PreviewEvent::NeedsPassword { .. } => return,
                }
                if let Some(spinner) = weak_spinner.upgrade() {
                    spinner.stop();
                    spinner.set_visible(false);
                }
            });
            let load = provider.load(
                PreviewRequest {
                    id: request_id,
                    entry: entry_for_bind.clone(),
                    text_byte_limit: TEXT_BYTE_LIMIT,
                    render_document: false,
                    pdf_page: page_index,
                    media_size: render_size,
                    model_palette: super::super::theme::ThemeManager::shared()
                        .active_model_palette(),
                    model_view: Default::default(),
                    archive_password: None,
                },
                emit,
            );
            loads.borrow_mut().insert(page_index, load);
        });

        let loads = self.pdf_loads.clone();
        let visible_pages_for_unbind = visible_pages.clone();
        let layers_for_unbind = text_layers.clone();
        let ranges_for_unbind = pdf_ranges.clone();
        factory.connect_unbind(move |_, item| {
            if let Some(item) = item.downcast_ref::<gtk::ListItem>() {
                let page = item.position() as i32;
                loads.borrow_mut().remove(&page);
                visible_pages_for_unbind.borrow_mut().remove(&page);
                pdf_drop_unselected_layer(&layers_for_unbind, &ranges_for_unbind, page);
            }
        });

        let list = gtk::ListView::new(Some(selection), Some(factory));
        list.add_css_class("preview-pdf-list");
        list.set_hexpand(true);
        list.set_vexpand(true);
        let scroll = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .hexpand(true)
            .vexpand(true)
            .build();

        scroll.add_css_class("preview-pdf-scroll");

        let zoom_scroll =
            gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        zoom_scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak_scroll = scroll.downgrade();
        let zoom_for_scroll = zoom.clone();
        let page_width_for_scroll = page_width.clone();
        let visible_pages_for_scroll = visible_pages.clone();
        zoom_scroll.connect_scroll(move |controller, _, dy| {
            if !controller
                .current_event_state()
                .contains(gtk::gdk::ModifierType::CONTROL_MASK)
            {
                return glib::Propagation::Proceed;
            }
            if let Some(scroll) = weak_scroll.upgrade() {
                let next = pdf_zoom_after_scroll(zoom_for_scroll.get(), dy);
                let previous = apply_pdf_zoom(
                    &scroll,
                    &zoom_for_scroll,
                    &page_width_for_scroll,
                    &visible_pages_for_scroll.borrow(),
                    next,
                );
                if let Some(previous) = previous {
                    preserve_pdf_view_center(&scroll, next / previous);
                }
            }
            glib::Propagation::Stop
        });
        scroll.add_controller(zoom_scroll);

        let reset = gio::SimpleAction::new("reset-zoom", None);
        let weak_scroll = scroll.downgrade();
        let zoom_for_reset = zoom.clone();
        let page_width_for_reset = page_width.clone();
        let visible_pages_for_reset = visible_pages.clone();
        reset.connect_activate(move |_, _| {
            let Some(scroll) = weak_scroll.upgrade() else {
                return;
            };
            // No view-center correction: the list view re-anchors the visible page itself.
            apply_pdf_zoom(
                &scroll,
                &zoom_for_reset,
                &page_width_for_reset,
                &visible_pages_for_reset.borrow(),
                PDF_MIN_ZOOM,
            );
        });
        let actions = gio::SimpleActionGroup::new();
        actions.add_action(&reset);
        scroll.insert_action_group("pdf", Some(&actions));

        scroll.set_focusable(true);
        scroll.set_cursor_from_name(Some("grab"));
        let drag_origin = Rc::new(Cell::new((0.0, 0.0)));
        let pan = gtk::GestureDrag::new();
        pan.set_button(1);
        pan.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak_scroll = scroll.downgrade();
        let drag_origin_for_begin = drag_origin.clone();
        let pages_for_begin = visible_pages.clone();
        let layers_for_begin = text_layers.clone();
        let ranges_for_begin = pdf_ranges.clone();
        let drag_for_begin = pdf_drag.clone();
        let anchor_for_begin = pdf_anchor.clone();
        let granularity_for_begin = pdf_granularity.clone();
        let press_for_begin = pdf_press.clone();
        pan.connect_drag_begin(move |gesture, x, y| {
            let Some(scroll) = weak_scroll.upgrade() else {
                return;
            };
            let hit = pdf_page_at(
                &scroll,
                &pages_for_begin.borrow(),
                &layers_for_begin.borrow(),
                x,
                y,
            );
            let Some((page, _area, layer, px, py)) = hit else {
                drag_for_begin.set(PdfDrag::Pan);
                scroll.set_cursor_from_name(Some("grabbing"));
                scroll.grab_focus();
                drag_origin_for_begin
                    .set((scroll.hadjustment().value(), scroll.vadjustment().value()));
                anchor_for_begin.set((-1, 0));
                pdf_apply_ranges(
                    &ranges_for_begin,
                    &layers_for_begin,
                    &pages_for_begin.borrow(),
                    HashMap::new(),
                );
                return;
            };
            gesture.set_state(gtk::EventSequenceState::Claimed);
            drag_for_begin.set(PdfDrag::Select);
            scroll.grab_focus();
            let caret = pdf_text::caret_at(&layer, px, py);
            let shift = gesture
                .current_event_state()
                .contains(gtk::gdk::ModifierType::SHIFT_MASK);
            let mut press = press_for_begin.borrow_mut();
            let streak = if !shift
                && press.3 == page
                && press.0.elapsed() < Duration::from_millis(450)
                && (x - press.1).abs() <= 6.0
                && (y - press.2).abs() <= 6.0
            {
                (press.4 + 1).min(3)
            } else {
                1
            };
            *press = (Instant::now(), x, y, page, streak);
            drop(press);
            let extend = shift && anchor_for_begin.get().0 >= 0;
            granularity_for_begin.set(if extend { 1 } else { streak });
            let desired = if extend {
                pdf_desired_ranges(
                    &layers_for_begin.borrow(),
                    anchor_for_begin.get(),
                    (page, caret),
                    1,
                )
            } else {
                let unit = match streak {
                    2 => pdf_text::word_range(&layer, caret),
                    3 => pdf_text::line_range(&layer, caret),
                    _ => (caret, caret),
                };
                anchor_for_begin.set((page, unit.0));
                HashMap::from([(page, unit)])
                    .into_iter()
                    .filter(|(_, r)| r.0 != r.1)
                    .collect()
            };
            pdf_apply_ranges(
                &ranges_for_begin,
                &layers_for_begin,
                &pages_for_begin.borrow(),
                desired,
            );
        });
        let weak_scroll = scroll.downgrade();
        let pages_for_update = visible_pages.clone();
        let layers_for_update = text_layers.clone();
        let ranges_for_update = pdf_ranges.clone();
        let drag_for_update = pdf_drag.clone();
        let anchor_for_update = pdf_anchor.clone();
        let granularity_for_update = pdf_granularity.clone();
        pan.connect_drag_update(move |gesture, offset_x, offset_y| {
            let Some(scroll) = weak_scroll.upgrade() else {
                return;
            };
            if drag_for_update.get() == PdfDrag::Select {
                let Some((start_x, start_y)) = gesture.start_point() else {
                    return;
                };
                let pages = pages_for_update.borrow();
                let layers = layers_for_update.borrow();
                let Some((current_page, layer, px, py)) = pdf_page_near(
                    &scroll,
                    &pages,
                    &layers,
                    start_x + offset_x,
                    start_y + offset_y,
                ) else {
                    return;
                };
                let caret = pdf_text::caret_at(&layer, px, py);
                let desired = pdf_desired_ranges(
                    &layers,
                    anchor_for_update.get(),
                    (current_page, caret),
                    granularity_for_update.get(),
                );
                drop(layers);
                pdf_apply_ranges(&ranges_for_update, &layers_for_update, &pages, desired);
                return;
            }
            let (horizontal, vertical) = drag_origin.get();
            set_adjustment_value(&scroll.hadjustment(), horizontal - offset_x);
            set_adjustment_value(&scroll.vadjustment(), vertical - offset_y);
        });
        let weak_scroll = scroll.downgrade();
        let drag_for_end = pdf_drag.clone();
        pan.connect_drag_end(move |_, _, _| {
            let panned = drag_for_end.replace(PdfDrag::Idle) == PdfDrag::Pan;
            if panned && let Some(scroll) = weak_scroll.upgrade() {
                scroll.set_cursor_from_name(Some("grab"));
            }
        });
        scroll.add_controller(pan);

        let motion = gtk::EventControllerMotion::new();
        let weak_scroll = scroll.downgrade();
        let pages_for_motion = visible_pages.clone();
        let layers_for_motion = text_layers.clone();
        let drag_for_motion = pdf_drag.clone();
        motion.connect_motion(move |_, x, y| {
            if drag_for_motion.get() != PdfDrag::Idle {
                return;
            }
            let Some(scroll) = weak_scroll.upgrade() else {
                return;
            };
            let over_text = pdf_page_at(
                &scroll,
                &pages_for_motion.borrow(),
                &layers_for_motion.borrow(),
                x,
                y,
            )
            .is_some();
            scroll.set_cursor_from_name(Some(if over_text { "text" } else { "grab" }));
        });
        scroll.add_controller(motion);

        let keys = gtk::EventControllerKey::new();
        let weak_scroll = scroll.downgrade();
        let pages_for_keys = visible_pages.clone();
        let layers_for_keys = text_layers.clone();
        let ranges_for_keys = pdf_ranges.clone();
        keys::on_press(&keys, move |_, press| {
            let Some(scroll) = weak_scroll.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if press.is(SELECT_ALL) {
                let layers = layers_for_keys.borrow();
                if layers.is_empty() {
                    return glib::Propagation::Proceed;
                }
                let desired = layers
                    .iter()
                    .map(|(page, layer)| (*page, (0, pdf_text::len(layer))))
                    .collect();
                drop(layers);
                pdf_apply_ranges(
                    &ranges_for_keys,
                    &layers_for_keys,
                    &pages_for_keys.borrow(),
                    desired,
                );
                glib::Propagation::Stop
            } else if press.is(COPY) {
                let text = pdf_selected_text(&layers_for_keys.borrow(), &ranges_for_keys.borrow());
                if text.is_empty() {
                    return glib::Propagation::Proceed;
                }
                scroll.clipboard().set_text(&text);
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        scroll.add_controller(keys);

        let zoom_for_tick = zoom.clone();
        let page_width_for_tick = page_width.clone();
        let visible_pages_for_tick = visible_pages.clone();
        scroll.add_tick_callback(move |scroll, _| {
            if scroll.width() > PDF_PAGE_GAP * 2 + 1 {
                let width = pdf_page_width(scroll, zoom_for_tick.get());
                if width != page_width_for_tick.replace(width) {
                    resize_pdf_pages(&visible_pages_for_tick.borrow(), width);
                }
            }
            glib::ControlFlow::Continue
        });
        self.content.append(&scroll);
    }
}

pub(super) fn set_pdf_page_texture(
    overlay: &gtk::Overlay,
    picture: &gtk::Picture,
    png: Vec<u8>,
    target_width: i32,
) {
    let Ok(texture) = gtk::gdk::Texture::from_bytes(&glib::Bytes::from_owned(png)) else {
        return;
    };
    picture.set_paintable(Some(&texture));
    resize_pdf_page(overlay, picture, target_width);
}

pub(super) fn pdf_zoom_after_scroll(current: f64, dy: f64) -> f64 {
    (current * (-dy * 0.14).exp()).clamp(PDF_MIN_ZOOM, PDF_MAX_ZOOM)
}

pub(super) fn pdf_page_width(scroll: &gtk::ScrolledWindow, zoom: f64) -> i32 {
    let fit_width = scroll
        .width()
        .min(media_layout::MAX_CONTENT_WIDTH)
        .saturating_sub(PDF_PAGE_GAP * 2)
        .max(1);
    (f64::from(fit_width) * zoom).round() as i32
}

pub(super) fn apply_pdf_zoom(
    scroll: &gtk::ScrolledWindow,
    zoom: &Cell<f64>,
    page_width: &Cell<i32>,
    pages: &HashMap<i32, (gtk::Overlay, gtk::Picture, gtk::DrawingArea)>,
    next: f64,
) -> Option<f64> {
    let previous = zoom.replace(next);
    if (next - previous).abs() < f64::EPSILON {
        return None;
    }
    let width = pdf_page_width(scroll, next);
    page_width.set(width);
    resize_pdf_pages(pages, width);
    Some(previous)
}

pub(super) fn resize_pdf_pages(
    pages: &HashMap<i32, (gtk::Overlay, gtk::Picture, gtk::DrawingArea)>,
    width: i32,
) {
    for (overlay, picture, _) in pages.values() {
        resize_pdf_page(overlay, picture, width);
    }
}

pub(super) fn resize_pdf_page(overlay: &gtk::Overlay, picture: &gtk::Picture, target_width: i32) {
    let Some(paintable) = picture.paintable() else {
        overlay.set_size_request(if target_width > 0 { target_width } else { -1 }, 560);
        return;
    };
    let texture_width = paintable.intrinsic_width();
    let texture_height = paintable.intrinsic_height();
    let width = if target_width > 0 {
        target_width
    } else if overlay.width() > 1 {
        overlay.width()
    } else {
        return;
    };
    if texture_width > 0 && texture_height > 0 {
        let ratio = f64::from(texture_width) / f64::from(texture_height);
        overlay.set_size_request(width, (f64::from(width) / ratio).round() as i32);
    }
}

pub(super) fn preserve_pdf_view_center(scroll: &gtk::ScrolledWindow, factor: f64) {
    let horizontal = scroll.hadjustment();
    let vertical = scroll.vadjustment();
    let horizontal_center = horizontal.value() + horizontal.page_size() / 2.0;
    let vertical_center = vertical.value() + vertical.page_size() / 2.0;
    glib::idle_add_local_once(glib::clone!(
        #[weak]
        scroll,
        move || {
            set_adjustment_value(
                &scroll.hadjustment(),
                horizontal_center * factor - scroll.hadjustment().page_size() / 2.0,
            );
            set_adjustment_value(
                &scroll.vadjustment(),
                vertical_center * factor - scroll.vadjustment().page_size() / 2.0,
            );
        }
    ));
}

pub(super) fn set_adjustment_value(adjustment: &gtk::Adjustment, value: f64) {
    let maximum = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
    adjustment.set_value(value.clamp(adjustment.lower(), maximum));
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum PdfDrag {
    Idle,
    Pan,
    Select,
}

pub(super) fn pdf_page_at(
    scroll: &gtk::ScrolledWindow,
    pages: &HashMap<i32, (gtk::Overlay, gtk::Picture, gtk::DrawingArea)>,
    layers: &HashMap<i32, Arc<PdfTextLayer>>,
    x: f64,
    y: f64,
) -> Option<(i32, gtk::DrawingArea, Arc<PdfTextLayer>, f32, f32)> {
    for (page, (_, _, area)) in pages {
        let Some(layer) = layers.get(page) else {
            continue;
        };
        let Some(point) =
            scroll.compute_point(area, &gtk::graphene::Point::new(x as f32, y as f32))
        else {
            continue;
        };
        let (ox, oy, s) =
            pdf_text::image_bounds(layer, f64::from(area.width()), f64::from(area.height()));
        let px = (f64::from(point.x()) - ox) / s;
        let py = (f64::from(point.y()) - oy) / s;
        if px >= 0.0
            && py >= 0.0
            && px <= f64::from(layer.width)
            && py <= f64::from(layer.height)
            && pdf_text::hit_text(layer, px as f32, py as f32)
        {
            return Some((*page, area.clone(), layer.clone(), px as f32, py as f32));
        }
    }
    None
}

pub(super) fn pdf_desired_ranges(
    layers: &HashMap<i32, Arc<PdfTextLayer>>,
    anchor: (i32, usize),
    caret: (i32, usize),
    granularity: u8,
) -> HashMap<i32, (usize, usize)> {
    let (anchor_page, anchor) = anchor;
    let (current_page, caret) = caret;
    let snap_lo = |layer: &PdfTextLayer, index: usize| match granularity {
        2 => pdf_text::word_range(layer, index).0,
        3 => pdf_text::line_range(layer, index).0,
        _ => index,
    };
    let snap_hi = |layer: &PdfTextLayer, index: usize| match granularity {
        2 => pdf_text::word_range(layer, index).1,
        3 => pdf_text::line_range(layer, index).1,
        _ => index,
    };
    let (first, last, first_start, last_end) = if anchor_page <= current_page {
        (anchor_page, current_page, anchor, caret)
    } else {
        (current_page, anchor_page, caret, anchor)
    };
    let mut desired = HashMap::new();
    for page in first..=last {
        let Some(layer) = layers.get(&page) else {
            continue;
        };
        let len = pdf_text::len(layer);
        let (start, end) = match (page == first, page == last) {
            (true, true) => (
                snap_lo(layer, first_start.min(last_end)),
                snap_hi(layer, first_start.max(last_end)),
            ),
            (true, false) => (snap_lo(layer, first_start), len),
            (false, true) => (0, snap_hi(layer, last_end)),
            _ => (0, len),
        };
        let (start, end) = (start.min(end), end.min(len));
        if start != end {
            desired.insert(page, (start, end));
        }
    }
    desired
}

pub(super) fn pdf_drop_unselected_layer(
    layers: &RefCell<HashMap<i32, Arc<PdfTextLayer>>>,
    ranges: &RefCell<HashMap<i32, (usize, usize)>>,
    page: i32,
) {
    if !ranges.borrow().contains_key(&page) {
        layers.borrow_mut().remove(&page);
    }
}

pub(super) fn pdf_apply_ranges(
    ranges: &RefCell<HashMap<i32, (usize, usize)>>,
    layers: &RefCell<HashMap<i32, Arc<PdfTextLayer>>>,
    pages: &HashMap<i32, (gtk::Overlay, gtk::Picture, gtk::DrawingArea)>,
    desired: HashMap<i32, (usize, usize)>,
) {
    let mut selected = ranges.borrow_mut();
    let mut dirty: Vec<i32> = desired
        .iter()
        .filter(|(page, range)| selected.get(*page) != Some(range))
        .map(|(page, _)| *page)
        .collect();
    dirty.extend(
        selected
            .keys()
            .filter(|page| !desired.contains_key(*page))
            .copied(),
    );
    *selected = desired;
    drop(selected);
    for page in &dirty {
        if !pages.contains_key(page) {
            pdf_drop_unselected_layer(layers, ranges, *page);
        }
    }
    for page in dirty {
        if let Some((_, _, area)) = pages.get(&page) {
            area.queue_draw();
        }
    }
}

pub(super) fn pdf_page_near(
    scroll: &gtk::ScrolledWindow,
    pages: &HashMap<i32, (gtk::Overlay, gtk::Picture, gtk::DrawingArea)>,
    layers: &HashMap<i32, Arc<PdfTextLayer>>,
    x: f64,
    y: f64,
) -> Option<(i32, Arc<PdfTextLayer>, f32, f32)> {
    let mut nearest: Option<(i32, Arc<PdfTextLayer>, f32, f32, f64)> = None;
    for (page, (_, _, area)) in pages {
        let Some(layer) = layers.get(page) else {
            continue;
        };
        let Some(point) =
            scroll.compute_point(area, &gtk::graphene::Point::new(x as f32, y as f32))
        else {
            continue;
        };
        let (ox, oy, s) =
            pdf_text::image_bounds(layer, f64::from(area.width()), f64::from(area.height()));
        let (px, py) = (
            (f64::from(point.x()) - ox) / s,
            (f64::from(point.y()) - oy) / s,
        );
        let dx = px.clamp(0.0, f64::from(layer.width)) - px;
        let dy = py.clamp(0.0, f64::from(layer.height)) - py;
        let distance = dx * dx + dy * dy;
        let better = nearest.as_ref().is_none_or(|(.., best)| distance < *best);
        if better {
            nearest = Some((
                *page,
                layer.clone(),
                (px.clamp(0.0, f64::from(layer.width))) as f32,
                (py.clamp(0.0, f64::from(layer.height))) as f32,
                distance,
            ));
        }
    }
    nearest.map(|(page, layer, px, py, _)| (page, layer, px, py))
}

pub(super) fn pdf_selected_text(
    layers: &HashMap<i32, Arc<PdfTextLayer>>,
    ranges: &HashMap<i32, (usize, usize)>,
) -> String {
    let mut pages: Vec<_> = ranges.iter().collect();
    pages.sort_by_key(|(page, _)| **page);
    let mut text = String::new();
    for (page, &(start, end)) in pages {
        let Some(layer) = layers.get(page) else {
            continue;
        };
        let part = pdf_text::selection_text(layer, start, end);
        if part.is_empty() {
            continue;
        }
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&part);
    }
    text
}

pub(super) fn pdf_selection_color() -> Option<gtk::gdk::RGBA> {
    crate::ui::theme::ThemeManager::shared()
        .current_tokens()
        .and_then(|tokens| gtk::gdk::RGBA::parse(&tokens.accent).ok())
}

pub(super) fn rounded_rect(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    use std::f64::consts::{FRAC_PI_2, PI};
    let r = r.min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, FRAC_PI_2, PI);
    cr.arc(x + r, y + r, r, PI, 3.0 * FRAC_PI_2);
    cr.close_path();
}
