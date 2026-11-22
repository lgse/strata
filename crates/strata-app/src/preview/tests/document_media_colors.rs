// SPDX-License-Identifier: MIT

//! Behavioral regressions for UI-44: theme-colored LaTeX equations and Mermaid diagrams in
//! rendered document previews (`crates/strata-app/src/preview/document.rs`).

use std::sync::Arc;

use rune::{Pixels, Tx};
use strata_components::{
    code_view::{CodeState, highlight::Highlighter},
    document_view::MediaState,
    preview_drawer::DocumentView,
};
use strata_fs::services::{DocumentLayout, DocumentMedia, DocumentUnit, DocumentUnitKind};
use strata_kit::{
    look::Look,
    theme::{Library, Rgba8, syntax::syntect_theme},
};

use super::{
    super::{
        document::{DocumentSpec, RenderedDocument, diagram_palette, tint_diagram_rgba},
        viewer::{RenderedBody, Viewer},
    },
    MODULE,
};
use crate::{
    App, shell,
    slots::{Inbox, Later},
    test_support::hosted,
};

fn theme_look(id: &str) -> Look {
    Library::new(Vec::new()).resolve(Some(id)).look()
}

fn media_unit(source: DocumentMedia, alt: &str) -> DocumentUnit {
    DocumentUnit {
        kind: DocumentUnitKind::Media {
            source,
            list_depth: None,
        },
        text: alt.to_owned(),
        copy_text: String::new(),
        spans: Vec::new(),
        wrap: true,
        first: true,
        last: true,
    }
}

fn sample_rgba() -> Vec<u8> {
    vec![
        0, 0, 0, 255, // pure black ink
        255, 255, 255, 255, // pure white background/fill
        0, 0, 0, 0, // fully transparent pixel
        0, 0, 0, 96, // translucent antialiased ink
        220, 40, 30, 255, // vibrant colored pixel (must survive on Image)
    ]
}

fn ready_pixels(doc: &RenderedDocument, unit: usize) -> &Pixels {
    match doc.state.media(unit) {
        MediaState::Ready(pixels) => pixels,
        other => panic!("expected Ready pixels for unit {unit}, got {other:?}"),
    }
}

fn rgba_slice(rgba: &[u8], pixel: usize) -> [u8; 4] {
    let start = pixel * 4;
    [
        rgba[start],
        rgba[start + 1],
        rgba[start + 2],
        rgba[start + 3],
    ]
}

fn rgb(color: Rgba8, alpha: u8) -> [u8; 4] {
    [color.r, color.g, color.b, alpha]
}

fn make_mixed_document(
    look: &Look,
    later: Later<App>,
) -> (RenderedDocument, Vec<super::super::document::MediaTask>) {
    let layout = DocumentLayout {
        units: vec![
            media_unit(
                DocumentMedia::Math {
                    source: "E = mc^2".to_owned(),
                    display: true,
                },
                "E = mc^2",
            ),
            media_unit(
                DocumentMedia::Math {
                    source: "x^2".to_owned(),
                    display: false,
                },
                "x^2",
            ),
            media_unit(
                DocumentMedia::Mermaid("flowchart LR\nA --> B".to_owned()),
                "Mermaid diagram",
            ),
            media_unit(DocumentMedia::Image("photo.png".to_owned()), "Photo"),
        ],
    };
    let blocks = Highlighter::new(Arc::new(syntect_theme(look))).blocks();
    RenderedDocument::new_without_pump(DocumentSpec {
        layout,
        warnings: Vec::new(),
        wrap: Some(true),
        source_available: true,
        document_path: None,
        blocks,
        later,
        palette: diagram_palette(look),
    })
}

