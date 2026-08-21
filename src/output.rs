use std::io::{self, Write};

use anyhow::Result;

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
