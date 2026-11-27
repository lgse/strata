// SPDX-License-Identifier: MIT

//! `strata [DIRECTORY]`: Strata's Rune interface in a window.

fn main() -> std::process::ExitCode {
    strata_app::run(std::env::args_os())
}
