//! The gallery's goldens and scripts, one test module per page: `<id>::a11y`, `<id>::png` and
//! `<id>::scripts`. `mise run test c1::` runs one page's; `mise run bless c1` rewrites only
//! its goldens. See `docs/testing.md`.

mod a11y;
mod keys;
mod png;
mod registry;
mod scripts;
mod support;

macro_rules! pages {
    ($($id:ident),* $(,)?) => {
        /// Every page with golden tests, in registry order (checked by `registry`).
        const IDS: &[&str] = &[$(stringify!($id)),*];
        $(
            mod $id {
                #[test]
                fn a11y() {
                    crate::a11y::check(stringify!($id));
                }
                #[test]
                fn png() {
                    crate::png::check(stringify!($id));
                }
                #[test]
                fn scripts() {
                    crate::scripts::check(stringify!($id));
                }
            }
        )*
    };
}

pages!(
    gallery, w1, w2, k1, k2, k3, k4, c1, c2, c3, c4, c5, c6, c7, c8, c9, c10, c10a, c11, c11b, c12,
    c13, c14a, c14b, c15, c16, c17, c18, c19, c20a, c20b, c20c, c20d, c21, c22, c23, c24, c25, c26,
    x1,
);
