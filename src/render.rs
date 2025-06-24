use core::fmt;
use std::{
    borrow::Borrow,
    fmt::Display,
    io::{self, Write},
    mem,
    time::Duration,
};

use chrono::{DateTime, Local};

use crate::{
    colors::{Colors, EscapeSequence},
    event::{ConnectionId, Direction, Timestamp},
    headtail::HeadTail,
};

pub struct Renderer {
    out: HeadTail<Box<dyn io::Write + 'static + Send>>,
    colors: &'static Colors,
    abbreviate: Option<u32>,
    color_stack: Vec<&'static EscapeSequence>,
    timing: TrackTime,
    current_style: Style,
    desired_style: Style,
    at_start: bool,
}

impl Renderer {
    pub fn new(colors: &'static Colors, out: Box<dyn io::Write + 'static + Send>) -> Self {
        let headtail = HeadTail::new(out);
        Renderer {
            out: headtail,
            colors,
            abbreviate: None,
            color_stack: vec![],
            current_style: Style::Normal,
            desired_style: Style::Normal,
            at_start: true,
            timing: TrackTime::new(),
        }
    }

    pub fn set_brief(&mut self, brief: u32) {
        self.abbreviate = Some(brief);
    }

    pub fn timestamp(&mut self, timestamp: &Timestamp) {
        self.timing.set_time(timestamp);
    }

    pub fn message(
        &mut self,
        context: impl Into<Context>,
        message: impl Display,
    ) -> io::Result<()> {
        self.render_timing()?;
        self.render_message(&context.into(), &message)
    }

    fn render_timing(&mut self) -> io::Result<()> {
        let print_sep = self.timing.register_activity();
        if print_sep {
            self.nl()?;
        }
        if let Some(announcement) = self.timing.announcement() {
            self.render_message(&Context::empty(), &(format_args!("TIME is {announcement}")))?;
            self.nl()?;
        }
        Ok(())
    }

    fn render_message(
        &mut self,
        context: &Context,
        message: &dyn Display,
    ) -> Result<(), io::Error> {
        self.style(Style::Frame);
        self.switch_style()?;
        write!(self.out.format_line(), "‣{} {message}", context)?;
        self.nl()?;
        self.out.flush()?;
        Ok(())
    }

    pub fn header(
        &mut self,
        context: impl Into<Context>,
        items: &[&dyn fmt::Display],
    ) -> io::Result<()> {
        self.render_timing()?;
        self.style(Style::Frame);
        self.switch_style()?;
        let mut out = self.out.format_line();
        write!(out, "┌{}", context.into())?;
        let mut sep = " ";
        for item in items {
            write!(out, "{sep}{item}")?;
            sep = ", ";
        }
        self.nl()?;
        self.at_start = true;
        if let Some(brief) = self.abbreviate {
            self.out.head_tail(brief, brief);
        }
        Ok(())
    }

    pub fn footer(&mut self, items: &[&dyn fmt::Display]) -> io::Result<()> {
        if !self.at_start {
            self.nl()?;
        }
        if self.abbreviate.is_some() {
            let tail = self.out.finish_tail()?;
            let old = self.style(Style::Frame);
            self.switch_style()?;
            let n = tail.skipped();
            if n > 0 {
                let lines = match n {
                    1 => "line",
                    _ => "lines",
                };
                write!(self.out.format_line(), "├ (skipped {n} {lines})")?;
                self.out.nl()?;
            }
            self.out.put_tail(tail)?;
            self.style(old);
            self.switch_style()?;
        }
        self.style(Style::Frame);
        self.switch_style()?;
        let mut out = self.out.format_line();
        write!(out, "└")?;
        let mut sep = " ";
        for item in items {
            write!(out, "{sep}{item}")?;
            sep = ", ";
        }
        self.nl()?;
        self.out.flush()?;
        Ok(())
    }

    pub fn put(&mut self, data: impl AsRef<[u8]>) -> io::Result<()> {
        if self.at_start {
            let old_style = self.style(Style::Frame);
            self.switch_style()?;
            self.out.put("│".as_bytes());
            self.style(old_style);
            self.at_start = false;
        }
        self.switch_style()?;
        self.out.put(data.as_ref());
        Ok(())
    }