#[test]
fn luminance_remap_maps_black_to_text_white_to_surface_and_preserves_alpha() {
    let raw = vec![
        0, 0, 0, 255, // black ink
        255, 255, 255, 255, // white background
        0, 0, 0, 0, // transparent
        0, 0, 0, 96, // translucent ink
        0, 255, 0, 255, // pure green (luma = 0.7152)
    ];

    for theme_id in ["catppuccin-mocha", "catppuccin-latte"] {
        let look = theme_look(theme_id);
        let palette = diagram_palette(&look);
        let out = tint_diagram_rgba(&raw, palette);

        assert_eq!(
            rgba_slice(&out, 0),
            rgb(palette.text, 255),
            "{theme_id}: black ink maps to theme text"
        );
        assert_eq!(
            rgba_slice(&out, 1),
            rgb(palette.surface, 255),
            "{theme_id}: white background maps to theme surface"
        );
        assert_eq!(
            rgba_slice(&out, 2)[3],
            0,
            "{theme_id}: transparent pixel keeps alpha 0"
        );
        assert_eq!(
            rgba_slice(&out, 3),
            rgb(palette.text, 96),
            "{theme_id}: translucent ink preserves straight alpha"
        );

        let expected_green = |t: u8, s: u8| {
            (f32::from(t) + 0.7152 * (f32::from(s) - f32::from(t)))
                .clamp(0.0, 255.0)
                .round() as u8
        };
        assert_eq!(
            rgba_slice(&out, 4),
            [
                expected_green(palette.text.r, palette.surface.r),
                expected_green(palette.text.g, palette.surface.g),
                expected_green(palette.text.b, palette.surface.b),
                255,
            ],
            "{theme_id}: green channel uses Rec. 709 luma weight"
        );
    }

    // Sanity check: dark theme text must be visibly lighter than its surface.
    let dark = diagram_palette(&theme_look("catppuccin-mocha"));
    assert!(
        u16::from(dark.text.r) + u16::from(dark.text.g) + u16::from(dark.text.b)
            > u16::from(dark.surface.r) + u16::from(dark.surface.g) + u16::from(dark.surface.b),
        "dark theme ink must be brighter than dark surface"
    );
}

#[test]
fn rendered_document_tints_math_and_mermaid_but_not_images_and_updates_on_theme_switch() {
    let dark_look = theme_look("catppuccin-mocha");
    let light_look = theme_look("catppuccin-latte");
    let dark = diagram_palette(&dark_look);
    let light = diagram_palette(&light_look);
    assert_ne!(dark, light);

    let (mut doc, tasks) = make_mixed_document(&dark_look, Inbox::new().later());
    assert_eq!(tasks.len(), 4);

    let doc_id = doc.id();
    let sample = sample_rgba();
    for task in &tasks {
        assert!(doc.on_media_ready(doc_id, task, Ok((5, 1, sample.clone())), &dark_look));
    }
    assert!(
        !doc.has_active_media_tasks(),
        "delivering synthetic media leaves no sandbox tasks running"
    );

    let dark_sigs: Vec<(u64, u64)> = (0..4)
        .map(|unit| ready_pixels(&doc, unit).signature())
        .collect();

    // Units 0..=2 (display Math, inline Math, Mermaid) map black->text, white->surface, alpha kept.
    for unit in 0..3 {
        let rgba = ready_pixels(&doc, unit).rgba();
        assert_eq!(rgba_slice(&rgba, 0), rgb(dark.text, 255));
        assert_eq!(rgba_slice(&rgba, 1), rgb(dark.surface, 255));
        assert_eq!(rgba_slice(&rgba, 2)[3], 0);
        assert_eq!(rgba_slice(&rgba, 3), rgb(dark.text, 96));
    }
    // Unit 3 (Image) keeps its original colors completely untouched.
    assert_eq!(ready_pixels(&doc, 3).rgba(), sample);

    // Live theme switch updates Math and Mermaid in-place without re-running the sandbox.
    assert!(doc.sync_theme(&light_look));
    assert!(
        !doc.sync_theme(&light_look),
        "repeating sync_theme with the same palette is a no-op"
    );
    assert!(
        !doc.has_active_media_tasks(),
        "theme switch must never re-queue sandbox rendering"
    );

    for (unit, &(old_id, old_gen)) in dark_sigs.iter().enumerate().take(3) {
        let pixels = ready_pixels(&doc, unit);
        let sig = pixels.signature();
        assert_eq!(
            sig.0, old_id,
            "unit {unit} updates the existing Pixels handle in-place"
        );
        assert!(
            sig.1 > old_gen,
            "unit {unit} bumps Pixels generation on theme change"
        );
        let rgba = pixels.rgba();
        assert_eq!(rgba_slice(&rgba, 0), rgb(light.text, 255));
        assert_eq!(rgba_slice(&rgba, 1), rgb(light.surface, 255));
        assert_eq!(rgba_slice(&rgba, 2)[3], 0);
        assert_eq!(rgba_slice(&rgba, 3), rgb(light.text, 96));
    }

    // Regular Image remains completely unchanged across the theme switch.
    assert_eq!(ready_pixels(&doc, 3).signature(), dark_sigs[3]);
    assert_eq!(ready_pixels(&doc, 3).rgba(), sample);
}

