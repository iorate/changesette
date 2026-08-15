use std::io::{self, Write};

use anyhow::Result;

/// Prints `text` and a newline to stdout, flushing immediately. A broken pipe
/// counts as success, so a consumer that stops reading early ends the output
/// quietly instead of turning the command into a panic or an error.
pub(crate) fn print_line(text: &str) -> Result<()> {
    let mut stdout = io::stdout().lock();
    match writeln!(stdout, "{text}").and_then(|()| stdout.flush()) {
        Err(err) if err.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        result => Ok(result?),
    }
}
