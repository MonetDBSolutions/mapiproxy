use memchr::{memchr, memrchr};

use super::BINARY_BYTES_PER_LINE;

#[derive(Debug, PartialEq, Eq)]
pub struct Abbreviated<'a> {
    pub first: &'a [u8],
    pub skipped: usize,
    pub last: &'a [u8],
}

impl<'a> Abbreviated<'a> {
    fn new(data: &'a [u8], head: usize, tail: usize) -> Abbreviated<'a> {
        if head + tail >= data.len() {
            Abbreviated {
                first: data,
                skipped: 0,
                last: &[],
            }
        } else {
            Abbreviated {
                first: &data[..head],
                skipped: data.len() - (head + tail),
                last: &data[data.len() - tail..],
            }
        }
    }
}

#[allow(dead_code)]
pub fn abbreviate(is_binary: bool, first: usize, last: usize, data: &[u8]) -> Abbreviated {
    if is_binary {
        abbreviate_bin(first, last, data)
    } else {
        abbreviate_text(first, last, data)
    }
}

pub fn abbreviate_bin(first: usize, last: usize, data: &[u8]) -> Abbreviated {
    let len = data.len();

    // Compute head = the number of bytes we want to keep at the
    // start, and tail = the number of bytes we want to keep at the end.
    // It's ok if these overlap or are longer than the total number of bytes,
    // Abbreviated::new will take care of that.

    let head = first * BINARY_BYTES_PER_LINE;

    // We must keep the start of the tail aligned with BINARY_BYTES_PER_LINE.
    // To do so we subtract the number of missing bytes on the last line.
    let missing = len.wrapping_neg() % BINARY_BYTES_PER_LINE;
    let tail = (last * BINARY_BYTES_PER_LINE) - missing;

    Abbreviated::new(data, head, tail)
}

#[test]
fn test_abbreviate_bin() {
    use std::ops::Range;

    #[track_caller]
    fn check(data: &[u8], expected_gap: Range<usize>) {
        let abb = abbreviate_bin(2, 2, data);

        assert!(
            data.starts_with(abb.first),
            "expect data to start with {:?}",
            abb.first
        );
        assert!(
            data.ends_with(abb.last),
            "expect data to end with {:?}",
            abb.last
        );
        let gap_start = abb.first.len();
        let gap_end = data.len() - abb.last.len();
        let gap = gap_start..gap_end;
        if expected_gap.is_empty() {
            assert!(gap.is_empty(), "expected gap to be empty, found {gap:?}");
        } else {
            assert_eq!(gap_start..gap_end, expected_gap);
        }
    }

    check(&[], 0..0);
    check(b"aaaaAAAAaaaaAA", 0..0);
    check(
        b"aaaaAAAAaaaaAAAAbbbbBBBBbbbbBBBBccccCCCCccccCCCCddddDDDDddddDD",
        0..0,
    );
    check(
        b"aaaaAAAAaaaaAAAAbbbbBBBBbbbbBBBBccccCCCCccccCCCCddddDDDDddddDDDD",
        0..0,
    );

    // should take a,b, d and e, skipping c
    check(
        b"aaaaAAAAaaaaAAAAbbbbBBBBbbbbBBBBccccCCCCccccCCCCddddDDDDddddDDDDe",
        32..48,
    );
    check(
        b"aaaaAAAAaaaaAAAAbbbbBBBBbbbbBBBBccccCCCCccccCCCCddddDDDDddddDDDDeeeeEEEEeeeeEEE",
        32..48,
    );
}

pub fn abbreviate_text(first: usize, last: usize, data: &[u8]) -> Abbreviated {
    let head = first_n_lines(first, data);
    let tail = last_n_lines(last, data);
    Abbreviated::new(data, head, tail)
}

fn first_n_lines(n: usize, data: &[u8]) -> usize {
    let mut pos = 0;

    for _ in 0..n {
        // advance pos to skip 1 line
        match memchr(b'\n', &data[pos..]) {
            None => {
                pos = data.len();
                break;
            }
            Some(len) => pos += len + 1,
        }
    }

    pos
}

fn last_n_lines(n: usize, data: &[u8]) -> usize {
    if n == 0 || data.is_empty() {
        return 0;
    }

    let mut pos = data.len();

    for _ in 0..n {
        if pos == 0 {
            break;
        }
        if data[pos - 1] == b'\n' {
            pos -= 1;
        }
        match memrchr(b'\n', &data[..pos]) {
            None => {
                pos = 0;
            }
            Some(i) => {
                pos = i + 1; // leave newline intact
            }
        }
    }

    data.len() - pos
}

#[test]
fn test_abbreviate_text() {
    #[track_caller]
    fn check(data: &str, expected_first: &str, expected_last: &str) {
        let abb = abbreviate_text(2, 2, data.as_bytes());
        let first = str::from_utf8(abb.first).expect("'first' must be utf8");
        let last = str::from_utf8(abb.last).expect("'last' must be utf8");

        assert_eq!(first, expected_first, "'first' mismatch");
        assert_eq!(last, expected_last, "'last' mismatch");
        assert_eq!(abb.skipped, data.len() - first.len() - last.len());
    }

    check("", "", "");
    check("\n", "\n", "");
    check("a\nb\nc\nd", "a\nb\nc\nd", "");
    check("a\nb\nc\nd\n", "a\nb\nc\nd\n", "");

    check("a\nb\nc\nd\ne", "a\nb\n", "d\ne");
    check("a\nb\nc\nd\ne\n", "a\nb\n", "d\ne\n");
}
