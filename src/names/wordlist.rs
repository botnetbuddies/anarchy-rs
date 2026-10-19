//! Contiguous dictionary storage and Ruby line-ending behavior.

use crate::util;

/// A wordlist loaded the way the original loads it (`readlines` then `strip!`).
///
/// Items live in one contiguous buffer with `(offset, len)` ranges, the same
/// trick the output sink uses. This removes one allocation per word (tens of
/// thousands for the Facebook dictionaries) and keeps enumeration scans
/// cache-friendly.
pub struct WordList {
    bytes: Vec<u8>,
    ranges: Vec<(u32, u32)>,
    /// True when the final line had no trailing newline *and* nothing to strip,
    /// so Ruby's `strip!` would have returned `nil` and the original would raise
    /// when the list is used. We surface that as an error at use time.
    pub last_nil: bool,
}

impl WordList {
    #[inline]
    pub fn len(&self) -> usize {
        self.ranges.len()
    }

    #[inline]
    pub fn get(&self, i: usize) -> &[u8] {
        let (a, b) = self.ranges[i];
        &self.bytes[a as usize..b as usize]
    }
}

pub fn load_wordlist(data: &[u8]) -> WordList {
    let mut bytes: Vec<u8> = Vec::with_capacity(data.len());
    let mut ranges: Vec<(u32, u32)> = Vec::new();
    let mut last_nil = false;
    let lines = readlines(data);
    let n = lines.len();
    for (i, line) in lines.iter().enumerate() {
        let stripped = util::strip(line);
        if stripped.len() == line.len() && i + 1 == n && !line.ends_with(b"\n") {
            // Ruby `String#strip!` returns nil here.
            last_nil = true;
        } else {
            let start = bytes.len();
            bytes.extend_from_slice(stripped);
            ranges.push((start as u32, bytes.len() as u32));
        }
    }
    WordList {
        bytes,
        ranges,
        last_nil,
    }
}

/// `given_name_list`: `readlines.map { |x| x.downcase!; x.strip! }`.
pub fn load_given_list(data: &[u8]) -> WordList {
    let raw = load_wordlist(data);
    let mut bytes: Vec<u8> = Vec::with_capacity(raw.bytes.len());
    let mut ranges: Vec<(u32, u32)> = Vec::with_capacity(raw.ranges.len());
    for i in 0..raw.len() {
        let start = bytes.len();
        util::downcase_into(raw.get(i), &mut bytes);
        ranges.push((start as u32, bytes.len() as u32));
    }
    WordList {
        bytes,
        ranges,
        last_nil: raw.last_nil,
    }
}

/// Ruby `File.readlines` — split on `\n`, keeping the terminator.
fn readlines(data: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut start = 0usize;
    for i in 0..data.len() {
        if data[i] == b'\n' {
            out.push(&data[start..=i]);
            start = i + 1;
        }
    }
    if start < data.len() {
        out.push(&data[start..]);
    }
    out
}
