//! The `Name` record and input-file parsing, reproducing the original's
//! initialization order (including its copy/paste bug) exactly.

use crate::util::{self, Delim};

pub const FIRSTNAME: usize = 0;
pub const LASTNAME: usize = 1;
pub const FIRSTINITIAL: usize = 2;
pub const LASTINITIAL: usize = 3;
pub const MIDDLENAME: usize = 4;
pub const MIDDLEINITIAL: usize = 5;

/// The six name fields after `Name#init` has run.
#[derive(Clone, Default)]
pub struct Name {
    pub firstname: Option<Vec<u8>>,
    pub lastname: Option<Vec<u8>>,
    pub middlename: Option<Vec<u8>>,
    pub firstinitial: Option<Vec<u8>>,
    pub lastinitial: Option<Vec<u8>>,
    pub middleinitial: Option<Vec<u8>>,
}

/// Read-only view over the six fields, implemented by both the owned [`Name`]
/// and the allocation-free [`NameRef`].
pub trait NameFields {
    fn firstname(&self) -> Option<&[u8]>;
    fn lastname(&self) -> Option<&[u8]>;
    fn middlename(&self) -> Option<&[u8]>;
    fn firstinitial(&self) -> Option<&[u8]>;
    fn lastinitial(&self) -> Option<&[u8]>;
    fn middleinitial(&self) -> Option<&[u8]>;
}

impl NameFields for Name {
    #[inline]
    fn firstname(&self) -> Option<&[u8]> {
        self.firstname.as_deref()
    }
    #[inline]
    fn lastname(&self) -> Option<&[u8]> {
        self.lastname.as_deref()
    }
    #[inline]
    fn middlename(&self) -> Option<&[u8]> {
        self.middlename.as_deref()
    }
    #[inline]
    fn firstinitial(&self) -> Option<&[u8]> {
        self.firstinitial.as_deref()
    }
    #[inline]
    fn lastinitial(&self) -> Option<&[u8]> {
        self.lastinitial.as_deref()
    }
    #[inline]
    fn middleinitial(&self) -> Option<&[u8]> {
        self.middleinitial.as_deref()
    }
}

/// Borrowed equivalent of [`Name`], used by the `--auto` cross-product so that
/// no `Person` or per-field `Vec` is allocated per generated person.
#[derive(Clone, Copy, Default)]
pub struct NameRef<'a> {
    pub firstname: Option<&'a [u8]>,
    pub lastname: Option<&'a [u8]>,
    pub middlename: Option<&'a [u8]>,
    pub firstinitial: Option<&'a [u8]>,
    pub lastinitial: Option<&'a [u8]>,
    pub middleinitial: Option<&'a [u8]>,
    pub present_mask: u8,
}

