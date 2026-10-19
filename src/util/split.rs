//! Ruby-compatible whitespace and field splitting.

/// Whitespace used by Ruby's `String#split` (awk mode) and `String#strip`.
const SPLIT_WS: [bool; 256] = {
    let mut t = [false; 256];
    t[b' ' as usize] = true;
    t[b'\t' as usize] = true;
    t[b'\n' as usize] = true;
    t[0x0b] = true;
    t[0x0c] = true;
    t[b'\r' as usize] = true;
    t
};

/// Ruby's `String#strip` also removes NUL bytes.
const STRIP_WS: [bool; 256] = {
    let mut t = SPLIT_WS;
    t[0] = true;
    t
};

#[inline(always)]
pub fn is_split_ws(b: u8) -> bool {
    SPLIT_WS[b as usize]
}

#[inline(always)]
pub fn is_strip_ws(b: u8) -> bool {
    STRIP_WS[b as usize]
}

/// Ruby `String#strip` (non-mutating view).
pub fn strip(s: &[u8]) -> &[u8] {
    let mut a = 0usize;
    let mut b = s.len();
    while a < b && is_strip_ws(s[a]) {
        a += 1;
    }
    while b > a && is_strip_ws(s[b - 1]) {
        b -= 1;
    }
    &s[a..b]
}

/// Delimiter used when splitting an input file.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Delim {
    /// `String#split(" ")` — awk-style whitespace mode.
    Space,
    /// `String#split(",")` / `String#split("\t")` — literal separator.
    Byte(u8),
}

/// Ruby `String#split` returning `(start, end)` byte ranges.
///
/// * Awk mode (`split(" ")`) collapses whitespace runs and never yields empty fields.
/// * Literal mode drops *trailing* empty fields but keeps leading/interior ones.
/// * An empty string always yields zero fields.
pub fn ruby_split(s: &[u8], d: Delim) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    match d {
        Delim::Space => {
            let mut i = 0usize;
            while i < s.len() {
                while i < s.len() && is_split_ws(s[i]) {
                    i += 1;
                }
                if i >= s.len() {
                    break;
                }
                let start = i;
                while i < s.len() && !is_split_ws(s[i]) {
                    i += 1;
                }
                out.push((start, i));
            }
        }
        Delim::Byte(sep) => {
            if s.is_empty() {
                return out;
            }
            let mut start = 0usize;
            for (i, &b) in s.iter().enumerate() {
                if b == sep {
                    out.push((start, i));
                    start = i + 1;
                }
            }
            out.push((start, s.len()));
            while let Some(&(a, b)) = out.last() {
                if a == b {
                    out.pop();
                } else {
                    break;
                }
            }
        }
    }
    out
}

/// Stack-buffer variant of `ruby_split`; `None` requests the wide-row fallback.
#[inline]
pub fn ruby_split_buf<'a>(
    s: &[u8],
    d: Delim,
    buf: &'a mut [(usize, usize); 16],
) -> Option<&'a [(usize, usize)]> {
    let mut count = 0usize;
    match d {
        Delim::Space => {
            let mut i = 0usize;
            while i < s.len() {
                while i < s.len() && is_split_ws(s[i]) {
                    i += 1;
                }
                if i >= s.len() {
                    break;
                }
                let start = i;
                while i < s.len() && !is_split_ws(s[i]) {
                    i += 1;
                }
                if count == buf.len() {
                    return None;
                }
                buf[count] = (start, i);
                count += 1;
            }
        }
        Delim::Byte(sep) => {
            if s.is_empty() {
                return Some(&buf[..0]);
            }
            let mut start = 0usize;
            for (i, &b) in s.iter().enumerate() {
                if b == sep {
                    if count == buf.len() {
                        return None;
                    }
                    buf[count] = (start, i);
                    count += 1;
                    start = i + 1;
                }
            }
            if count == buf.len() {
                return None;
            }
            buf[count] = (start, s.len());
            count += 1;
            while count > 0 {
                let (a, b) = buf[count - 1];
                if a == b {
                    count -= 1;
                } else {
                    break;
                }
            }
        }
    }
    Some(&buf[..count])
}
