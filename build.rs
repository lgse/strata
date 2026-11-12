// SPDX-License-Identifier: MIT

fn main() {
    glib_build_tools::compile_resources(&["data"], "data/strata.gresource.xml", "strata.gresource");
}