#[test]
fn live_theme_switch_in_app_updates_open_document_media_without_sandbox() {
    hosted(
        MODULE,
        "document_media_colors::live_theme_switch_in_app_updates_open_document_media_without_sandbox",
        |mut host| {
            let sample = sample_rgba();
            let switch_theme = |host: &mut crate::host::offscreen::Offscreen,
                                theme_id: &'static str| {
                host.runtime().update(move |app: &mut App, tx: &mut Tx| {
                    shell::saved(app).change(|prefs| {
                        prefs.mode = "manual".to_owned();
                        prefs.theme = theme_id.to_owned();
                    });
                    shell::follow_look_tx(app, tx);
                });
                while host.turn() {}
            };

            switch_theme(&mut host, "catppuccin-mocha");
            let dark = diagram_palette(&host.app().look);

            // Mount a RenderedDocument with Math, Mermaid, and Image media without spawning sandbox.
            let sample_for_load = sample.clone();
            host.runtime().update(move |app: &mut App, _: &mut Tx| {
                let (doc, tasks) = make_mixed_document(&app.look, app.preview.loader.later.clone());
                let doc_id = doc.id();
                app.preview.viewer = Viewer::document(
                    DocumentView::Rendered,
                    Some(RenderedBody::Document(Box::new(doc))),
                    CodeState::new("# source".to_owned()),
                    false,
                );
                for task in &tasks {
                    assert!(app.preview.viewer.on_media_ready(
                        doc_id,
                        task,
                        Ok((5, 1, sample_for_load.clone())),
                        &app.look,
                    ));
                }
            });
            while host.turn() {}

            let check_media = |app: &App, expected: super::super::document::DiagramPalette| {
                let Viewer::Document(viewer) = &app.preview.viewer else {
                    panic!("expected Document viewer");
                };
                let doc = viewer
                    .rendered_document_for_test()
                    .expect("expected RenderedDocument");
                assert!(!doc.has_active_media_tasks());
                for unit in 0..3 {
                    let rgba = ready_pixels(doc, unit).rgba();
                    assert_eq!(rgba_slice(&rgba, 0), rgb(expected.text, 255));
                    assert_eq!(rgba_slice(&rgba, 1), rgb(expected.surface, 255));
                    assert_eq!(rgba_slice(&rgba, 2)[3], 0);
                }
                assert_eq!(ready_pixels(doc, 3).rgba(), sample);
            };

            check_media(host.app(), dark);

            // Switch live from dark to light: open RenderedDocument updates immediately.
            switch_theme(&mut host, "catppuccin-latte");
            let light = diagram_palette(&host.app().look);
            assert_ne!(dark, light);
            check_media(host.app(), light);

            // Switch theme while viewing Source, then return to Rendered: media already matches.
            host.runtime().update(|app: &mut App, _: &mut Tx| {
                assert!(app.preview.viewer.show_view(DocumentView::Source));
            });
            while host.turn() {}
            switch_theme(&mut host, "catppuccin-mocha");
            host.runtime().update(|app: &mut App, _: &mut Tx| {
                assert!(app.preview.viewer.show_view(DocumentView::Rendered));
            });
            while host.turn() {}
            check_media(host.app(), dark);
        },
    );
}
