use std::{io, mem};

use bstr::ByteSlice;

use State::*;

pub struct HeadTail<W> {
    out: W,
    buf: Vec<u8>,
    state: State,
    aux1: Vec<u8>,
    aux2: Vec<usize>,
}

enum State {
    /// Everything in the buffer will eventually be written.
    Passthrough,

    /// Like [`Passthrough`] but [`lines_left`] is decremented after each line.
    /// If it reaches 0 we switch to tail mode.
    Head { lines_left: u64, ntail: u32 },

    /// State for tail mode is captured in a separate struct [`TailState`]
    Tail(TailState),
}

#[derive(Default)]
struct TailState {
    /// We set the head aside while processing the tail.
    /// Eventually we'll append the tail lines to this buffer
    /// and restore it as the buffer.
    head: Vec<u8>,

    /// Start of the lines that currently form the tail.
    /// The oldest (earliest) line is at `lines[oldest_idx]`.
    /// All data before that position will be discarded.
    lines: Vec<usize>,

    /// Index in [`lines`] of the oldest still relevant line.
    oldest_idx: usize,

    /// Number of newlines seen
    newlines: u64,

    /// Number of tail lines to print
    ntail: u64,
}

impl TailState {
    #[cfg(debug_assertions)]
    fn nlines(&self) -> usize {
        self.lines.len()
    }

    fn line(&self, i: usize) -> usize {
        self.lines[(self.oldest_idx + i) % self.lines.len()]
    }

    fn oldest_line(&self) -> usize {
        self.line(0)
    }

    fn newest_line(&self) -> usize {
        self.line(self.lines.len() - 1)
    }

    #[cfg(debug_assertions)]
    fn lines(&self) -> impl Iterator<Item = usize> + '_ {
        let start = self.oldest_idx;
        let part1 = &self.lines[start..];
        let part2 = &self.lines[..start];
        part1.iter().chain(part2).cloned()
    }

    fn newline(&mut self, pos: usize) {
        debug_assert!(pos >= self.newest_line());
        // the oldest line is discarded and the next-oldest becomes the oldest
        self.lines[self.oldest_idx] = pos;
        self.oldest_idx = (self.oldest_idx + 1) % self.lines.len();
        self.newlines += 1;
    }
}

impl<W> HeadTail<W> {
    pub fn new(out: W) -> Self {
        Self::with_capacity(out, 4 * 8192)
    }

    pub fn with_capacity(out: W, cap: usize) -> Self {
        let ht = HeadTail {
            out,
            buf: Vec::with_capacity(cap),
            state: State::Passthrough,
            aux1: Vec::with_capacity(cap),
            aux2: vec![],
        };
        ht.sanity_check();
        ht
    }

    pub fn sanity_check(&self) {
        #[cfg(debug_assertions)]
        {
            use claim::{assert_le, assert_lt, assert_gt};
            use itertools::Itertools;

            match self.state {
                Passthrough => {}
                Head { lines_left, .. } => assert_ne!(lines_left, 0),

                Tail(ref ts) => {
                    assert_gt!(ts.lines.len(), 0);
                    assert_lt!(ts.oldest_idx, ts.lines.len());
                    assert!(
                        ts.lines().is_sorted(),
                        "tail: {:?}",
                        ts.lines().collect_vec()
                    );
                    let newest_line = ts.line(ts.nlines() - 1);
                    assert_le!(newest_line, self.buf.len());
                }
            }
        }
    }
}

impl<W: io::Write> HeadTail<W> {
    pub fn flush(&mut self) -> io::Result<()> {
        self.sanity_check();

        // Locate the buffer currently holding the head
        let buf = match &mut self.state {
            Passthrough | Head { .. } => &mut self.buf,
            Tail(tail_state) => &mut tail_state.head,
        };
        if !buf.is_empty() {
            self.out.write_all(buf)?;
            buf.clear();
        }

        self.sanity_check();
        Ok(())
    }

    pub fn compact(&mut self) {
        self.sanity_check();

        let Tail(tail_state) = &mut self.state else {
            return;
        };

        let tail_start = tail_state.oldest_line();
        if tail_start == 0 {
            return;
        }

        // remove the gap from the data
        let _ = self.buf.drain(0..tail_start);
        // tail lines have moved, adjust bookkeeping
        for line_start in &mut tail_state.lines {
            *line_start -= tail_start;
        }

        self.sanity_check();
    }

