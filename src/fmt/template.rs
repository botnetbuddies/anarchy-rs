//! Template types and compilation.

/// Expansion tokens that can survive the format-string substitution stage.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExpKind {
    AZ,        // %a-z
    AZUp,      // %A-Z
    Given,     // %givenname
    GivenUp,   // %Givenname  (original downcases the list, so this is lowercase too)
    Surname,   // %surname
    SurnameUp, // %Surname
    DD,        // %DD
    D,         // %D
}

impl ExpKind {
    /// Branch priority from the original's `format` method (lower = expanded first).
    #[inline]
    pub fn rank(self) -> u8 {
        match self {
            ExpKind::AZ => 0,
            ExpKind::AZUp => 1,
            ExpKind::Given => 2,
            ExpKind::GivenUp => 3,
            ExpKind::Surname => 4,
            ExpKind::SurnameUp => 5,
            ExpKind::DD => 6,
            ExpKind::D => 7,
        }
    }

    #[inline]
    pub fn is_given(self) -> bool {
        matches!(self, ExpKind::Given | ExpKind::GivenUp)
    }

    #[inline]
    pub fn is_surname(self) -> bool {
        matches!(self, ExpKind::Surname | ExpKind::SurnameUp)
    }

    pub fn text(self) -> &'static [u8] {
        match self {
            ExpKind::AZ => b"%a-z",
            ExpKind::AZUp => b"%A-Z",
            ExpKind::Given => b"%givenname",
            ExpKind::GivenUp => b"%Givenname",
            ExpKind::Surname => b"%surname",
            ExpKind::SurnameUp => b"%Surname",
            ExpKind::DD => b"%DD",
            ExpKind::D => b"%D",
        }
    }
}

/// A compiled template segment.
#[derive(Clone, Copy, Debug)]
pub enum Seg {
    Lit(u32, u32),
    FirstUpper,
    MiddleUpper,
    LastUpper,
    FirstLower,
    MiddleLower,
    LastLower,
    FirstInitUpper,
    MiddleInitUpper,
    LastInitUpper,
    FirstInitLower,
    MiddleInitLower,
    LastInitLower,
    Tok(ExpKind),
}

#[derive(Clone)]
pub enum PluginKind {
    Format { segs: Vec<Seg>, lits: Vec<u8> },
    TruncFirstLast8,
    TruncFirst4Last4,
}

#[derive(Clone)]
pub struct Plugin {
    pub name: String,
    pub kind: PluginKind,
    /// Custom `-F` formats can reassemble expansion tokens from fragments
    /// produced by an earlier expansion round (the original's `format` operates
    /// on strings, not on a fixed template). Those must use the faithful
    /// round-based expansion; the 24 static templates never do.
    pub strict: bool,
    pub required_mask: u8,
    /// Up to three field/dot indexes for token-free, lowercase ASCII names.
    /// Index 6 is a dot and 7 is empty. Custom and expanding formats use None.
    pub ascii_parts: Option<[u8; 3]>,
    /// An earlier selected format emits the same ASCII recipe.
    pub ascii_duplicate: bool,
    /// Rare `%%` templates need Ruby's ordered named-field replacements.
    pub sequential_source: Option<Vec<u8>>,
}

/// ABK -> format string, replicating the original's ordered `gsub!` sequence.
pub fn abk_to_format(s: &str) -> String {
    const SUBS: [(&str, &str); 12] = [
        ("Anna", "%F"),
        ("Boom", "%M"),
        ("Key", "%L"),
        ("anna", "%f"),
        ("boom", "%m"),
        ("key", "%l"),
        ("A", "%i.F"),
        ("B", "%i.M"),
        ("K", "%i.L"),
        ("a", "%i.f"),
        ("b", "%i.m"),
        ("k", "%i.l"),
    ];
    let mut out = s.to_string();
    for (from, to) in SUBS {
        if out.contains(from) {
            out = out.replace(from, to);
        }
    }
    out
}

