// SPDX-License-Identifier: MIT

mod args;
mod controller;
mod window;

use std::io::Write;

use serde_json::{Value, json};

fn main() {
    let options = match args::Options::parse(std::env::args_os().skip(1)) {
        Ok(options) => options,
        Err(why) => {
            eprintln!("shell: {why}\n\n{}", args::USAGE);
            std::process::exit(2);
        }
    };
    if options.help {
        println!("{}", args::USAGE);
        return;
    }
    std::process::exit(controller::run(options));
}

/// One JSON line on stdout.
pub fn log(event: &str, fields: Value) {
    let mut line = json!({ "ev": event });
    if let (Some(obj), Value::Object(fields)) = (line.as_object_mut(), fields) {
        obj.extend(fields);
    }
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}
