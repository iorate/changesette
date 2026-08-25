use std::{
    fmt,
    io::{self, IsTerminal, Write},
    path::Path,
};

use anyhow::Result;
use serde::Serialize;
use tracing::{Event, Level, Subscriber, level_filters::LevelFilter};
use tracing_subscriber::{
    fmt::{
        FmtContext, FormatEvent, FormatFields,
        format::{self},
    },
    registry::LookupSpan,
};

/// Installs the global tracing subscriber that renders every event at or
/// above `max_level` to stderr through [`Formatter`].
pub(crate) fn init_subscriber(max_level: LevelFilter) {
    tracing_subscriber::fmt()
        .event_format(Formatter)
        .with_max_level(max_level)
        // The default writer is stdout, which belongs to the machine-readable
        // main output.
        .with_writer(|| LenientStderr(io::stderr()))
        .init();
}

/// Renders an event as `error: message`, `warning: message`, or
/// `debug: message` — info messages are results of the normal flow and are
/// printed bare, in the cargo / git style.
pub(crate) struct Formatter;

impl<S, N> FormatEvent<S, N> for Formatter
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: format::Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let prefix = match *event.metadata().level() {
            Level::ERROR => "error: ",
            Level::WARN => "warning: ",
            Level::INFO => "",
            _ => "debug: ",
        };
        writer.write_str(prefix)?;
        ctx.field_format().format_fields(writer.by_ref(), event)?;
        writeln!(writer)
    }
}

// Swallows broken pipes like `print_line` does: the fmt layer would
// otherwise report the failed write with an `eprintln!`, which panics once
// stderr is gone.
struct LenientStderr(io::Stderr);

impl Write for LenientStderr {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.0.write(buf) {
            Err(err) if err.kind() == io::ErrorKind::BrokenPipe => Ok(buf.len()),
            result => result,
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.0.flush() {
            Err(err) if err.kind() == io::ErrorKind::BrokenPipe => Ok(()),
            result => result,
        }
    }
}

/// Renders `path` for user-facing messages, with `/` as the separator on
/// every platform.
pub(crate) fn display_path(path: &Path) -> String {
    let text = path.display().to_string();
    if cfg!(windows) {
        text.replace('\\', "/")
    } else {
        text
    }
}

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
    let mut stdout = io::stdout().lock();
    match writeln!(stdout, "{text}").and_then(|()| stdout.flush()) {
        // A consumer that stops reading early should end the output quietly,
        // not turn the command into a panic or an error.
        Err(err) if err.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        result => Ok(result?),
    }
}