    fn make_room(&mut self) -> io::Result<()> {
        let capacity = self.buf.capacity();
        let used = self.buf.len();
        let free = capacity - used;

        if free > capacity / 8 {
            // plenty of room left, do nothing
            return Ok(());
        }

        let Tail(tail_state) = &self.state else {
            // in head mode we can simply flush the buffer when it runs full.
            return self.flush();
        };

        // Usually the tail should be small relative to the capacity so if we
        // run out of capacity we can make room by compacting the buffer.
        // However, if the user tries to hold many lines or if the lines are
        // long, the tail may actually need most of the capacity. Compacting
        // would mean a large memmove for little gain so it's better to not
        // compact and allow the buffer to grow to a larger capacity.
        let tail = used - tail_state.oldest_line();
        if tail <= capacity / 4 {
            self.compact();
        }

        Ok(())
    }
}

impl<W: io::Write> HeadTail<W> {
    pub fn put(&mut self, data: &[u8]) {
        self.sanity_check();
        debug_assert!(data.find_byte(b'\n').is_none());
        // we can always just append to the buffer.
        self.buf.extend_from_slice(data);
        self.sanity_check();
    }

    pub fn format_line(&mut self) -> FormatLine<'_, W> {
        FormatLine(self)
    }

    pub fn nl(&mut self) -> io::Result<()> {
        self.buf.push(b'\n');

        // Ordered by likelyhood. This involved splitting Head{} into multiple
        // cases.
        match &mut self.state {
            Passthrough => {}

            Head {
                lines_left: n @ 2..,
                ..
            } => *n -= 1,

            Tail(tail_state) => tail_state.newline(self.buf.len()),

            Head {
                lines_left: 1,
                ntail,
            } => {
                let ntail = *ntail;
                self.switch_to_tail_mode(ntail);
            }

            Head { lines_left: 0, .. } => unreachable!(),
        }

        // Maybe flush or compact
        self.make_room()?;

        self.sanity_check();

        Ok(())
    }

    fn switch_to_tail_mode(&mut self, ntail: u32) {
        let head = mem::take(&mut self.buf);

        let mut tailbuf = mem::take(&mut self.aux1);
        tailbuf.clear();

        let mut lines = mem::take(&mut self.aux2);
        lines.clear();
        lines.resize(ntail as usize + 1, 0);

        self.buf = tailbuf;
        self.state = Tail(TailState {
            head,
            lines,
            oldest_idx: 0,
            newlines: 0,
            ntail: ntail as u64,
        })
    }

    pub fn head_tail(&mut self, nhead: u32, ntail: u32) {
        self.sanity_check();
        let Passthrough = &self.state else {
            panic!("already in head-tail mode");
        };

        if nhead > 0 {
            self.state = Head {
                lines_left: nhead as u64,
                ntail,
            };
        } else {
            self.switch_to_tail_mode(ntail);
        }

        self.sanity_check();
    }

    pub fn finish_tail(&mut self) -> io::Result<Tail> {
        self.sanity_check();

        // Switch back to Passthrough mode, remember everything worth remembering
        let tail_state = match &mut self.state {
            Passthrough => panic!("can only call finish_tail() in tail mode"),
            Head { .. } => {
                // never reached the tail
                self.state = Passthrough;
                let mut buf = mem::take(&mut self.aux1);
                buf.clear();
                return Ok(Tail {
                    buf,
                    start: 0,
                    skipped: 0,
                });
            }
            Tail(ts) => ts,
        };

        let tail = mem::take(&mut self.buf);
        let tail_start = tail_state.oldest_line();
        let linesbuf = mem::take(&mut tail_state.lines);
        let skipped = tail_state.newlines.saturating_sub(tail_state.ntail);
        // Recover the state from before we switched to tail mode
        self.buf = mem::take(&mut tail_state.head);
        self.aux2 = linesbuf;
        self.state = Passthrough;

        Ok(Tail {
            buf: tail,
            start: tail_start,
            skipped,
        })
    }

    pub fn put_tail(&mut self, mut tail: Tail) -> io::Result<()> {
        let data: &[u8] = tail.as_ref();

        // First flush if it would require a reallocation.
        let free = self.buf.capacity() - self.buf.len();
        if data.len() > free {
            self.flush()?;
        }
        if tail.start == 0 && self.buf.is_empty() {
            // Avoid a memcopy
            mem::swap(&mut self.buf, &mut tail.buf);
        } else {
            self.buf.extend_from_slice(data);
        }

        // We've appended, deal with that
        self.make_room()?;

        // Keep vecs around for next time
        tail.buf.clear();
        self.aux1 = tail.buf;

        Ok(())
    }
}

pub struct Tail {
    buf: Vec<u8>,
    start: usize,
    skipped: u64,
}

impl AsRef<[u8]> for Tail {
    fn as_ref(&self) -> &[u8] {
        &self.buf[self.start..]
    }
}

impl Tail {
    pub fn skipped(&self) -> u64 {
        self.skipped
    }
}

pub struct FormatLine<'a, W>(&'a mut HeadTail<W>);

impl<'a, W: io::Write> io::Write for FormatLine<'a, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.put(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}