impl<'a> NameRef<'a> {
    /// Mirror `Name::init` for a person whose only populated columns are
    /// `firstname` and `lastname` (the `--auto` shape): empty values become
    /// `None`, and both initials are the first character of the respective
    /// field.
    #[inline]
    pub fn from_first_last(first: Option<&'a [u8]>, last: Option<&'a [u8]>) -> NameRef<'a> {
        let first = first.filter(|s| !s.is_empty());
        let last = last.filter(|s| !s.is_empty());
        let mut mask = 0u8;
        if first.is_some() {
            mask |= (1 << FIRSTNAME) | (1 << FIRSTINITIAL);
        }
        if last.is_some() {
            mask |= (1 << LASTNAME) | (1 << LASTINITIAL);
        }
        NameRef {
            firstname: first,
            lastname: last,
            firstinitial: first.map(first_char_slice),
            lastinitial: last.map(first_char_slice),
            middlename: None,
            middleinitial: None,
            present_mask: mask,
        }
    }

    /// Borrowing equivalent of [`Name::init`], including the original's
    /// independent-initial columns and its `@middleinitial = s[:firstinitial]`
    /// / `@lastinitial = s[:firstinitial]` copy/paste bug.
    ///
    /// The hot input-file path uses this to avoid six `Vec` clones per row.
    #[inline]
    pub fn from_person(p: &Person<'a>) -> NameRef<'a> {
        let nz = |v: &Option<&'a [u8]>| v.filter(|s| !s.is_empty());
        let firstname = nz(&p[FIRSTNAME]);
        let middlename = nz(&p[MIDDLENAME]);
        let lastname = nz(&p[LASTNAME]);
        let firstinitial_col = nz(&p[FIRSTINITIAL]);
        let firstinitial = firstinitial_col.or_else(|| firstname.map(first_char_slice));
        // `init` derives these from the name, then overwrites them with the
        // *first-initial column* when the respective column is present.
        let middleinitial = if nz(&p[MIDDLEINITIAL]).is_some() {
            firstinitial_col
        } else {
            middlename.map(first_char_slice)
        };
        let lastinitial = if nz(&p[LASTINITIAL]).is_some() {
            firstinitial_col
        } else {
            lastname.map(first_char_slice)
        };
        let mut mask = 0u8;
        if firstname.is_some() {
            mask |= 1 << FIRSTNAME;
        }
        if lastname.is_some() {
            mask |= 1 << LASTNAME;
        }
        if firstinitial.is_some() {
            mask |= 1 << FIRSTINITIAL;
        }
        if lastinitial.is_some() {
            mask |= 1 << LASTINITIAL;
        }
        if middlename.is_some() {
            mask |= 1 << MIDDLENAME;
        }
        if middleinitial.is_some() {
            mask |= 1 << MIDDLEINITIAL;
        }
        NameRef {
            firstname,
            lastname,
            middlename,
            firstinitial,
            middleinitial,
            lastinitial,
            present_mask: mask,
        }
    }
}

#[inline]
fn first_char_slice(s: &[u8]) -> &[u8] {
    &s[..util::nth_char_boundary(s, 1)]
}

impl NameFields for NameRef<'_> {
    #[inline]
    fn firstname(&self) -> Option<&[u8]> {
        self.firstname
    }
    #[inline]
    fn lastname(&self) -> Option<&[u8]> {
        self.lastname
    }
    #[inline]
    fn middlename(&self) -> Option<&[u8]> {
        self.middlename
    }
    #[inline]
    fn firstinitial(&self) -> Option<&[u8]> {
        self.firstinitial
    }
    #[inline]
    fn lastinitial(&self) -> Option<&[u8]> {
        self.lastinitial
    }
    #[inline]
    fn middleinitial(&self) -> Option<&[u8]> {
        self.middleinitial
    }
}

/// A parsed input row: the raw column values *before* `Name#init`.
///
/// `None` means the field was absent or the empty string (the original maps
/// `""` to `nil` before doing anything else).
pub type Person<'a> = [Option<&'a [u8]>; 6];

/// Ruby `String#[]` for a single-character range: `s[0..0]`.
fn first_char(s: &[u8]) -> Vec<u8> {
    let end = util::nth_char_boundary(s, 1);
    s[..end].to_vec()
}

