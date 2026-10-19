//! The format engine: plugin registry, ABK -> format-string translation,
//! precompiled segment templates, and the iterative expansion engine.
//!
//! Faithfulness note: the original substitutes name values into the *format
//! string* and only then runs the expansion loop over the resulting string.
//! That means a value can reassemble an expansion token across a boundary
//! (e.g. `-F 'x%givenname%D'` gets its `a` rewritten by the ABK pass and then
//! restored by the `%i.f` substitution). We therefore resolve each template to
//! a byte string per person and re-tokenize it, exactly like the original,
//! while keeping regex-free precompiled segments for the static plugins.

use std::sync::OnceLock;

use crate::names::{load_given_list, NameFields, WordList};
use crate::util;

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
const TOKENS: [(&str, Seg); 20] = [
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

/// Expansion tokens recognised by the original's post-substitution loop.
const EXP_TOKENS: [(&[u8], ExpKind); 8] = [
    (b"%givenname", ExpKind::Given),
    (b"%Givenname", ExpKind::GivenUp),
    (b"%surname", ExpKind::Surname),
    (b"%Surname", ExpKind::SurnameUp),
    (b"%a-z", ExpKind::AZ),
    (b"%A-Z", ExpKind::AZUp),
    (b"%DD", ExpKind::DD),
    (b"%D", ExpKind::D),
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

fn format_plugin(name: &str, raw: &str, abk: bool, strict: bool) -> Plugin {
    let expanded = if abk {
        abk_to_format(raw)
    } else {
        raw.to_string()
    };
    let (segs, lits) = parse_template(&expanded);
    let ascii_parts = if strict {
        None
    } else {
        compile_ascii_parts(&segs, &lits)
    };
    let sequential_source =
        (strict && expanded.contains("%%")).then(|| expanded.as_bytes().to_vec());
    let kind = PluginKind::Format { segs, lits };
    let required_mask = compute_required_mask(&kind);
    Plugin {
        name: name.to_string(),
        kind,
        strict,
        required_mask,
        ascii_parts,
        ascii_duplicate: false,
        sequential_source,
    }
}

/// Run after format selection: a subset may omit the earlier equivalent format.
pub fn mark_ascii_duplicates(plugins: &mut [Plugin]) {
    for i in 0..plugins.len() {
        plugins[i].ascii_duplicate = plugins[i].ascii_parts.is_some()
            && plugins[..i]
                .iter()
                .any(|p| p.ascii_parts == plugins[i].ascii_parts);
    }
}

fn compile_ascii_parts(segs: &[Seg], lits: &[u8]) -> Option<[u8; 3]> {
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

/// The 24 static plugins, in registration order (`format-plugins.rb`).
pub fn static_plugins() -> Vec<Plugin> {
    vec![
        format_plugin("first", "%f", false, false),
        format_plugin("firstlast", "annakey", true, false),
        format_plugin("first.last", "anna.key", true, false),
        Plugin {
            name: "firstlast[8]".to_string(),
            kind: PluginKind::TruncFirstLast8,
            strict: false,
            required_mask: (1 << crate::names::FIRSTNAME) | (1 << crate::names::LASTNAME),
            ascii_parts: None,
            ascii_duplicate: false,
            sequential_source: None,
        },
        Plugin {
            name: "first[4]last[4]".to_string(),
            kind: PluginKind::TruncFirst4Last4,
            strict: false,
            required_mask: (1 << crate::names::FIRSTNAME) | (1 << crate::names::LASTNAME),
            ascii_parts: None,
            ascii_duplicate: false,
            sequential_source: None,
        },
        format_plugin("firstl", "annak", true, false),
        format_plugin("f.last", "a.key", true, false),
        format_plugin("flast", "akey", true, false),
        format_plugin("lfirst", "kanna", true, false),
        format_plugin("l.first", "k.anna", true, false),
        format_plugin("lastf", "keya", true, false),
        format_plugin("last", "key", true, false),
        format_plugin("last.f", "key.a", true, false),
        format_plugin("last.first", "key.anna", true, false),
        format_plugin("FLast", "AKey", true, false),
        format_plugin("first1", "anna%D", true, false),
        format_plugin("fl", "ak", true, false),
        format_plugin("fmlast", "abkey", true, false),
        format_plugin("firstmiddlelast", "annaboomkey", true, false),
        format_plugin("fml", "abk", true, false),
        format_plugin("FL", "AK", true, false),
        format_plugin("FirstLast", "%F%L", true, false),
        format_plugin("First.Last", "%F.%L", true, false),
        format_plugin("Last", "Key", true, false),
    ]
}

/// Custom `-F` plugin: always routed through the ABK translator (`format_anna`).
/// Uses the faithful round-based expansion because arbitrary formats can
/// reassemble tokens from expanded fragments.
pub fn custom_plugin(fmt: &str) -> Plugin {
    format_plugin("custom", fmt, true, true)
}

/// Lazily-loaded substitution dictionaries (`given_name_list` / `family_name_list`).
pub struct Ctx {
    pub given_path: String,
    pub family_path: String,
    given: OnceLock<WordList>,
    family: OnceLock<WordList>,
}

fn load_or_die(path: &str) -> WordList {
    match std::fs::read(path) {
        Ok(data) => load_given_list(&data),
        Err(e) => {
            eprintln!("username-anarchy: cannot read file: {path}: {e}");
            std::process::exit(1);
        }
    }
}

impl Ctx {
    pub fn new(given_path: String, family_path: String) -> Ctx {
        Ctx {
            given_path,
            family_path,
            given: OnceLock::new(),
            family: OnceLock::new(),
        }
    }

    #[inline]
    pub fn given(&self) -> &WordList {
        self.given.get_or_init(|| load_or_die(&self.given_path))
    }

    #[inline]
    pub fn family(&self) -> &WordList {
        self.family.get_or_init(|| load_or_die(&self.family_path))
    }

    pub fn readable(path: &str) -> bool {
        std::fs::File::open(path).is_ok()
    }
}

#[inline]
pub fn exp_len(kind: ExpKind, ctx: &Ctx) -> usize {
    match kind {
        ExpKind::AZ | ExpKind::AZUp => 26,
        ExpKind::Given | ExpKind::GivenUp => ctx.given().len(),
        ExpKind::Surname | ExpKind::SurnameUp => ctx.family().len(),
        ExpKind::DD => 100,
        ExpKind::D => 10,
    }
}

#[inline]
fn write_exp(kind: ExpKind, i: usize, ctx: &Ctx, out: &mut Vec<u8>) {
    match kind {
        ExpKind::AZ => out.push(b'a' + i as u8),
        ExpKind::AZUp => out.push(b'A' + i as u8),
        ExpKind::Given | ExpKind::GivenUp => util::push_bytes(out, ctx.given().get(i)),
        ExpKind::Surname | ExpKind::SurnameUp => util::push_bytes(out, ctx.family().get(i)),
        ExpKind::DD => {
            out.push(b'0' + (i / 10) as u8);
            out.push(b'0' + (i % 10) as u8);
        }
        ExpKind::D => out.push(b'0' + i as u8),
    }
}

/// Resolve a template to the substituted byte string the original would hand to
/// its expansion loop.
pub fn resolve_string<N: NameFields + ?Sized>(
    segs: &[Seg],
    lits: &[u8],
    name: &N,
    out: &mut Vec<u8>,
) {
    out.clear();
    for seg in segs {
        match *seg {
            Seg::Lit(a, b) => util::push_bytes(out, &lits[a as usize..b as usize]),
            Seg::FirstUpper => val(name.firstname(), out, true, b"%Givenname"),
            Seg::MiddleUpper => val(name.middlename(), out, true, b"%Givenname"),
            Seg::LastUpper => val(name.lastname(), out, true, b"%Surname"),
            Seg::FirstLower => val(name.firstname(), out, false, b"%givenname"),
            Seg::MiddleLower => val(name.middlename(), out, false, b"%givenname"),
            Seg::LastLower => val(name.lastname(), out, false, b"%surname"),
            Seg::FirstInitUpper => val(name.firstinitial(), out, true, b"%A-Z"),
            Seg::MiddleInitUpper => val(name.middleinitial(), out, true, b"%A-Z"),
            Seg::LastInitUpper => val(name.lastinitial(), out, true, b"%A-Z"),
            Seg::FirstInitLower => val(name.firstinitial(), out, false, b"%a-z"),
            Seg::MiddleInitLower => val(name.middleinitial(), out, false, b"%a-z"),
            Seg::LastInitLower => val(name.lastinitial(), out, false, b"%a-z"),
            Seg::Tok(k) => util::push_bytes(out, k.text()),
        }
    }
}

/// A replacement can introduce a token consumed later in Ruby's gsub order:
/// `%%f` -> `%mary` -> `bethary` when the middle name is Beth.
pub fn resolve_sequential<N: NameFields + ?Sized>(
    source: &[u8],
    name: &N,
    out: &mut Vec<u8>,
    tmp: &mut Vec<u8>,
) {
    const ORDER: [&str; 12] = [
        "%F", "%M", "%L", "%f", "%m", "%l", "%i.F", "%i.M", "%i.L", "%i.f", "%i.m", "%i.l",
    ];
    out.clear();
    out.extend_from_slice(source);
    let mut replacement = Vec::new();
    for token in ORDER {
        let needle = token.as_bytes();
        if !out.windows(needle.len()).any(|w| w == needle) {
            continue;
        }
        let seg = TOKENS.iter().find(|(t, _)| *t == token).unwrap().1;
        resolve_string(&[seg], &[], name, &mut replacement);
        tmp.clear();
        let mut rest = out.as_slice();
        while let Some(i) = rest.windows(needle.len()).position(|w| w == needle) {
            tmp.extend_from_slice(&rest[..i]);
            tmp.extend_from_slice(&replacement);
            rest = &rest[i + needle.len()..];
        }
        tmp.extend_from_slice(rest);
        std::mem::swap(out, tmp);
    }
}

/// Resolve a static template for a case-insensitive run.
///
/// Uppercase segments use `downcase(upcase_it(x))` — the exact composition the
/// original applies (upcase during substitution, downcase before printing) —
/// and lowercase segments are downcased as usual. The result is therefore
/// already in final form, so [`crate::engine::Emitter::emit_lower`] can insert
/// it without a second case-folding pass. Only used for the 24 built-in
/// templates, whose literals contain no letters.
pub fn resolve_string_lower<N: NameFields + ?Sized>(
    segs: &[Seg],
    lits: &[u8],
    name: &N,
    out: &mut Vec<u8>,
) {
    out.clear();
    for seg in segs {
        match *seg {
            Seg::Lit(a, b) => util::push_bytes(out, &lits[a as usize..b as usize]),
            Seg::FirstUpper => val_lower(name.firstname(), out, b"%Givenname"),
            Seg::MiddleUpper => val_lower(name.middlename(), out, b"%Givenname"),
            Seg::LastUpper => val_lower(name.lastname(), out, b"%Surname"),
            Seg::FirstLower => val(name.firstname(), out, false, b"%givenname"),
            Seg::MiddleLower => val(name.middlename(), out, false, b"%givenname"),
            Seg::LastLower => val(name.lastname(), out, false, b"%surname"),
            Seg::FirstInitUpper => val_lower(name.firstinitial(), out, b"%A-Z"),
            Seg::MiddleInitUpper => val_lower(name.middleinitial(), out, b"%A-Z"),
            Seg::LastInitUpper => val_lower(name.lastinitial(), out, b"%A-Z"),
            Seg::FirstInitLower => val(name.firstinitial(), out, false, b"%a-z"),
            Seg::MiddleInitLower => val(name.middleinitial(), out, false, b"%a-z"),
            Seg::LastInitLower => val(name.lastinitial(), out, false, b"%a-z"),
            Seg::Tok(k) => util::push_bytes(out, k.text()),
        }
    }
}

fn val(v: Option<&[u8]>, out: &mut Vec<u8>, upper: bool, missing: &[u8]) {
    match v {
        Some(s) => {
            if upper {
                if !util::upcase_it_into(s, out) {
                    util::push_bytes(out, missing);
                }
            } else {
                util::downcase_into(s, out);
            }
        }
        None => util::push_bytes(out, missing),
    }
}

fn val_lower(v: Option<&[u8]>, out: &mut Vec<u8>, missing: &[u8]) {
    match v {
        Some(s) => {
            if !util::upcase_then_downcase_into(s, out) {
                util::push_bytes(out, missing);
            }
        }
        None => util::push_bytes(out, missing),
    }
}

/// [`resolve_string_lower`] for fields already known to be lowercase ASCII
/// (see [`field_props`]): present segments are copied verbatim, while missing
/// fields still produce the expansion token. This is the common input-file
/// case and skips the per-segment `is_ascii` + lowercase scans.
pub fn resolve_string_lower_raw<N: NameFields + ?Sized>(
    segs: &[Seg],
    lits: &[u8],
    name: &N,
    out: &mut Vec<u8>,
) {
    out.clear();
    for seg in segs {
        match *seg {
            Seg::Lit(a, b) => util::push_bytes(out, &lits[a as usize..b as usize]),
            Seg::FirstUpper => val_raw(name.firstname(), out, b"%Givenname"),
            Seg::MiddleUpper => val_raw(name.middlename(), out, b"%Givenname"),
            Seg::LastUpper => val_raw(name.lastname(), out, b"%Surname"),
            Seg::FirstLower => val_raw(name.firstname(), out, b"%givenname"),
            Seg::MiddleLower => val_raw(name.middlename(), out, b"%givenname"),
            Seg::LastLower => val_raw(name.lastname(), out, b"%surname"),
            Seg::FirstInitUpper => val_raw(name.firstinitial(), out, b"%A-Z"),
            Seg::MiddleInitUpper => val_raw(name.middleinitial(), out, b"%A-Z"),
            Seg::LastInitUpper => val_raw(name.lastinitial(), out, b"%A-Z"),
            Seg::FirstInitLower => val_raw(name.firstinitial(), out, b"%a-z"),
            Seg::MiddleInitLower => val_raw(name.middleinitial(), out, b"%a-z"),
            Seg::LastInitLower => val_raw(name.lastinitial(), out, b"%a-z"),
            Seg::Tok(k) => util::push_bytes(out, k.text()),
        }
    }
}

fn val_raw(v: Option<&[u8]>, out: &mut Vec<u8>, missing: &[u8]) {
    match v {
        Some(s) if !s.is_empty() => util::push_bytes(out, s),
        _ => util::push_bytes(out, missing),
    }
}

/// One scan over the six fields, returning:
///
/// * `plain`: every present field is lowercase ASCII without `%` tokens (so case folding is a
///   no-op and the uppercase composition equals the lowercase one), and
/// * `clean`: no field contains a space or apostrophe (so the post-processing
///   `delete(" ")/delete("'")` pass cannot change the candidate).
///
/// Both properties are sufficient because the built-in templates' literals are
/// only `.`; custom formats bypass them.
pub fn field_props<N: NameFields + ?Sized>(name: &N) -> (bool, bool) {
    let mut plain = true;
    let mut clean = true;
    let fields = [
        name.firstname(),
        name.middlename(),
        name.lastname(),
        name.firstinitial(),
        name.middleinitial(),
        name.lastinitial(),
    ];
    for f in fields.iter().flatten() {
        for &b in *f {
            if b >= 0x80 || b.is_ascii_uppercase() || b == b'%' {
                plain = false;
            }
            if b == b' ' || b == b'\'' {
                clean = false;
            }
        }
        if !plain && !clean {
            break;
        }
    }
    (plain, clean)
}

/// One expansion-token occurrence in a resolved string.
pub struct Occ {
    pub off: usize,
    pub len: usize,
    pub kind: ExpKind,
    /// Position in the rank-sorted odometer order (0 = outermost loop).
    pub odo: usize,
}

/// Whether `s` contains at least one expansion token.
///
/// Equivalent to `!tokenize(s).is_empty()`, but allocation-free; the common
/// no-substitution path only needs the yes/no answer.
pub fn has_token(s: &[u8]) -> bool {
    let mut i = 0usize;
    while i < s.len() {
        if s[i] == b'%' {
            for (tok, _) in EXP_TOKENS {
                if s[i..].starts_with(tok) {
                    return true;
                }
            }
        }
        i += 1;
    }
    false
}

/// Find every expansion token in the resolved string, in left-to-right order.
pub fn tokenize(s: &[u8]) -> Vec<Occ> {
    let mut raw: Vec<(usize, usize, ExpKind)> = Vec::new();
    let mut i = 0usize;
    while i < s.len() {
        if s[i] == b'%' {
            let mut matched = false;
            for (tok, kind) in EXP_TOKENS {
                if s[i..].starts_with(tok) {
                    raw.push((i, tok.len(), kind));
                    i += tok.len();
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
    // Odometer order: by (branch rank, offset).
    let mut order: Vec<usize> = (0..raw.len()).collect();
    order.sort_by(|&a, &b| {
        raw[a]
            .2
            .rank()
            .cmp(&raw[b].2.rank())
            .then(raw[a].0.cmp(&raw[b].0))
    });
    let mut odo_of = vec![0usize; raw.len()];
    for (k, &oi) in order.iter().enumerate() {
        odo_of[oi] = k;
    }
    raw.iter()
        .enumerate()
        .map(|(i, &(off, len, kind))| Occ {
            off,
            len,
            kind,
            odo: odo_of[i],
        })
        .collect()
}

pub fn expansion_total(occ: &[Occ], ctx: &Ctx) -> u64 {
    let mut total: u64 = 1;
    for o in occ {
        total = total.saturating_mul(exp_len(o.kind, ctx) as u64);
    }
    total
}

/// Enumerate expanded strings in the original's exact order.
pub fn enumerate<F: FnMut(&[u8])>(
    s: &[u8],
    occ: &[Occ],
    ctx: &Ctx,
    limit: u64,
    scratch: &mut Vec<u8>,
    mut f: F,
) {
    if occ.is_empty() {
        if limit == 0 {
            return;
        }
        scratch.clear();
        util::push_bytes(scratch, s);
        util::strip_spaces_and_apostrophes(scratch);
        f(scratch);
        return;
    }
    let lens: Vec<usize> = occ.iter().map(|o| exp_len(o.kind, ctx)).collect();
    if lens.contains(&0) {
        return;
    }
    let mut idx = vec![0usize; occ.len()];
    for _ in 0..limit {
        scratch.clear();
        let mut pos = 0usize;
        for o in occ.iter() {
            util::push_bytes(scratch, &s[pos..o.off]);
            write_exp(o.kind, idx[o.odo], ctx, scratch);
            pos = o.off + o.len;
        }
        util::push_bytes(scratch, &s[pos..]);
        util::strip_spaces_and_apostrophes(scratch);
        f(scratch);
        let mut k = occ.len();
        while k > 0 {
            k -= 1;
            let oi = occ[k].odo;
            idx[oi] += 1;
            if idx[oi] < lens[oi] {
                break;
            }
            idx[oi] = 0;
        }
    }
}

/// Locate the single token the original's `format` loop would expand next:
/// the one with the lowest branch rank, breaking ties by leftmost offset.
/// This is exactly the branch that `while (e =~ /…/)` selects each round.
pub fn first_branch(s: &[u8]) -> Option<(usize, usize, ExpKind)> {
    let mut best: Option<(u8, usize, usize, ExpKind)> = None;
    let mut i = 0usize;
    while i < s.len() {
        if s[i] == b'%' {
            let mut matched = false;
            for (tok, kind) in EXP_TOKENS {
                if s[i..].starts_with(tok) {
                    let rank = kind.rank();
                    let better = match best {
                        None => true,
                        Some((br, bo, _, _)) => (rank, i) < (br, bo),
                    };
                    if better {
                        best = Some((rank, i, tok.len(), kind));
                    }
                    i += tok.len();
                    matched = true;
                    break;
                }
            }
            if matched {
                continue;
            }
        }
        i += 1;
    }
    best.map(|(_, off, len, k)| (off, len, k))
}

/// The original's expansion loop, reproduced literally: repeatedly expand the
/// next branch token across the whole array, one round at a time. Fragments
/// produced by one round can reassemble a token for a later round (this is the
/// quirky `-F '%givenname'` case), so this cannot be reduced to a fixed-order
/// odometer. Only custom formats take this path.
pub fn expand_strict(mut arr: Vec<Vec<u8>>, ctx: &Ctx) -> Vec<Vec<u8>> {
    loop {
        let mut next: Vec<Vec<u8>> = Vec::with_capacity(arr.len().saturating_mul(2));
        let mut expanded = false;
        for e in arr {
            match first_branch(&e) {
                Some((off, len, kind)) => {
                    expanded = true;
                    let n = exp_len(kind, ctx);
                    for j in 0..n {
                        let mut s = Vec::with_capacity(e.len() + 8);
                        s.extend_from_slice(&e[..off]);
                        write_exp(kind, j, ctx, &mut s);
                        s.extend_from_slice(&e[off + len..]);
                        next.push(s);
                    }
                }
                // Finished entries move to the next round instead of being
                // cloned once per remaining expansion round.
                None => next.push(e),
            }
        }
        if !expanded {
            return next;
        }
        arr = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::names::{NameRef, Person};

    fn person<'a>(cols: [&'a str; 6]) -> Person<'a> {
        cols.map(|s| {
            if s.is_empty() {
                None
            } else {
                Some(s.as_bytes())
            }
        })
    }

    #[test]
    fn field_props_classifies() {
        let props = |c: [&str; 6]| field_props(&NameRef::from_person(&person(c)));
        assert_eq!(props(["anna", "key", "", "", "", ""]), (true, true));
        assert_eq!(props(["a", "k", "b", "", "", ""]), (true, true));
        // Uppercase forces the folding path.
        assert_eq!(props(["Anna", "key", "", "", "", ""]), (false, true));
        // Non-ASCII forces the folding path.
        assert_eq!(props(["é", "key", "", "", "", ""]), (false, true));
        // A percent sign may be an expansion token inside a supplied name.
        assert_eq!(props(["ann%d", "key", "", "", "", ""]), (false, true));
        // A space or apostrophe is `plain` but not `clean`.
        assert_eq!(props(["mary jane", "key", "", "", "", ""]), (true, false));
        assert_eq!(props(["o'brien", "key", "", "", "", ""]), (true, false));
    }

    /// The raw resolver is only used for `plain` names, where it must produce
    /// exactly what the general `downcase(upcase_it(x))` resolver produces.
    #[test]
    fn raw_lower_matches_folding_for_plain_names() {
        let cases = [
            ["anna", "key", "", "", "", ""],
            ["a", "k", "b", "", "", ""],
            ["mary", "jane", "o'brien", "", "", ""],
            ["anna", "key", "", "K", "", ""],
        ];
        let plugins = static_plugins();
        for c in cases {
            let p = person(c);
            let name = NameRef::from_person(&p);
            assert!(field_props(&name).0, "{c:?} should be plain");
            for p in &plugins {
                if let PluginKind::Format { segs, lits } = &p.kind {
                    let mut folded = Vec::new();
                    let mut raw = Vec::new();
                    resolve_string_lower(segs, lits, &name, &mut folded);
                    resolve_string_lower_raw(segs, lits, &name, &mut raw);
                    assert_eq!(folded, raw, "{} {c:?}", p.name);
                }
            }
        }
    }
}
