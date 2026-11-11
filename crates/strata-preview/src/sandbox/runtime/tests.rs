// SPDX-License-Identifier: MIT

use std::{collections::HashMap, ffi::OsString, os::unix::ffi::OsStringExt};

use super::{
    ALLOWED_RUNTIME_ROOTS, GDK_PIXBUF_MODULE_FILE_VARIABLE, PATH_VARIABLE, PRLIMIT_VARIABLE,
    ROOT_VARIABLE, Runtime, resolve,
};

fn resolve_with(variables: &[(&str, &str)]) -> Runtime {
    let variables: HashMap<&str, OsString> = variables
        .iter()
        .map(|(name, value)| (*name, OsString::from(value)))
        .collect();
    resolve(|name| variables.get(name).cloned())
}

#[test]
fn launch_time_values_override_the_embedded_configuration() {
    let runtime = resolve_with(&[
        (PATH_VARIABLE, "/nix/store/ffmpeg/bin:/nix/store/magick/bin"),
        (ROOT_VARIABLE, "/nix/store"),
        (PRLIMIT_VARIABLE, "/nix/store/util-linux/bin/prlimit"),
        (
            GDK_PIXBUF_MODULE_FILE_VARIABLE,
            "/nix/store/loaders/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache",
        ),
    ]);

    assert_eq!(
        runtime,
        Runtime {
            path: "/nix/store/ffmpeg/bin:/nix/store/magick/bin".to_owned(),
            root: "/nix/store".to_owned(),
            prlimit: "/nix/store/util-linux/bin/prlimit".to_owned(),
            gdk_pixbuf_module_file: Some(
                "/nix/store/loaders/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache".to_owned()
            ),
        }
    );
    assert_eq!(
        runtime.tool_directories().collect::<Vec<_>>(),
        [
            std::path::Path::new("/nix/store/ffmpeg/bin"),
            std::path::Path::new("/nix/store/magick/bin"),
        ]
    );
}

#[test]
fn unset_variables_keep_the_embedded_configuration() {
    assert_eq!(resolve_with(&[]), Runtime::embedded());
    let runtime = resolve_with(&[(ROOT_VARIABLE, "/gnu/store")]);
    assert_eq!(runtime.root, "/gnu/store");
    assert_eq!(runtime.path, Runtime::embedded().path);
    assert_eq!(runtime.prlimit, Runtime::embedded().prlimit);
}

#[test]
fn unsafe_roots_are_ignored() {
    let embedded = Runtime::embedded();
    for root in [
        "",
        "/",
        "/home",
        "/home/alice",
        "/root",
        "/tmp",
        "/run/user/1000",
        "/usr/..",
        "/usr/../home",
        "/nix/store/./..",
        "usr",
        "nix/store",
        "/usrdata",
    ] {
        let runtime = resolve_with(&[(ROOT_VARIABLE, root)]);
        assert_eq!(runtime.root, embedded.root, "{root:?} must be rejected");
    }
    for root in ALLOWED_RUNTIME_ROOTS {
        let nested = format!("{root}/fhs-env/usr");
        for accepted in [*root, nested.as_str()] {
            let runtime = resolve_with(&[(ROOT_VARIABLE, accepted)]);
            assert_eq!(runtime.root, accepted, "{accepted:?} must be accepted");
        }
    }
}

#[test]
fn relative_empty_and_traversing_paths_are_ignored() {
    let embedded = Runtime::embedded();
    for path in [
        "",
        "bin",
        "/usr/bin:",
        ":/usr/bin",
        "/usr/bin:relative",
        "/usr/../bin",
    ] {
        let runtime = resolve_with(&[(PATH_VARIABLE, path)]);
        assert_eq!(runtime.path, embedded.path, "{path:?} must be rejected");
    }
    for value in ["", "prlimit", "/usr/bin/../bin/prlimit", "usr/bin/prlimit"] {
        let runtime = resolve_with(&[
            (PRLIMIT_VARIABLE, value),
            (GDK_PIXBUF_MODULE_FILE_VARIABLE, value),
        ]);
        assert_eq!(
            runtime.prlimit, embedded.prlimit,
            "{value:?} must be rejected"
        );
        assert_eq!(
            runtime.gdk_pixbuf_module_file, embedded.gdk_pixbuf_module_file,
            "{value:?} must be rejected"
        );
    }
}

#[test]
fn non_utf8_values_are_ignored() {
    let embedded = Runtime::embedded();
    let runtime = resolve(|name| {
        (name == ROOT_VARIABLE || name == PATH_VARIABLE)
            .then(|| OsString::from_vec(vec![b'/', b'u', 0xff]))
    });
    assert_eq!(runtime, embedded);
}
