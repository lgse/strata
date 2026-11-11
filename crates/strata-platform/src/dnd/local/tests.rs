use super::*;

/// The owner's ruling (review round 3): the token names no process, only the drag: a counter
/// and a nonce, which other clients may see.
#[test]
fn each_drag_gets_its_own_token_with_no_process_id() {
    let (a, b) = (new_token(), new_token());
    assert_ne!(a, b);
    for t in [&a, &b] {
        assert!(is_token(t), "{t}");
        let id = t
            .strip_prefix("application/x-local-drag;id=")
            .expect("prefix");
        let parts: Vec<&str> = id.split('.').collect();
        assert_eq!(parts.len(), 2, "a counter and a nonce, nothing else: {t}");
        assert!(parts[0].parse::<u64>().is_ok(), "{t}");
        assert_eq!(parts[1].len(), 16, "{t}");
        assert!(parts[1].chars().all(|c| c.is_ascii_hexdigit()), "{t}");
    }
}

#[test]
fn tokens_are_hidden_from_what_an_app_sees() {
    let mimes = vec![
        "text/uri-list".to_owned(),
        new_token(),
        "application/x-local-dragged-thing".to_owned(),
    ];
    assert_eq!(token_in(&mimes), Some(mimes[1].as_str()));
    assert_eq!(
        visible(&mimes),
        vec!["text/uri-list", "application/x-local-dragged-thing"],
        "a type that only starts like the token stays"
    );
}
