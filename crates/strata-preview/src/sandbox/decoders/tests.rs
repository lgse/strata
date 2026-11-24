// SPDX-License-Identifier: MIT

use super::*;

const LISTING: &str = "Decoders:
 V..... = Video
 ------
 V....D 012v                 Uncompressed 4:2:2 10-bit
 VF...D webp                 WebP image
 V....D vp8                  On2 VP8 (webp_anim fallback)
";

#[test]
fn decoder_listings_match_whole_decoder_names_only() {
    let with_animation = format!("{LISTING} V.S..D webp_anim            Animated WebP image\n");
    let cases = [
        ("FFmpeg 9 listing", with_animation.as_str(), true),
        ("still WebP only, name in a description", LISTING, false),
        ("empty output", "", false),
    ];
    for (name, listing, expected) in cases {
        assert_eq!(lists_decoder(listing, "webp_anim"), expected, "{name}");
    }
}
