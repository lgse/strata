use super::{Content, TEXT_TYPES};
use crate::clipboard::Data;

#[test]
fn text_goes_by_every_text_name_when_it_fits_latin1() {
    let c = Content::new().text("café");
    assert_eq!(c.mimes().collect::<Vec<_>>(), TEXT_TYPES);
}

#[test]
fn text_beyond_latin1_leaves_out_text_and_string() {
    let c = Content::new().text("✓ done");
    assert_eq!(
        c.mimes().collect::<Vec<_>>(),
        ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain"]
    );
}

#[test]
fn string_is_latin1_and_reads_back_as_the_same_text() {
    let (spec, _) = Content::new().text("café").into_parts();
    let string = spec.writer("STRING").and_then(|w| w.get()).expect("STRING");
    assert_eq!(&*string, [b'c', b'a', b'f', 0xe9]);
    let utf8 = spec
        .writer("UTF8_STRING")
        .and_then(|w| w.get())
        .expect("UTF8_STRING");
    assert_eq!(&*utf8, "café".as_bytes());
    for (mime, bytes) in [("STRING", &*string), ("UTF8_STRING", &*utf8)] {
        let d = Data {
            mime: mime.into(),
            bytes: bytes.to_vec(),
        };
        assert_eq!(d.text().as_deref(), Some("café"), "{mime}");
    }
}

#[test]
fn a_type_offered_twice_keeps_its_first_data() {
    let (spec, _) = Content::new()
        .with("text/x-a", "first")
        .with("text/x-a", "second")
        .into_parts();
    assert_eq!(spec.mimes().count(), 1);
    assert_eq!(
        &*spec.writer("text/x-a").and_then(|w| w.get()).expect("data"),
        b"first"
    );
}

#[test]
fn data_that_isnt_a_text_type_has_no_text() {
    let d = Data {
        mime: "image/png".into(),
        bytes: b"abc".to_vec(),
    };
    assert_eq!(d.text(), None);
}
