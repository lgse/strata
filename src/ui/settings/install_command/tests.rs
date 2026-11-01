// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn install_commands_run_as_arguments_and_wait_before_closing() {
    let arguments = terminal_install_arguments("printf %s|%s;\t$HOME");
    let output = std::process::Command::new(&arguments[0])
        .args(&arguments[1..])
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run the install script");
    let stdout = String::from_utf8(output.stdout).expect("UTF-8 output");
    assert_eq!(
        stdout,
        format!("$HOME|;\n{} ", crate::i18n::tr("Press Enter to close."))
    );
}