/// Format-string tokens, longest first (matches the original's ordered `gsub!`).
pub(super) const TOKENS: [(&str, Seg); 20] = [
    ("%i.F", Seg::FirstInitUpper),
    ("%i.M", Seg::MiddleInitUpper),
    ("%i.L", Seg::LastInitUpper),
    ("%i.f", Seg::FirstInitLower),
    ("%i.m", Seg::MiddleInitLower),
    ("%i.l", Seg::LastInitLower),
    ("%Givenname", Seg::Tok(ExpKind::GivenUp)),
    ("%givenname", Seg::Tok(ExpKind::Given)),
    ("%Surname", Seg::Tok(ExpKind::SurnameUp)),
    ("%surname", Seg::Tok(ExpKind::Surname)),
    ("%a-z", Seg::Tok(ExpKind::AZ)),
    ("%A-Z", Seg::Tok(ExpKind::AZUp)),
    ("%DD", Seg::Tok(ExpKind::DD)),
    ("%F", Seg::FirstUpper),
    ("%M", Seg::MiddleUpper),
    ("%L", Seg::LastUpper),
    ("%f", Seg::FirstLower),
    ("%m", Seg::MiddleLower),
    ("%l", Seg::LastLower),
    ("%D", Seg::Tok(ExpKind::D)),
];

pub fn parse_template(s: &str) -> (Vec<Seg>, Vec<u8>) {
    let bytes = s.as_bytes();
    let mut segs = Vec::new();
    let mut lits: Vec<u8> = Vec::new();
    let mut lit_start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let rest = &s[i..];
            let mut matched = false;
            for (tok, seg) in TOKENS {
                if rest.starts_with(tok) {
                    if i > lit_start {
                        let a = lits.len() as u32;
                        lits.extend_from_slice(&bytes[lit_start..i]);
                        segs.push(Seg::Lit(a, lits.len() as u32));
                    }
                    segs.push(seg);
                    i += tok.len();
                    lit_start = i;
                    matched = true;
                    break;
                }
            }
            if !matched {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    if i > lit_start {
        let a = lits.len() as u32;
        lits.extend_from_slice(&bytes[lit_start..i]);
        segs.push(Seg::Lit(a, lits.len() as u32));
    }
    (segs, lits)
}

pub fn compute_required_mask(kind: &PluginKind) -> u8 {
    let mut mask = 0u8;
    match kind {
        PluginKind::TruncFirstLast8 | PluginKind::TruncFirst4Last4 => {
            mask |= (1 << crate::names::FIRSTNAME) | (1 << crate::names::LASTNAME);
        }
        PluginKind::Format { segs, .. } => {
            for seg in segs {
                match seg {
                    Seg::FirstUpper | Seg::FirstLower => mask |= 1 << crate::names::FIRSTNAME,
                    Seg::LastUpper | Seg::LastLower => mask |= 1 << crate::names::LASTNAME,
                    Seg::MiddleUpper | Seg::MiddleLower => mask |= 1 << crate::names::MIDDLENAME,
                    Seg::FirstInitUpper | Seg::FirstInitLower => {
                        mask |= 1 << crate::names::FIRSTINITIAL
                    }
                    Seg::LastInitUpper | Seg::LastInitLower => {
                        mask |= 1 << crate::names::LASTINITIAL
                    }
                    Seg::MiddleInitUpper | Seg::MiddleInitLower => {
                        mask |= 1 << crate::names::MIDDLEINITIAL
                    }
                    Seg::Lit(_, _) | Seg::Tok(_) => {}
                }
            }
        }
    }
    mask
}

pub(super) fn compile_ascii_parts(segs: &[Seg], lits: &[u8]) -> Option<[u8; 3]> {
    use crate::names::*;
    if segs.len() > 3 {
        return None;
    }
    let mut parts = [7; 3];
    for (slot, seg) in parts.iter_mut().zip(segs) {
        *slot = match *seg {
            Seg::FirstUpper | Seg::FirstLower => FIRSTNAME as u8,
            Seg::MiddleUpper | Seg::MiddleLower => MIDDLENAME as u8,
            Seg::LastUpper | Seg::LastLower => LASTNAME as u8,
            Seg::FirstInitUpper | Seg::FirstInitLower => FIRSTINITIAL as u8,
            Seg::MiddleInitUpper | Seg::MiddleInitLower => MIDDLEINITIAL as u8,
            Seg::LastInitUpper | Seg::LastInitLower => LASTINITIAL as u8,
            Seg::Lit(a, b) if &lits[a as usize..b as usize] == b"." => 6,
            _ => return None,
        };
    }
    Some(parts)
}
