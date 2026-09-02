use std::{
    fmt,
    io::{self, IsTerminal, Write},
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

pub(crate) fn init_subscriber(max_level: LevelFilter) {
    tracing_subscriber::fmt()
        .event_format(Formatter)
        .with_max_level(max_level)
        // The default writer is stdout, which belongs to the machine-readable
        // main output.
        .with_writer(|| LenientStderr(io::stderr()))
        .init();
}

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

pub(crate) fn print_json(value: &impl Serialize) -> Result<()> {
    let json = if io::stdout().is_terminal() {
        serde_json::to_string_pretty(value)?
    } else {
        serde_json::to_string(value)?
    };
    print_line(&json)
}

pub(crate) fn print_line(text: &str) -> Result<()> {
    let mut stdout = io::stdout().lock();
    match writeln!(stdout, "{text}").and_then(|()| stdout.flush()) {
        // A consumer that stops reading early should end the output quietly,
        // not turn the command into a panic or an error.
        Err(err) if err.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        result => Ok(result?),
    }
}
