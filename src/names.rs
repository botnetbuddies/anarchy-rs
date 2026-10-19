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

/// A parsed input row: the raw column values *before* `Name#init`.
///
/// `None` means the field was absent or the empty string (the original maps
/// `""` to `nil` before doing anything else).
pub type Person = [Option<Vec<u8>>; 6];

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
    pub fn init(p: &Person) -> Name {
        // The original's very first step maps every empty column value to nil.
        let nz = |v: &Option<Vec<u8>>| -> Option<Vec<u8>> {
            v.as_ref().filter(|s| !s.is_empty()).cloned()
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

pub struct LoadedNames {
    pub people: Vec<Person>,
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
pub fn load_names_list(data: &[u8]) -> Result<LoadedNames, String> {
    let mut lines: Vec<Vec<u8>> = readlines(data).into_iter().map(|l| l.to_vec()).collect();

    if lines.first().map(|l| !looks_like_header(l)).unwrap_or(true) {
        lines.insert(0, b"firstname lastname".to_vec());
    }

    let first = lines.first().cloned().unwrap_or_default();
    let delim = if first.contains(&b'\t') {
        Delim::Byte(b'\t')
    } else if first.contains(&b',') {
        Delim::Byte(b',')
    } else if first.contains(&b' ') {
        Delim::Space
    } else {
        return Err("Cannot auto-detect delimiter.".to_string());
    };

    // Header parsing.
    let mut columns: Vec<Option<usize>> = Vec::new();
    let mut stdout: Vec<u8> = Vec::new();
    for (i, (a, b)) in util::ruby_split(&first, delim).iter().enumerate() {
        let mut header = first[*a..*b].to_vec();
        for byte in header.iter_mut() {
            *byte = byte.to_ascii_lowercase();
        }
        let header = util::strip(&header).to_vec();
        match column_index(&header) {
            Some(idx) => {
                if i >= columns.len() {
                    columns.resize(i + 1, None);
                }
                columns[i] = Some(idx);
            }
            None => {
                stdout.extend_from_slice(b"# Ignored unknown column: ");
                stdout.extend_from_slice(&header);
                stdout.push(b'\n');
            }
        }
    }

    // Data rows.
    let mut people: Vec<Person> = Vec::new();
    for line in lines.iter().skip(1) {
        if line.as_slice() == b"\n" {
            continue;
        }
        let fields = util::ruby_split(line, delim);
        if fields.is_empty() {
            continue;
        }
        let mut person: Person = Default::default();
        for (i, (a, b)) in fields.iter().enumerate() {
            let v = &line[*a..*b];
            let lower = util::downcase(v);
            let stripped = util::strip(&lower);
            let idx = columns.get(i).copied().flatten();
            if let Some(idx) = idx {
                if stripped.is_empty() {
                    person[idx] = None;
                } else {
                    person[idx] = Some(stripped.to_vec());
                }
            }
        }
        people.push(person);
    }

    Ok(LoadedNames { people, stdout })
}

/// A wordlist loaded the way the original loads it (`readlines` then `strip!`).
pub struct WordList {
    pub items: Vec<Vec<u8>>,
    /// True when the final line had no trailing newline *and* nothing to strip,
    /// so Ruby's `strip!` would have returned `nil` and the original would raise
    /// when the list is used. We surface that as an error at use time.
    pub last_nil: bool,
}

pub fn load_wordlist(data: &[u8]) -> WordList {
    let mut items = Vec::new();
    let mut last_nil = false;
    let lines = readlines(data);
    let n = lines.len();
    for (i, line) in lines.iter().enumerate() {
        let stripped = util::strip(line);
        if stripped.len() == line.len() && i + 1 == n && !line.ends_with(b"\n") {
            // Ruby `String#strip!` returns nil here.
            last_nil = true;
        } else {
            items.push(stripped.to_vec());
        }
    }
    WordList { items, last_nil }
}

/// `given_name_list`: `readlines.map { |x| x.downcase!; x.strip! }`.
pub fn load_given_list(data: &[u8]) -> WordList {
    let raw = load_wordlist(data);
    WordList {
        items: raw
            .items
            .into_iter()
            .map(|x| util::downcase(&strip_inline(&x)))
            .collect(),
        last_nil: raw.last_nil,
    }
}

fn strip_inline(x: &[u8]) -> Vec<u8> {
    util::strip(x).to_vec()
}

/// `family_name_list`: identical to `given_name_list`.
#[allow(dead_code)]
pub fn load_family_list(data: &[u8]) -> WordList {
    load_given_list(data)
}
