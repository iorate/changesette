use std::io::{self, IsTerminal, Write};

use anyhow::Result;
use serde::Serialize;

/// Prints `value` as JSON and a newline to stdout — pretty-printed when
/// stdout is a terminal, single-line otherwise; a broken pipe counts as
/// success.
pub(crate) fn print_json(value: &impl Serialize) -> Result<()> {
    let json = if io::stdout().is_terminal() {
        serde_json::to_string_pretty(value)?
    } else {
        serde_json::to_string(value)?
    };
    print_line(&json)
}

/// Prints `text` and a newline to stdout, flushing immediately; a broken
/// pipe counts as success.
pub(crate) fn print_line(text: &str) -> Result<()> {
    write_line(io::stdout().lock(), text)
}

/// Prints `text` and a newline to stderr, flushing immediately; a broken
/// pipe counts as success.
pub(crate) fn eprint_line(text: &str) -> Result<()> {
    write_line(io::stderr().lock(), text)
}

fn write_line(mut writer: impl Write, text: &str) -> Result<()> {
    match writeln!(writer, "{text}").and_then(|()| writer.flush()) {
        // A consumer that stops reading early should end the output quietly,
        // not turn the command into a panic or an error.
        Err(err) if err.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        result => Ok(result?),
    }
}
