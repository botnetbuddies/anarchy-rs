//! Whole-file parsing into borrowed name fields.
use super::{Person, FIRSTINITIAL, FIRSTNAME, LASTINITIAL, LASTNAME, MIDDLEINITIAL, MIDDLENAME};
use crate::util::{self, Delim};

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
