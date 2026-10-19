//! Small byte/string helpers that mirror Ruby's String semantics.
//!
//! Names in the input files and wordlists are essentially always ASCII, so every
//! hot path takes an ASCII fast branch and only falls back to full Unicode
//! case-folding when a non-ASCII byte is actually present.

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

/// Exact equality for short usernames without a call to musl's bytewise memcmp.
/// Overlapping first/last words cover every byte, including unaligned slices.
#[inline(always)]
pub fn bytes_equal(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let n = a.len();
    if n > 16 {
        a == b
    } else if n >= 8 {
        u64::from_ne_bytes(a[..8].try_into().unwrap())
            == u64::from_ne_bytes(b[..8].try_into().unwrap())
            && u64::from_ne_bytes(a[n - 8..].try_into().unwrap())
                == u64::from_ne_bytes(b[n - 8..].try_into().unwrap())
    } else if n >= 4 {
        u32::from_ne_bytes(a[..4].try_into().unwrap())
            == u32::from_ne_bytes(b[..4].try_into().unwrap())
            && u32::from_ne_bytes(a[n - 4..].try_into().unwrap())
                == u32::from_ne_bytes(b[n - 4..].try_into().unwrap())
    } else if n >= 2 {
        u16::from_ne_bytes(a[..2].try_into().unwrap())
            == u16::from_ne_bytes(b[..2].try_into().unwrap())
            && a[n - 1] == b[n - 1]
    } else {
        n == 0 || a[0] == b[0]
    }
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

#[inline]
fn ascii_lowercase_byte(b: u8) -> u8 {
    if b.is_ascii_uppercase() {
        b + 32
    } else {
        b
    }
}

/// `true` if every byte is ASCII.
#[inline]
pub fn is_ascii(s: &[u8]) -> bool {
    s.iter().all(|&b| b < 0x80)
}

/// Ruby `String#downcase`, appending into an existing buffer.
///
/// Writing into the caller's buffer avoids one heap allocation per format
/// segment, which the profiler showed to be a dominant cost.
pub fn downcase_into(s: &[u8], out: &mut Vec<u8>) {
    if is_ascii(s) {
        let start = out.len();
        push_bytes(out, s);
        ascii_lowercase_in_place(&mut out[start..]);
    } else {
        let text = String::from_utf8_lossy(s);
        // Ruby uses context-free case mappings; str::to_lowercase applies the
        // contextual final-sigma rule (ΟΣ -> ος), which Ruby does not.
        for c in text.chars().flat_map(char::to_lowercase) {
            let mut bytes = [0; 4];
            out.extend_from_slice(c.encode_utf8(&mut bytes).as_bytes());
        }
    }
}

/// Ruby `String#downcase`. ASCII fast path; context-free Unicode fallback.
pub fn downcase(s: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(s.len());
    downcase_into(s, &mut v);
    v
}

/// In-place ASCII lowercase; returns `false` if the string was not pure ASCII
/// (in which case the caller must use [`downcase`]).
#[inline]
pub fn ascii_lowercase_in_place(s: &mut [u8]) -> bool {
    let mut all_ascii = true;
    for b in s.iter_mut() {
        if *b >= 0x80 {
            all_ascii = false;
        } else {
            *b = ascii_lowercase_byte(*b);
        }
    }
    all_ascii
}

/// Append `s` to `out`.
///
/// ## Why this exists (profiler-justified `unsafe`)
///
/// The format engine appends millions of short byte slices (name fragments and
/// format literals). `Vec::extend_from_slice` lowers to a call to `memcpy`, and
/// `perf` attributed ~18% of a benchmark run to that symbol, almost all of it
/// call overhead on slices of a handful of bytes. For slices up to 16 bytes this
/// copies with constant-size, inlined `copy_nonoverlapping` calls instead.
///
/// ## Safety
///
/// This is the *only* unsafe block in the program. It is sound because:
/// * `out.reserve(s.len())` guarantees spare capacity for `s.len()` more bytes,
/// * the 8/4/2/1-byte sub-copies use the overlapping-tail trick, so every byte
///   written lies within `[len, len + s.len())` which is reserved, and
/// * `s` must not alias `out` (all call sites pass independent buffers).
///
/// The raw pointer arithmetic never reads or writes out of bounds, and
/// `set_len` only exposes bytes that were just initialised.
#[inline(always)]
pub fn push_bytes(out: &mut Vec<u8>, s: &[u8]) {
    let len = out.len();
    if s.len() > 16 {
        out.extend_from_slice(s);
        return;
    }
    out.reserve(s.len());
    // SAFETY: `reserve` above made room for `s.len()` bytes at `out[len]`, and
    // every sub-copy below writes only inside that region. `s` is a distinct
    // allocation from `out`, so the non-overlapping copies are well-defined.
    unsafe {
        let n = s.len();
        let src = s.as_ptr();
        let dst = out.as_mut_ptr().add(len);
        if n >= 8 {
            std::ptr::copy_nonoverlapping(src, dst, 8);
            std::ptr::copy_nonoverlapping(src.add(n - 8), dst.add(n - 8), 8);
        } else if n >= 4 {
            std::ptr::copy_nonoverlapping(src, dst, 4);
            std::ptr::copy_nonoverlapping(src.add(n - 4), dst.add(n - 4), 4);
        } else if n >= 2 {
            std::ptr::copy_nonoverlapping(src, dst, 2);
            std::ptr::copy_nonoverlapping(src.add(n - 2), dst.add(n - 2), 2);
        } else if n == 1 {
            std::ptr::copy_nonoverlapping(src, dst, 1);
        }
        out.set_len(len + n);
    }
}

/// Ruby `upcase_it(s)`: upcase the first character, downcase the rest,
/// writing into an existing buffer. Returns `false` when Ruby would have
/// produced `nil` (i.e. the input was empty).
///
/// Mirrors:
/// ```ruby
/// def upcase_it(s)
///   if s and not s == ""
///     s[0..0].upcase + s[1..-1].downcase
///   else
///     nil
///   end
/// end
/// ```
pub fn upcase_it_into(s: &[u8], out: &mut Vec<u8>) -> bool {
    if s.is_empty() {
        return false;
    }
    if is_ascii(s) {
        let start = out.len();
        push_bytes(out, s);
        out[start] = out[start].to_ascii_uppercase();
        for b in out[start + 1..].iter_mut() {
            *b = ascii_lowercase_byte(*b);
        }
    } else {
        let text = String::from_utf8_lossy(s);
        let mut chars = text.chars();
        // Non-empty input guarantees at least one char.
        if let Some(first) = chars.next() {
            for c in first.to_uppercase() {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
            downcase_into(chars.as_str().as_bytes(), out);
        }
    }
    true
}

/// Ruby `upcase_it(s)` followed by `String#downcase`, i.e.
/// `downcase(s[0..0].upcase + s[1..-1].downcase)`.
///
/// This is the composition the original applies to every `%F`/`%L` segment
/// when `--case-insensitive` is on: the segment is upcased during template
/// substitution and the whole candidate is downcased just before printing.
/// Computing it in one pass yields the same bytes (including the `ß -> SS ->
/// ss` expansion, which plain `downcase` would not) while avoiding a second
/// full scan and a temporary buffer.
///
/// Returns `false` when Ruby's `upcase_it` would have returned `nil` (empty
/// input).
pub fn upcase_then_downcase_into(s: &[u8], out: &mut Vec<u8>) -> bool {
    if s.is_empty() {
        return false;
    }
    if is_ascii(s) {
        let start = out.len();
        push_bytes(out, s);
        ascii_lowercase_in_place(&mut out[start..]);
        return true;
    }
    let text = String::from_utf8_lossy(s);
    let end = nth_char_boundary(text.as_bytes(), 1);
    let first = &text[..end];
    let rest = &text[end..];
    // `downcase(upcase(first))` — not always `downcase(first)` (e.g. ß -> ss).
    let first_up = first.to_uppercase();
    downcase_into(first_up.as_bytes(), out);
    // `downcase(downcase(rest)) == downcase(rest)`.
    downcase_into(rest.as_bytes(), out);
    true
}

/// Byte index just past the `n`-th character (Ruby `String#[]` range semantics).
pub fn nth_char_boundary(s: &[u8], n: usize) -> usize {
    if is_ascii(s) {
        return n.min(s.len());
    }
    let mut count = 0usize;
    let mut i = 0usize;
    while i < s.len() && count < n {
        // advance one UTF-8 character
        let b = s[i];
        let len = if b < 0x80 {
            1
        } else if b >> 5 == 0b110 {
            2
        } else if b >> 4 == 0b1110 {
            3
        } else if b >> 3 == 0b11110 {
            4
        } else {
            1
        };
        i += len;
        count += 1;
    }
    i
}

/// Ruby `String#delete(" ")` combined with `String#delete("'")`.
///
/// The original always runs both, so we strip spaces and apostrophes in one pass.
#[inline]
pub fn strip_spaces_and_apostrophes(s: &mut Vec<u8>) {
    if s.iter().any(|&b| b == b' ' || b == b'\'') {
        s.retain(|&b| b != b' ' && b != b'\'');
    }
}

#[cfg(test)]
mod tests {
    use super::bytes_equal;

    #[test]
    fn short_equality_checks_every_byte_and_length() {
        for n in 0..=64 {
            let bytes: Vec<u8> = (0..n + 1).map(|i| (i * 71) as u8).collect();
            let a = &bytes[1..]; // Include unaligned word reads.
            assert!(bytes_equal(a, a));
            assert!(!bytes_equal(a, &bytes));
            for i in 0..n {
                let mut different = a.to_vec();
                different[i] ^= 1;
                assert!(!bytes_equal(a, &different), "length {n}, byte {i}");
            }
        }
    }
}
