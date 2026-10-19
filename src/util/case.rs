//! ASCII and Unicode casing with Ruby semantics.

use super::push_bytes;

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
