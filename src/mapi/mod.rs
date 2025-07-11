mod abbreviate;
mod analyzer;

use std::{
    collections::HashMap,
    io::{self, ErrorKind},
};

use crate::{
    event::{ConnectionId, Direction, MapiEvent, Timestamp},
    render::{Renderer, Style},
    Level,
};

use self::analyzer::Analyzer;

const BINARY_BYTES_PER_LINE: usize = 16;

#[derive(Debug)]
pub struct State {
    level: Level,
    force_binary: bool,
    accs: HashMap<ConnectionId, (Accumulator, Accumulator)>,
}

impl State {
    pub fn new(level: Level, force_binary: bool) -> Self {
        State {
            level,
            force_binary,
            accs: Default::default(),
        }
    }

    pub fn handle(
        &mut self,
        timestamp: &Timestamp,
        event: &MapiEvent,
        renderer: &mut Renderer,
    ) -> io::Result<()> {
        renderer.timestamp(timestamp);
        match event {
            MapiEvent::BoundPort(port) => {
                renderer.message((None, None), format_args!("LISTEN on port {port}"))?;
            }

            MapiEvent::Incoming { id, local, peer } => {
                renderer.message(id, format_args!("INCOMING on {local} from {peer}"))?;
                self.add_connection(id, peer.is_unix());
            }

            MapiEvent::Connecting { id, remote } => {
                renderer.message(id, format_args!("CONNECTING to {remote}"))?;
            }

            MapiEvent::Connected { id } => {
                renderer.message(id, "CONNECTED")?;
            }

            MapiEvent::ConnectFailed {
                id,
                remote,
                immediately,
                error,
            } => {
                let immediately = if *immediately { " immediately" } else { "" };
                renderer.message(
                    id,
                    format_args!("CONNECT FAILED{immediately}: {remote}: {error}"),
                )?;
            }

            MapiEvent::End { id } => {
                renderer.message(id, "ENDED")?;
                self.remove_connection(id);
            }

            MapiEvent::Aborted { id, error } => {
                renderer.message(id, format_args!("ABORTED: {error}"))?;
                self.remove_connection(id);
            }

            MapiEvent::Data {
                id,
                direction,
                data,
            } => {
                let Some((upstream, downstream)) = self.accs.get_mut(id) else {
                    panic!("got data for conn {id} but don't have accumulators for it")
                };
                let acc = match direction {
                    Direction::Upstream => upstream,
                    Direction::Downstream => downstream,
                };
                acc.handle_data(data, renderer)?;
            }

            MapiEvent::ShutdownRead { id, direction } => {
                self.check_incomplete(*id, *direction, renderer)?;
                let sender = direction.sender();
                renderer.message((*id, *direction), format_args!("{sender} stopped sending"))?;
            }

            MapiEvent::ShutdownWrite {
                id,
                direction,
                discard: n,
            } => {
                let receiver = direction.receiver();
                renderer.message(
                    (*id, *direction),
                    format_args!("{receiver} has stopped receiving data, discarding {n} bytes"),
                )?;
            }

            MapiEvent::Oob(id, direction, byte) => {
                let sender = direction.sender();
                renderer.message(
                    (*id, *direction),
                    format_args!("{sender}  sent an Out-Of-Band message: {byte}"),
                )?;
            }
        }

        Ok(())
    }

    fn add_connection(&mut self, id: &ConnectionId, unix_client: bool) {
        let level = self.level;
        let upstream = Accumulator::new(
            *id,
            Direction::Upstream,
            level,
            self.force_binary,
            unix_client,
        );
        let downstream =
            Accumulator::new(*id, Direction::Downstream, level, self.force_binary, false);
        let new = (upstream, downstream);
        let prev = self.accs.insert(*id, new);
        if prev.is_some() {
            panic!("Already have state for incoming connection {id}");
        }
    }

    fn remove_connection(&mut self, id: &ConnectionId) {
        let ended = self.accs.remove(id);
        if ended.is_none() {
            panic!("Found no state to remove for end event on connection {id}");
        }
    }

    fn check_incomplete(
        &mut self,
        id: ConnectionId,
        direction: Direction,
        renderer: &mut Renderer,
    ) -> io::Result<()> {
        let Some((upstream, downstream)) = self.accs.get_mut(&id) else {
            panic!("got data for conn {id} but don't have accumulators for it")
        };
        let acc = match direction {
            Direction::Upstream => upstream,
            Direction::Downstream => downstream,
        };
        if let Err(e) = acc.check_incomplete() {
            renderer.message((id, direction), e)?;
        };
        Ok(())
    }
}