    pub fn nl(&mut self) -> io::Result<()> {
        let old_style = self.style(Style::Normal);
        self.switch_style()?;
        self.out.nl()?;
        self.style(old_style);
        self.at_start = true;
        Ok(())
    }

    pub fn at_start(&self) -> bool {
        self.at_start
    }

    pub fn style(&mut self, mut style: Style) -> Style {
        mem::swap(&mut self.desired_style, &mut style);
        style
    }

    fn switch_style(&mut self) -> io::Result<()> {
        let style = self.desired_style;
        if style == self.current_style {
            return Ok(());
        }

        while let Some(sequence) = self.color_stack.pop() {
            self.out.put(sequence.disable.as_bytes());
        }

        let colors = self.colors;
        match style {
            Style::Normal => self.push_style(&colors.normal)?,
            Style::Error => {
                self.push_style(&colors.red)?;
                self.push_style(&colors.bold)?;
            }
            Style::Frame => self.push_style(&colors.cyan)?,
            Style::Header => self.push_style(&colors.bold)?,
            Style::Whitespace => self.push_style(&colors.red)?,
            Style::Digit => self.push_style(&colors.green)?,
            Style::Letter => self.push_style(&colors.blue)?,
        }

        self.current_style = self.desired_style;
        Ok(())
    }

    fn push_style(&mut self, seq: &'static EscapeSequence) -> io::Result<()> {
        self.out.put(seq.enable.as_bytes());
        self.color_stack.push(seq);
        Ok(())
    }
}

pub struct Context(Option<ConnectionId>, Option<Direction>);

impl Context {
    pub fn empty() -> Self {
        (None, None).into()
    }
}

impl fmt::Display for Context {
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

impl<C: Borrow<ConnectionId>> From<C> for Context {
    fn from(value: C) -> Self {
        Context(Some(*value.borrow()), None)
    }
}

impl From<(ConnectionId, Direction)> for Context {
    fn from(value: (ConnectionId, Direction)) -> Self {
        let (id, dir) = value;
        Context(Some(id), Some(dir))
    }
}

impl From<(Option<ConnectionId>, Option<Direction>)> for Context {
    fn from(value: (Option<ConnectionId>, Option<Direction>)) -> Self {
        let (id, dir) = value;
        Context(id, dir)
    }
}

#[derive(Debug)]
struct TrackTime {
    now: Option<Timestamp>,
    last_activity: Option<Timestamp>,
    next_announcement: Timestamp,
}

impl TrackTime {
    const SEPARATOR_THRESHOLD: Duration = Duration::from_millis(500);
    const ANNOUNCEMENT_THRESHOLD: Duration = Duration::from_secs(60);

    fn new() -> Self {
        TrackTime {
            now: None,
            last_activity: None,
            next_announcement: Timestamp(Duration::ZERO),
        }
    }

    fn set_time(&mut self, now: &Timestamp) {
        self.now = Some(now.clone())
    }

    fn now(&self) -> &Timestamp {
        self.now.as_ref().unwrap()
    }

    /// There has been activity, return true if a separator line must be printed.
    fn register_activity(&mut self) -> bool {
        let now = self.now().clone();
        let Some(prev) = self.last_activity.replace(now) else {
            return false;
        };
        self.elapsed_since(&prev) >= Self::SEPARATOR_THRESHOLD
    }

    fn announcement(&mut self) -> Option<String> {
        if self.now() < &self.next_announcement {
            return None;
        }

        // decide the next time
        let units = self.now().0.as_secs_f64() / Self::ANNOUNCEMENT_THRESHOLD.as_secs_f64();
        let mut ceil = units.ceil();
        // we need strictly greater, not greater or equal
        if ceil == units {
            ceil += 1.0;
        }
        let ceil_seconds = ceil * Self::ANNOUNCEMENT_THRESHOLD.as_secs_f64();
        self.next_announcement = Timestamp(Duration::from_secs_f64(ceil_seconds));

        // format the timestamp
        let now = self.now();
        let epoch = chrono::DateTime::UNIX_EPOCH;
        let utc_now = epoch + now.0;
        let local_now: DateTime<Local> = DateTime::from(utc_now);
        let formatted = local_now.format("%Y-%m-%d %H:%M:%S%.3f").to_string();
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
