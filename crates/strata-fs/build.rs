// SPDX-License-Identifier: MIT

fn main() {
    // Two merge tests copy from bundled resources, the one location without paths that
    // needs no running service.
    glib_build_tools::compile_resources(
        &["../../data"],
        "test-resources.gresource.xml",
        "strata.gresource",
    );
}
