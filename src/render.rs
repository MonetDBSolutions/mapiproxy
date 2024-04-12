use core::fmt;
use std::{
    fmt::Display,
    io::{self, BufWriter, Write},
    mem,
    time::Duration,
};

use chrono::{DateTime, Local};

use crate::event::{ConnectionId, Direction, Timestamp};

pub struct Renderer {
    colored: bool,
    timing: TrackTime,
    out: BufWriter<Box<dyn io::Write + 'static + Send>>,
    current_style: Style,
    at_start: Option<Style>, // if Some(s), we're at line start, style to be reset to s
}

impl Renderer {
    pub fn new(colored: bool, out: Box<dyn io::Write + 'static + Send>) -> Self {
        let buffered = BufWriter::with_capacity(4 * 8192, out);
        Renderer {
            colored,
            out: buffered,
            current_style: Style::Normal,
            at_start: Some(Style::Normal),
            timing: TrackTime::default(),
        }
    }

    fn show_elapsed_time(&mut self) -> io::Result<()> {
        let print_sep = self.timing.activity();
        if print_sep {
            writeln!(self.out)?;
        }
        if let Some(announcement) = self.timing.announcement() {
            let message = format!("TIME is {announcement}");
            self.message_no_check_time(None, None, &message)?;
        }
        Ok(())
    }
    pub fn set_timestamp(&mut self, timestamp: &Timestamp) {
        self.timing.set_time(timestamp);
    }

    pub fn message(
        &mut self,
        id: Option<ConnectionId>,
        direction: Option<Direction>,
        message: impl Display,
    ) -> io::Result<()> {
        self.show_elapsed_time()?;
        self.message_no_check_time(id, direction, &message)
    }

    fn message_no_check_time(
        &mut self,
        id: Option<ConnectionId>,
        direction: Option<Direction>,
        message: &dyn Display,
    ) -> Result<(), io::Error> {
        self.style(Style::Frame)?;
        writeln!(self.out, "‣{} {message}", IdStream::from((id, direction)))?;
        self.style(Style::Normal)?;
        self.out.flush()?;
        Ok(())
    }

    pub fn header(
        &mut self,
        id: ConnectionId,
        direction: Direction,
        items: &[&dyn fmt::Display],
    ) -> io::Result<()> {
        self.show_elapsed_time()?;
        let old_style = self.style(Style::Frame)?;
        write!(self.out, "┌{}", IdStream::from((id, direction)))?;
        let mut sep = " ";
        for item in items {
            write!(self.out, "{sep}{item}")?;
            sep = ", ";
        }
        writeln!(self.out)?;
        self.at_start = Some(old_style);
        assert_eq!(self.current_style, Style::Frame);
        Ok(())
    }

    pub fn footer(&mut self, items: &[&dyn fmt::Display]) -> io::Result<()> {
        self.clear_line()?;
        assert_eq!(self.current_style, Style::Frame);
        write!(self.out, "└")?;
        let mut sep = " ";
        for item in items {
            write!(self.out, "{sep}{item}")?;
            sep = ", ";
        }
        writeln!(self.out)?;
        self.style(Style::Normal)?;
        self.out.flush()?;
        Ok(())
    }

    pub fn put(&mut self, data: impl AsRef<[u8]>) -> io::Result<()> {
        if let Some(style) = self.at_start {
            assert_eq!(self.current_style, Style::Frame);
            self.out.write_all("│".as_bytes())?;
            self.style(style)?;
            self.at_start = None;
        }
        self.out.write_all(data.as_ref())?;
        Ok(())
    }

    pub fn clear_line(&mut self) -> io::Result<()> {
        if self.at_start.is_none() {
            self.nl()?;
        }
        Ok(())
    }

    pub fn nl(&mut self) -> io::Result<()> {
        let old_style = self.style(Style::Frame)?;
        writeln!(self.out)?;
        self.at_start = Some(old_style);
        Ok(())
    }

    pub fn style(&mut self, mut style: Style) -> io::Result<Style> {
        if style == self.current_style {
            return Ok(style);
        }
        if self.colored {
            self.write_style(style)?;
        }
        mem::swap(&mut self.current_style, &mut style);
        Ok(style)
    }

    fn write_style(&mut self, style: Style) -> io::Result<()> {
        // Black=30 Red=31 Green=32 Yellow=33 Blue=34 Magenta=35 Cyan=36 White=37

        let escape_sequence = match style {
            Style::Normal => "",
            Style::Header => "\u{1b}[1m",          // bold
            Style::Frame => "\u{1b}[36m",          // cyan
            Style::Error => "\u{1b}[1m\u{1b}[31m", // bold red
            Style::Whitespace => "\u{1b}[31m",     // red
            Style::Digit => "\u{1b}[32m",          // green
            Style::Letter => "\u{1b}[34m",         // blue
        };
        self.out.write_all(b"\x1b[m")?; // NORMAL
        self.out.write_all(escape_sequence.as_bytes())?;
        Ok(())
    }
}

pub struct IdStream(Option<ConnectionId>, Option<Direction>);

impl fmt::Display for IdStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(id) = self.0 {
            write!(f, " {id}")?;
        }
        if let Some(dir) = self.1 {
            write!(f, " {dir}")?;
        }
        Ok(())
    }
}

impl From<(ConnectionId, Direction)> for IdStream {
    fn from(value: (ConnectionId, Direction)) -> Self {
        let (id, dir) = value;
        IdStream(Some(id), Some(dir))
    }
}

impl From<(Option<ConnectionId>, Option<Direction>)> for IdStream {
    fn from(value: (Option<ConnectionId>, Option<Direction>)) -> Self {
        let (id, dir) = value;
        IdStream(id, dir)
    }
}

#[derive(Debug, Default)]
struct TrackTime {
    now: Option<Timestamp>,
    last_activity: Option<Timestamp>,
    last_announce: Option<Timestamp>,
}

impl TrackTime {
    const SEPARATOR_THRESHOLD: Duration = Duration::from_millis(500);
    const ANNOUNCEMENT_THRESHOLD: Duration = Duration::from_secs(60);

    fn set_time(&mut self, now: &Timestamp) {
        self.now = Some(now.clone())
    }

    fn now(&self) -> &Timestamp {
        self.now.as_ref().unwrap()
    }

    /// There has been activity, return true if a separator line must be printed.
    fn activity(&mut self) -> bool {
        let now = self.now().clone();
        let Some(prev) = self.last_activity.replace(now) else {
            return false;
        };
        self.elapsed_since(&prev) >= Self::SEPARATOR_THRESHOLD
    }

    fn must_announce(&self) -> bool {
        let Some(prev) = &self.last_announce else {
            return true;
        };
        self.elapsed_since(prev) >= Self::ANNOUNCEMENT_THRESHOLD
    }

    fn announcement(&mut self) -> Option<String> {
        if !self.must_announce() {
            return None;
        }
        let now = self.now();
        let epoch = chrono::DateTime::UNIX_EPOCH;
        let utc_now = epoch + now.0;
        let local: DateTime<Local> = DateTime::from(utc_now);
        let formatted = local.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        self.last_announce = Some(now.clone());
        Some(formatted)
    }

    // Return the time elapsed since 'time' if known and positive, [Duration::MAX] otherwise.
    fn elapsed_since(&self, time: &Timestamp) -> Duration {
        let now = self.now();
        if let Some(delta) = now.0.checked_sub(time.0) {
            return delta;
        }
        Duration::MAX
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Style {
    Normal,
    Error,
    Frame,
    Header,
    Whitespace,
    Digit,
    Letter,
}
