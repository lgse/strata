use super::*;

#[test]
fn no_wl_proxy_get_display_is_unsupported() {
    // libwayland 1.22 and older: the symbol isn't there.
    assert_eq!(
        get_display_with(|| None).err(),
        Some(Unsupported::LibwaylandBelow1_23)
    );
}
