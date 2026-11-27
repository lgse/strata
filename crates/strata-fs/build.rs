// SPDX-License-Identifier: MIT

fn main() {
    glib_build_tools::compile_resources(
        &["../../data"],
        "test-resources.gresource.xml",
        "strata.gresource",
    );
}