#[derive(Debug)]
pub struct Accumulator {
    id: ConnectionId,
    direction: Direction,
    level: Level,
    force_binary: bool,
    analyzer: Analyzer,
    binary: Binary,
    buf: Vec<u8>,
    error_reported: bool,
}

impl Accumulator {
    fn new(
        id: ConnectionId,
        direction: Direction,
        level: Level,
        force_binary: bool,
        unix_client: bool,
    ) -> Self {
        Accumulator {
            id,
            direction,
            level,
            force_binary,
            analyzer: Analyzer::new(unix_client),
            binary: Binary::new(),
            buf: Vec::with_capacity(8192),
            error_reported: false,
        }
    }

    fn handle_data(&mut self, data: &[u8], renderer: &mut Renderer) -> io::Result<()> {
        match self.level {
            Level::Raw => self.handle_raw(renderer, data),
            Level::Blocks | Level::Messages => self.handle_frame(renderer, data),
        }
    }

    fn handle_raw(&mut self, renderer: &mut Renderer, mut data: &[u8]) -> Result<(), io::Error> {
        renderer.header(
            (self.id, self.direction),
            &[&format_args!("{n} bytes", n = data.len())],
        )?;
        let mut n = 0;
        let mut error_at = None;
        while let Some(head) = self.analyzer.split_chunk(&mut data) {
            let style = if self.analyzer.was_head() {
                Style::Header
            } else if self.analyzer.was_error() {
                if !self.error_reported {
                    error_at = Some(n);
                    self.error_reported = true;
                }
                Style::Error
            } else {
                Style::Normal
            };
            n += head.len();
            for b in head {
                self.binary.add(*b, style, renderer)?;
            }
        }
        self.binary.finish(renderer)?;
        if let Some(pos) = error_at {
            renderer.footer(&[&format!(
                "encountered mapi protocol error at byte {pos}/{n}"
            )])?;
        } else {
            renderer.footer(&[])?;
        }
        Ok(())
    }

    fn handle_frame(&mut self, renderer: &mut Renderer, mut data: &[u8]) -> Result<(), io::Error> {
        loop {
            let whole = data;
            let Some(chunk) = self.analyzer.split_chunk(&mut data) else {
                break;
            };

            if self.analyzer.was_error() {
                if !self.buf.is_empty() {
                    let kind = if self.level == Level::Messages {
                        "incomplete message before error"
                    } else {
                        "incomplete block before error"
                    };
                    renderer.header((self.id, self.direction), &[&kind])?;
                    self.dump_frame_as_binary(&self.buf, renderer)?;
                    renderer.footer(&[])?;
                    self.buf.clear();
                    self.level = Level::Raw;
                }
                renderer.message((self.id, self.direction), "mapi protocol error")?;
                self.error_reported = true;
                self.level = Level::Raw;
                return self.handle_raw(renderer, whole);
            }
            if !self.analyzer.was_body() {
                continue;
            }

            let at_end = match self.level {
                Level::Blocks => self.analyzer.was_block_boundary(),
                Level::Messages => self.analyzer.was_message_boundary(),
                Level::Raw => unreachable!(),
            };

            if !at_end {
                self.buf.extend_from_slice(chunk);
                continue;
            }

            // we have a complete frame, dump it
            let frame = if self.buf.is_empty() {
                Some(chunk)
            } else {
                self.buf.extend_from_slice(chunk);
                None
            };
            self.dump_frame(frame, renderer)?;
            self.buf.clear();
        }
        Ok(())
    }

    fn dump_frame(&mut self, data: Option<&[u8]>, renderer: &mut Renderer) -> io::Result<()> {
        let data = data.unwrap_or(&self.buf);
        let len = data.len();
        let is_binary =
            self.force_binary || self.is_scary(data) || std::str::from_utf8(data).is_err();

        let format = if is_binary { "binary" } else { "text" };
        let kind = if self.level == Level::Messages {
            "message"
        } else {
            "block"
        };
        renderer.header(
            (self.id, self.direction),
            &[&format, &kind, &format_args!("{len} bytes")],
        )?;

        if is_binary {
            self.dump_frame_as_binary(data, renderer)?;
        } else {
            self.dump_frame_as_text(data, renderer)?;
        }

        renderer.footer(&[])?;
        Ok(())
    }

    fn check_incomplete(&mut self) -> io::Result<()> {
        if let Err(situation) = self.analyzer.check_incomplete() {
            let side = self.direction.sender();
            let message = format!("{side} closed the connection {situation}");
            let kind = ErrorKind::UnexpectedEof;
            return Err(io::Error::new(kind, message));
        }
        Ok(())
    }

    fn dump_frame_as_binary(&self, data: &[u8], renderer: &mut Renderer) -> io::Result<()> {
        let mut bin = Binary::new();
        for b in data {
            bin.add(*b, Style::Normal, renderer)?;
        }
        bin.finish(renderer)?;
        Ok(())
    }

