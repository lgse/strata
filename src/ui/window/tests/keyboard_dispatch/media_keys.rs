// SPDX-License-Identifier: MIT

use super::*;
use crate::services::{
    LoadHandle, Preview, PreviewContent, PreviewEvent, PreviewProvider, PreviewRequest,
    SandboxedMedia,
};

struct MediaPreview;

impl PreviewProvider for MediaPreview {
    fn load(&self, request: PreviewRequest, emit: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
        emit(PreviewEvent::Ready(Preview {
            request_id: request.id,
            entry: request.entry,
            content_type: "video/mp4".into(),
            content: PreviewContent::SandboxedMedia {
                media: SandboxedMedia {
                    path: "/synthetic-video.mp4".into(),
                    size: request.media_size,
                    backend: crate::sandbox::MediaPreviewBackend::Software,
                },
            },
        }));
        LoadHandle::new(|| {})
    }
}

fn player(widget: &gtk::Widget) -> Option<crate::ui::media::DecodedMedia> {
    if let Some(media) = widget
        .downcast_ref::<gtk::Picture>()
        .and_then(|picture| picture.paintable())
        .and_downcast::<crate::ui::media::DecodedMedia>()
    {
        return Some(media);
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(media) = player(&widget) {
            return Some(media);
        }
        child = widget.next_sibling();
    }
    None
}

#[test]
fn media_modifiers_leave_plain_arrows_and_space_to_the_browser() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::media_keys::media_modifiers_leave_plain_arrows_and_space_to_the_browser",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::Icons, BrowserMode::List] {
                let fixture = KeyboardFixture::with_provider(Rc::new(MediaPreview));
                fixture.view.set_view_mode(mode);
                fixture.view.browser().focus_active();
                wait_until(|| fixture.view.item_view_has_focus());
                assert!(fixture.press(Key::space, ModifierType::empty()));
                let media = player(&fixture.preview.widget()).expect("preview player");
                crate::ui::media::tests::use_test_decoder(&media, false, 20_000_000);
                wait_until(|| media.is_prepared());
                let manager = ThemeManager::shared();
                manager.set_preview_volume(0.5);
                manager.set_preview_muted(false);
                let modifiers = ModifierType::CONTROL_MASK | ModifierType::ALT_MASK;
                assert!(fixture.press(Key::space, modifiers));
                assert!(!media.is_playing());
                assert!(fixture.press(Key::Down, modifiers));
                assert!((manager.preview_volume() - 0.4).abs() < 0.001);
                assert_eq!(fixture.selected(), vec![0]);
                assert!(fixture.press(Key::m, modifiers));
                assert!(manager.preview_muted());
                assert!(fixture.press(Key::m, modifiers));
                assert!(!manager.preview_muted());
                assert!(fixture.press(Key::Right, modifiers));
                wait_until(|| media.timestamp() >= 5_000_000);
                assert_eq!(fixture.selected(), vec![0]);
                let handled = fixture.press(Key::Down, ModifierType::empty());
                if mode == BrowserMode::Columns {
                    assert!(handled);
                    assert_eq!(fixture.selected(), vec![1]);
                } else {
                    assert!(
                        !handled,
                        "single-pane navigation must reach the native collection"
                    );
                }
                assert!((manager.preview_volume() - 0.4).abs() < 0.001);
                assert!(fixture.press(Key::space, ModifierType::empty()));
                assert!(!fixture.preview.is_enabled());
            }
        },
    );
}