impl Name {
    /// `Name#init`, in the original's exact order.
    ///
    /// Quirk preserved: an explicit `middleinitial`/`lastinitial` column
    /// overwrites the incremented initial with `s[:firstinitial]` (the raw
    /// column value), not with the supplied initial. This is a copy/paste bug
    /// in the original and is reproduced verbatim.
    pub fn init(p: &Person<'_>) -> Name {
        // The original's very first step maps every empty column value to nil.
        let nz = |v: &Option<&[u8]>| -> Option<Vec<u8>> {
            v.filter(|s| !s.is_empty()).map(|s| s.to_vec())
        };
        let p = [
            nz(&p[FIRSTNAME]),
            nz(&p[LASTNAME]),
            nz(&p[FIRSTINITIAL]),
            nz(&p[LASTINITIAL]),
            nz(&p[MIDDLENAME]),
            nz(&p[MIDDLEINITIAL]),
        ];
        let firstname = p[FIRSTNAME].clone();
        let middlename = p[MIDDLENAME].clone();
        let lastname = p[LASTNAME].clone();

        let mut n = Name {
            firstname: firstname.clone(),
            lastname: lastname.clone(),
            middlename: middlename.clone(),
            firstinitial: firstname.as_deref().map(first_char),
            middleinitial: middlename.as_deref().map(first_char),
            lastinitial: lastname.as_deref().map(first_char),
        };

        if let Some(fi) = &p[FIRSTINITIAL] {
            n.firstinitial = Some(fi.clone());
        }
        // Original bug: `@middleinitial = s[:firstinitial]`
        if p[MIDDLEINITIAL].is_some() {
            n.middleinitial = p[FIRSTINITIAL].clone();
        }
        // Original bug: `@lastinitial = s[:firstinitial]`
        if p[LASTINITIAL].is_some() {
            n.lastinitial = p[FIRSTINITIAL].clone();
        }
        n
    }
}

fn column_index(header: &[u8]) -> Option<usize> {
    match header {
        b"firstname" => Some(FIRSTNAME),
        b"lastname" => Some(LASTNAME),
        b"firstinitial" => Some(FIRSTINITIAL),
        b"lastinitial" => Some(LASTINITIAL),
        b"middlename" => Some(MIDDLENAME),
        b"middleinitial" => Some(MIDDLEINITIAL),
        _ => None,
    }
}

/// The header-validation regex from `load_names_list` (case-sensitive substring
/// alternation).
const HEADER_TOKENS: [&[u8]; 12] = [
    b"Firstname",
    b"Middlename",
    b"Lastname",
    b"firstname",
    b"middlename",
    b"lastname",
    b"firstinitial",
    b"lastinitial",
    b"middleinitial",
    b"Firstinitial",
    b"Lastinitial",
    b"Middleinitial",
];

fn looks_like_header(line: &[u8]) -> bool {
    HEADER_TOKENS.iter().any(|tok| contains(line, tok))
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    if needle.len() > hay.len() {
        return false;
    }
    hay.windows(needle.len()).any(|w| w == needle)
}

pub struct LoadedNames<'a> {
    pub people: Vec<Person<'a>>,
    /// Lines printed by the original while parsing unknown column headers.
    pub stdout: Vec<u8>,
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