    fn dump_frame_as_text(&self, data: &[u8], renderer: &mut Renderer) -> io::Result<()> {
        renderer.style(Style::Normal);
        for byte in data {
            match *byte {
                b'\n' => {
                    renderer.put("↵")?;
                    renderer.nl()?;
                }
                b'\t' => {
                    renderer.put("→")?;
                }
                b => renderer.put([b])?,
            }
        }
        if !renderer.at_start() {
            renderer.nl()?;
        }
        Ok(())
    }

    fn is_scary(&self, data: &[u8]) -> bool {
        for &b in data {
            if b < b' ' && b != b'\n' && b != b'\t' {
                return true;
            }
        }
        false
    }
}

#[derive(Debug)]
struct Binary {
    row: [(u8, Style); BINARY_BYTES_PER_LINE],
    col: usize,
}

impl Binary {
    fn new() -> Self {
        Binary {
            row: [(0, Style::Normal); BINARY_BYTES_PER_LINE],
            col: 0,
        }
    }

    fn add(&mut self, byte: u8, mut style: Style, renderer: &mut Renderer) -> io::Result<()> {
        if style == Style::Normal {
            style = match byte {
                b'0'..=b'9' => Style::Digit,
                b'a'..=b'z' | b'A'..=b'Z' => Style::Letter,
                b' ' | b'\t' | b'\r' | b'\n' => Style::Whitespace,
                _ => style,
            }
        }

        self.row[self.col] = (byte, style);
        self.col += 1;

        if self.col == BINARY_BYTES_PER_LINE {
            self.write_out(renderer, false)
        } else {
            Ok(())
        }
    }

    fn finish(&mut self, renderer: &mut Renderer) -> io::Result<()> {
        if self.col == 0 {
            return Ok(());
        }
        self.write_out(renderer, true)
    }

    fn write_out(&mut self, renderer: &mut Renderer, _keep_head_state: bool) -> io::Result<()> {
        const HEX_DIGITS: [u8; 16] = *b"0123456789abcdef";
        let mut cur_head = false;
        for (i, (byte, style)) in self.row[..self.col].iter().cloned().enumerate() {
            renderer.style(Style::Normal);
            self.put_sep(i, &mut cur_head, style, renderer)?;

            let hi = HEX_DIGITS[byte as usize / 16];
            let lo = HEX_DIGITS[byte as usize & 0xF];

            renderer.style(style);
            renderer.put([hi, lo])?;
        }
        renderer.style(Style::Normal);

        for i in self.col..BINARY_BYTES_PER_LINE {
            self.put_sep(i, &mut cur_head, Style::Frame, renderer)?;
            renderer.put(b"__")?;
        }

        // if the sep includes a style change, this is its
        // chance to wrap it up
        self.put_sep(
            BINARY_BYTES_PER_LINE,
            &mut cur_head,
            Style::Normal,
            renderer,
        )?;

        for (byte, style) in &self.row[..self.col] {
            renderer.style(*style);
            renderer.put(Self::readable(&[*byte]))?;
        }

        renderer.nl()?;

        self.col = 0;
        Ok(())
    }

    fn put_sep(
        &self,
        i: usize,
        in_head: &mut bool,
        style: Style,
        renderer: &mut Renderer,
    ) -> Result<(), io::Error> {
        let extra_space: [u8; BINARY_BYTES_PER_LINE + 1] = [
            0, 0, 0, 0, //
            1, 0, 0, 0, //
            2, 0, 0, 0, //
            1, 0, 0, 0, //
            4,
        ];
        let spaces = "          ";
        let extra = extra_space[i] as usize;
        let (open, close) = ("⟨", "⟩");
        let is_head = style == Style::Header;
        // let (open, close) = ("«", "»");
        match (*in_head, is_head) {
            (false, true) => {
                renderer.put(&spaces[..extra])?;
                let old_style = renderer.style(Style::Header);
                renderer.put(open)?;
                renderer.style(old_style);
            }
            (true, false) => {
                let old_style = renderer.style(Style::Header);
                renderer.put(close)?;
                renderer.style(old_style);
                renderer.put(&spaces[..extra])?;
            }
            _ => renderer.put(&spaces[..extra + 1])?,
        }
        *in_head = is_head;
        Ok(())
    }

    fn readable(byte: &[u8; 1]) -> &[u8] {
        // note that the readable range does not include 0x7f (DEL)
        let s = match byte[0] {
            b' ' => "·",
            0x21..=0x7e => return byte.as_ref(),
            b'\n' => "↵",
            b'\t' => "→",
            0 => "░",
            _ => "▒",
        };
        s.as_bytes()
    }
}
