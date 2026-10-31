// SPDX-License-Identifier: MIT

use gtk::gdk::{Key, ModifierType as Modifiers};

use crate::ui::keys::{Binding, Command};

catalog! {
    listed {
        MEDIA_PLAY_PAUSE: Command "media_play_pause" = Binding::ctrl_alt(Key::space);
        MEDIA_SEEK_BACK: Command "media_seek_back" = Binding::ctrl_alt(Key::Left);
        MEDIA_SEEK_FORWARD: Command "media_seek_forward" = Binding::ctrl_alt(Key::Right);
        MEDIA_VOLUME_UP: Command "media_volume_up" = Binding::ctrl_alt(Key::Up);
        MEDIA_VOLUME_DOWN: Command "media_volume_down" = Binding::ctrl_alt(Key::Down);
        MEDIA_MUTE: Command "media_mute" = Binding::ctrl_alt(Key::m);
        MEDIA_PREVIOUS: Command "media_previous" = Binding::ctrl_alt(Key::less);
        MEDIA_NEXT: Command "media_next" = Binding::ctrl_alt(Key::greater);
    }
    tenxer_only {
        // Without Ctrl+Alt, where the preview shows audio or video.
        TENXER_MEDIA_MUTE: Command "media_mute_in_preview" = Binding::plain(Key::m);
        TENXER_MEDIA_PREVIOUS: Command "media_previous_in_preview" = Binding::plain(Key::less);
        TENXER_MEDIA_NEXT: Command "media_next_in_preview" = Binding::plain(Key::greater);
        // The media timeline reads these in either map.
        MEDIA_SEEK_BACK_PAGE: Command "media_seek_back_page" = Binding::shift(Key::Left),
            live in both;
        MEDIA_SEEK_FORWARD_PAGE: Command "media_seek_forward_page" = Binding::shift(Key::Right),
            live in both;
        // AZERTY types 0 with Shift.
        MEDIA_SEEK_START: Command "media_seek_start" =
            Binding::plain(Key::_0).ignoring(Modifiers::SHIFT_MASK), live in both;
        MEDIA_SEEK_END: Command "media_seek_end" = Binding::shift(Key::g), live in both;
        MEDIA_VOLUME_STEP_UP: Command "media_volume_step_up" =>
            [Binding::plain(Key::plus), Binding::plain(Key::equal)], live in both;
        MEDIA_VOLUME_STEP_DOWN: Command "media_volume_step_down" = Binding::plain(Key::minus),
            live in both;
        SCROLL_PREVIEW_DOWN: Command "scroll_preview_down" = Binding::shift(Key::j);
        SCROLL_PREVIEW_UP: Command "scroll_preview_up" = Binding::shift(Key::k);
        /// 10xer opens find with **/** so Ctrl+F keeps paging the document.
        FIND_IN_PREVIEW: Command "find_in_preview" => default [Binding::ctrl(Key::f)],
            tenxer [Binding::plain(Key::slash)];
    }
    unlisted {
        PDF_ZOOM_RESET: Command "pdf_zoom_reset" = Binding::ctrl(Key::_0), live in both;
        PREVIOUS_MATCH: Command "previous_match" = Binding::shift(Key::Return), live in both;

        // GTK's own text keys, which a focused document keeps.
        DOCUMENT_START: Binding = Binding::ctrl(Key::Home);
        DOCUMENT_END: Binding = Binding::ctrl(Key::End);
    }
}