/// `load_names_list` from the original.
pub fn load_names_list<'a>(data: &'a mut Vec<u8>) -> Result<LoadedNames<'a>, String> {
    if data.is_empty() {
        return Ok(LoadedNames {
            people: Vec::new(),
            stdout: Vec::new(),
        });
    }

    let first_nl = data.iter().position(|&b| b == b'\n');
    let first_line_len = match first_nl {
        Some(pos) => pos + 1,
        None => data.len(),
    };
    let has_header = looks_like_header(&data[..first_line_len]);
    let (first_line_buf, data_start) = if has_header {
        (data[..first_line_len].to_vec(), first_line_len)
    } else {
        (b"firstname lastname".to_vec(), 0)
    };

    let first_line = first_line_buf.as_slice();
    let delim = if first_line.contains(&b'\t') {
        Delim::Byte(b'\t')
    } else if first_line.contains(&b',') {
        Delim::Byte(b',')
    } else if first_line.contains(&b' ') {
        Delim::Space
    } else {
        return Err("Cannot auto-detect delimiter.".to_string());
    };

    // Header parsing.
    let mut columns: Vec<Option<usize>> = Vec::new();
    let mut stdout: Vec<u8> = Vec::new();
    for (i, &(a, b)) in util::ruby_split(first_line, delim).iter().enumerate() {
        let mut header = first_line[a..b].to_vec();
        for byte in header.iter_mut() {
            *byte = byte.to_ascii_lowercase();
        }
        let header = util::strip(&header);
        match column_index(header) {
            Some(idx) => {
                if i >= columns.len() {
                    columns.resize(i + 1, None);
                }
                columns[i] = Some(idx);
            }
            None => {
                stdout.extend_from_slice(b"# Ignored unknown column: ");
                stdout.extend_from_slice(header);
                stdout.push(b'\n');
            }
        }
    }

    // Keep ASCII input in its original allocation. Full Unicode lowercasing
    // can change byte lengths, so finish that transformation before borrowing.
    if !util::ascii_lowercase_in_place(&mut data[data_start..]) {
        let lower = util::downcase(&data[data_start..]);
        data.truncate(data_start);
        data.extend_from_slice(&lower);
    }

    let data_ref: &'a [u8] = &*data;
    let mut people: Vec<Person<'a>> = Vec::new();
    let mut field_buf = [(0usize, 0usize); 16];
    let mut line_start = data_start;
    let total_len = data_ref.len();

    while line_start < total_len {
        let line_end = match data_ref[line_start..].iter().position(|&b| b == b'\n') {
            Some(pos) => line_start + pos + 1,
            None => total_len,
        };
        let line = &data_ref[line_start..line_end];
        line_start = line_end;

        if line == b"\n" || line == b"\r\n" {
            continue;
        }
        let overflow;
        let fields = match util::ruby_split_buf(line, delim, &mut field_buf) {
            Some(fields) => fields,
            None => {
                overflow = util::ruby_split(line, delim);
                overflow.as_slice()
            }
        };
        if fields.is_empty() {
            continue;
        }
        let mut person: Person<'a> = [None; 6];
        for (i, &(a, b)) in fields.iter().enumerate() {
            let v = &line[a..b];
            let stripped = util::strip(v);
            let idx = columns.get(i).copied().flatten();
            if let Some(idx) = idx {
                person[idx] = (!stripped.is_empty()).then_some(stripped);
            }
        }
        people.push(person);
    }

    Ok(LoadedNames { people, stdout })
}

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

/// `family_name_list`: identical to `given_name_list`.
#[allow(dead_code)]
pub fn load_family_list(data: &[u8]) -> WordList {
    load_given_list(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person<'a>(cols: [&'a str; 6]) -> Person<'a> {
        cols.map(|s| {
            if s.is_empty() {
                None
            } else {
                Some(s.as_bytes())
            }
        })
    }

    /// `NameRef::from_person` must be indistinguishable from `Name::init`,
    /// including the original's `middleinitial`/`lastinitial` copy/paste bug.
    #[test]
    fn from_person_matches_init() {
        let cases = [
            ["anna", "key", "", "", "", ""],
            ["anna", "key", "", "", "boom", ""],
            ["Anna", "Key", "A", "K", "Boom", "M"],
            ["anna", "key", "", "K", "", ""],
            ["anna", "key", "", "", "", "M"],
            ["", "", "", "", "", ""],
            ["é", "key", "", "", "", ""],
            ["anna", "key", "É", "", "", ""],
        ];
        for c in cases {
            let p = person(c);
            let owned = Name::init(&p);
            let borrowed = NameRef::from_person(&p);
            assert_eq!(owned.firstname.as_deref(), borrowed.firstname, "{c:?}");
            assert_eq!(owned.lastname.as_deref(), borrowed.lastname, "{c:?}");
            assert_eq!(owned.middlename.as_deref(), borrowed.middlename, "{c:?}");
            assert_eq!(
                owned.firstinitial.as_deref(),
                borrowed.firstinitial,
                "{c:?}"
            );
            assert_eq!(owned.lastinitial.as_deref(), borrowed.lastinitial, "{c:?}");
            assert_eq!(
                owned.middleinitial.as_deref(),
                borrowed.middleinitial,
                "{c:?}"
            );
        }
    }
}
