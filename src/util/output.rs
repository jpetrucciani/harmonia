use std::io::{self, Write};

use console::style;
use dialoguer::Confirm;

pub fn info(message: &str) {
    let _ = writeln!(io::stderr(), "{}", message);
}

pub fn warn(message: &str) {
    let _ = writeln!(io::stderr(), "{}", style(message).yellow());
}

pub fn error(message: &str) {
    let _ = writeln!(io::stderr(), "{}", style(message).red());
}

pub fn git_op(message: &str) {
    let _ = writeln!(io::stderr(), "{} {}", style("git").cyan(), message);
}

pub fn confirm(prompt: &str, assume_yes: bool) -> Result<bool, dialoguer::Error> {
    if assume_yes {
        return Ok(true);
    }

    Confirm::new().with_prompt(prompt).default(false).interact()
}
