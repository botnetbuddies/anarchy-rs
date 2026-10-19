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

#[derive(Clone, Debug)]
pub struct Prep {
    pub title: Vec<u8>,
    pub fold: Vec<u8>,
    pub lower: Vec<u8>,
}

impl Default for Prep {
    fn default() -> Self {
        Prep {
            title: Vec::with_capacity(64),
            fold: Vec::with_capacity(64),
            lower: Vec::with_capacity(64),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PreparedName<'a> {
    pub lower: [Option<&'a [u8]>; 6],
    pub title: [Option<&'a [u8]>; 6],
    pub fold: [Option<&'a [u8]>; 6],
    pub present: u16,
    pub plain: bool,
    pub clean: bool,
    pub has_pct: bool,
}

#[derive(Clone, Copy, Debug)]
pub enum Mode {
    Normal,
    LowerFold,
}

#[rustfmt::skip]
pub const MISSING: [[&[u8]; 2]; 6] = [
    [b"%givenname", b"%Givenname"], // 0 first
    [b"%givenname", b"%Givenname"], // 1 middle
    [b"%surname",   b"%Surname"],   // 2 last
    [b"%a-z",       b"%A-Z"],       // 3 firstinit
    [b"%a-z",       b"%A-Z"],       // 4 middleinit
    [b"%a-z",       b"%A-Z"],       // 5 lastinit
];

#[inline(always)]
pub fn seg_slot(seg: Seg) -> i32 {
    match seg {
        Seg::FirstUpper | Seg::FirstLower => 0,
        Seg::MiddleUpper | Seg::MiddleLower => 1,
        Seg::LastUpper | Seg::LastLower => 2,
        Seg::FirstInitUpper | Seg::FirstInitLower => 3,
        Seg::MiddleInitUpper | Seg::MiddleInitLower => 4,
        Seg::LastInitUpper | Seg::LastInitLower => 5,
        _ => -1,
    }
}

/// Build the per-person prepared field table.
///
/// All three casings of every field are computed once per person (rather than
/// once per person *per plugin*), and `present` lets templates whose required
/// fields are missing skip resolution entirely. The transformed byte forms
/// live in the caller's [`Prep`] (reused across people, so after warmup this
/// is allocation-free); the two-phase push-then-reference scheme keeps the
/// returned borrows valid regardless of reallocation during the pushes.
///
/// `need_title` is false when case folding is on and no strict plugin is
/// selected, in which case `Mode::Normal` resolution never runs and the
/// title-cased variants are skipped (aliased to `fold`, never read).
pub fn prepare<'p, N: NameFields + ?Sized>(
    name: &'p N,
    prep: &'p mut Prep,
    need_title: bool,
) -> PreparedName<'p> {
    let fields = [
        name.firstname(),
        name.middlename(),
        name.lastname(),
        name.firstinitial(),
        name.middleinitial(),
        name.lastinitial(),
    ];
    // Which slots need a transformed (owned) form; lengths recorded in phase 1
    // so phase 2 can slice without recomputing Unicode casing lengths.
    let mut lower_len = [0usize; 6];
    let mut title_len = [0usize; 6];
    let mut fold_len = [0usize; 6];
    let mut present = 0u16;
    let mut plain = true;
    let mut clean = true;
    let mut has_pct = false;

    prep.title.clear();
    prep.lower.clear();
    prep.fold.clear();

    // Phase 1: classify and append every transformed form.
    for (i, f) in fields.iter().enumerate() {
        let Some(f) = *f else { continue };
        if f.is_empty() {
            continue;
        }
        present |= 1u16 << i;
        let mut ascii = true;
        let mut lower_ok = true;
        for &b in f {
            if b >= 0x80 {
                ascii = false;
                plain = false;
            } else if b.is_ascii_uppercase() {
                lower_ok = false;
                plain = false;
            }
            if b == b' ' || b == b'\'' {
                clean = false;
            }
            if b == b'%' {
                has_pct = true;
            }
        }
        if lower_ok && ascii {
            // Already lowercase ASCII: borrow, only the title form is derived.
            if need_title {
                let before = prep.title.len();
                prep.title.push(f[0].to_ascii_uppercase());
                util::push_bytes(&mut prep.title, &f[1..]);
                title_len[i] = prep.title.len() - before;
            }
        } else if ascii {
            // ASCII with uppercase: lower = downcase(f); fold == lower; title.
            let before = prep.lower.len();
            util::downcase_into(f, &mut prep.lower);
            lower_len[i] = prep.lower.len() - before;
            if need_title {
                let before = prep.title.len();
                util::upcase_it_into(f, &mut prep.title);
                title_len[i] = prep.title.len() - before;
            }
        } else {
            // Non-ASCII: all three forms are distinct byte sequences.
            let before = prep.lower.len();
            util::downcase_into(f, &mut prep.lower);
            lower_len[i] = prep.lower.len() - before;
            if need_title {
                let before = prep.title.len();
                util::upcase_it_into(f, &mut prep.title);
                title_len[i] = prep.title.len() - before;
            }
            let before = prep.fold.len();
            util::upcase_then_downcase_into(f, &mut prep.fold);
            fold_len[i] = prep.fold.len() - before;
        }
    }

    // Phase 2: all pushes are done, so borrows are stable.
    let mut lower: [Option<&[u8]>; 6] = [None; 6];
    let mut title: [Option<&[u8]>; 6] = [None; 6];
    let mut fold: [Option<&[u8]>; 6] = [None; 6];
    let mut loff = 0usize;
    let mut toff = 0usize;
    let mut foff = 0usize;
    for (i, f) in fields.iter().enumerate() {
        if (present & (1u16 << i)) == 0 {
            continue;
        }
        let f = *f;
        if lower_len[i] > 0 {
            lower[i] = Some(&prep.lower[loff..loff + lower_len[i]]);
            loff += lower_len[i];
        } else {
            lower[i] = f;
        }
        if title_len[i] > 0 {
            title[i] = Some(&prep.title[toff..toff + title_len[i]]);
            toff += title_len[i];
        } else {
            // Either plain (title = upper-first of a lowercase field, which is
            // only read when need_title, and plain fields do push that form) or
            // need_title == false (title is never read in LowerFold mode).
            title[i] = lower[i];
        }
        if fold_len[i] > 0 {
            fold[i] = Some(&prep.fold[foff..foff + fold_len[i]]);
            foff += fold_len[i];
        } else if lower_len[i] > 0 {
            fold[i] = lower[i]; // ASCII: downcase(upcase_it(x)) == downcase(x)
        } else {
            fold[i] = f; // plain: already lowercase ASCII
        }
    }

    PreparedName {
        lower,
        title,
        fold,
        present,
        plain,
        clean,
        has_pct,
    }
}

/// Unrolled segment resolution over a prepared name.
///
/// The 24 static templates have at most three segments; matching on the count
/// lets LLVM fully unroll the hot loops and drop the per-iteration bounds
/// checks. Missing fields fall back to the static `MISSING` token table.
#[inline]
pub fn resolve(segs: &[Seg], lits: &[u8], pn: &PreparedName, mode: Mode, out: &mut Vec<u8>) {
    out.clear();
    match segs.len() {
        1 => push_seg(segs[0], lits, pn, mode, out),
        2 => {
            push_seg(segs[0], lits, pn, mode, out);
            push_seg(segs[1], lits, pn, mode, out);
        }
        3 => {
            push_seg(segs[0], lits, pn, mode, out);
            push_seg(segs[1], lits, pn, mode, out);
            push_seg(segs[2], lits, pn, mode, out);
        }
        n => {
            for &seg in segs {
                push_seg(seg, lits, pn, mode, out);
            }
            let _ = n;
        }
    }
}

#[inline(always)]
fn push_seg(seg: Seg, lits: &[u8], pn: &PreparedName, mode: Mode, out: &mut Vec<u8>) {
    match seg {
        Seg::Lit(a, b) => util::push_bytes(out, &lits[a as usize..b as usize]),
        Seg::Tok(k) => util::push_bytes(out, k.text()),
        other => {
            let slot = seg_slot(other) as usize;
            let upper = matches!(
                other,
                Seg::FirstUpper
                    | Seg::MiddleUpper
                    | Seg::LastUpper
                    | Seg::FirstInitUpper
                    | Seg::MiddleInitUpper
                    | Seg::LastInitUpper
            );
            let table = match mode {
                Mode::Normal => {
                    if upper {
                        &pn.title
                    } else {
                        &pn.lower
                    }
                }
                Mode::LowerFold => {
                    if upper {
                        &pn.fold
                    } else {
                        &pn.lower
                    }
                }
            };
            match table[slot] {
                Some(v) => util::push_bytes(out, v),
                None => util::push_bytes(out, MISSING[slot][upper as usize]),
            }
        }
    }
}

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

#[derive(Clone, Copy, Debug)]
pub struct TemplateInfo {
    pub required: u16,
    pub has_tok: bool,
    pub lit_pct: bool,
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
    pub required: u16,
    pub has_tok: bool,
    pub lit_pct: bool,
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

fn template_info(segs: &[Seg], lits: &[u8]) -> TemplateInfo {
    let mut required = 0u16;
    for seg in segs {
        let s = seg_slot(*seg);
        if s >= 0 {
            required |= 1u16 << s;
        }
        if let Seg::Tok(_) = *seg {
            // presence tracked via has_tok
        }
    }
    let mut has_tok = false;
    let mut lit_pct = false;
    for seg in segs {
        if let Seg::Tok(_) = *seg {
            has_tok = true;
            break;
        }
    }
    for &b in lits {
        if b == b'%' {
            lit_pct = true;
            break;
        }
    }
    TemplateInfo {
        required,
        has_tok,
        lit_pct,
    }
}

fn format_plugin(name: &str, raw: &str, abk: bool, strict: bool) -> Plugin {
    let expanded = if abk {
        abk_to_format(raw)
    } else {
        raw.to_string()
    };
    let (segs, lits) = parse_template(&expanded);
    let info = template_info(&segs, &lits);
    Plugin {
        name: name.to_string(),
        kind: PluginKind::Format { segs, lits },
        strict,
        required: info.required,
        has_tok: info.has_tok,
        lit_pct: info.lit_pct,
    }
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
            required: (1u16 << 0) | (1u16 << 2),
            has_tok: false,
            lit_pct: false,
        },
        Plugin {
            name: "first[4]last[4]".to_string(),
            kind: PluginKind::TruncFirst4Last4,
            strict: false,
            required: (1u16 << 0) | (1u16 << 2),
            has_tok: false,
            lit_pct: false,
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

/// Push a field value into `out`, upper-casing its first character when
/// `upper`, or `missing` when the field is absent.
///
/// `MISSING[slot][upper]` is the equivalent table used by [`resolve`]; this
/// function is kept for the cold `estimate`/`all_outputs` paths.
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

    fn person(cols: [&str; 6]) -> Person {
        cols.map(|s| {
            if s.is_empty() {
                None
            } else {
                Some(s.as_bytes().to_vec())
            }
        })
    }

    #[test]
    fn prepare_classifies() {
        let props = |c: [&str; 6]| {
            let p = person(c);
            let name = NameRef::from_person(&p);
            let mut prep = Prep::default();
            let pn = prepare(&name, &mut prep, true);
            (pn.plain, pn.clean)
        };
        assert_eq!(props(["anna", "key", "", "", "", ""]), (true, true));
        assert_eq!(props(["a", "k", "b", "", "", ""]), (true, true));
        // Uppercase forces the folding path.
        assert_eq!(props(["Anna", "key", "", "", "", ""]), (false, true));
        // Non-ASCII forces the folding path.
        assert_eq!(props(["é", "key", "", "", "", ""]), (false, true));
        // A space or apostrophe is `plain` but not `clean`.
        assert_eq!(props(["mary jane", "key", "", "", "", ""]), (true, false));
        assert_eq!(props(["o'brien", "key", "", "", "", ""]), (true, false));
    }

    fn ref_val(v: Option<&[u8]>, out: &mut Vec<u8>, upper: bool, missing: &[u8]) {
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

    fn ref_val_lower(v: Option<&[u8]>, out: &mut Vec<u8>, missing: &[u8]) {
        match v {
            Some(s) => {
                if !util::upcase_then_downcase_into(s, out) {
                    util::push_bytes(out, missing);
                }
            }
            None => util::push_bytes(out, missing),
        }
    }

    /// Reference implementation, transcribed from the pre-`prepare` resolvers.
    fn ref_resolve<N: NameFields + ?Sized>(
        segs: &[Seg],
        lits: &[u8],
        name: &N,
        mode: Mode,
        out: &mut Vec<u8>,
    ) {
        out.clear();
        let upper_val = |v: Option<&[u8]>, out: &mut Vec<u8>, missing: &[u8]| match mode {
            Mode::Normal => ref_val(v, out, true, missing),
            Mode::LowerFold => ref_val_lower(v, out, missing),
        };
        for seg in segs {
            match *seg {
                Seg::Lit(a, b) => util::push_bytes(out, &lits[a as usize..b as usize]),
                Seg::FirstUpper => upper_val(name.firstname(), out, b"%Givenname"),
                Seg::MiddleUpper => upper_val(name.middlename(), out, b"%Givenname"),
                Seg::LastUpper => upper_val(name.lastname(), out, b"%Surname"),
                Seg::FirstLower => ref_val(name.firstname(), out, false, b"%givenname"),
                Seg::MiddleLower => ref_val(name.middlename(), out, false, b"%givenname"),
                Seg::LastLower => ref_val(name.lastname(), out, false, b"%surname"),
                Seg::FirstInitUpper => upper_val(name.firstinitial(), out, b"%A-Z"),
                Seg::MiddleInitUpper => upper_val(name.middleinitial(), out, b"%A-Z"),
                Seg::LastInitUpper => upper_val(name.lastinitial(), out, b"%A-Z"),
                Seg::FirstInitLower => ref_val(name.firstinitial(), out, false, b"%a-z"),
                Seg::MiddleInitLower => ref_val(name.middleinitial(), out, false, b"%a-z"),
                Seg::LastInitLower => ref_val(name.lastinitial(), out, false, b"%a-z"),
                Seg::Tok(k) => util::push_bytes(out, k.text()),
            }
        }
    }

    /// `prepare` + `resolve` must be byte-identical to the old resolvers for
    /// both modes and every static template, including missing fields and
    /// non-ASCII / mixed-case values.
    #[test]
    fn resolve_matches_reference() {
        let cases = [
            ["anna", "key", "", "", "", ""],
            ["a", "k", "b", "", "", ""],
            ["Anna", "Key", "A", "K", "Boom", "M"],
            ["anna", "key", "", "K", "", ""],
            ["anna", "key", "", "", "", "M"],
            ["mary jane", "key", "", "", "", ""],
            ["o'brien", "key", "", "", "", ""],
            ["", "key", "", "", "", ""],
            ["é", "straße", "", "", "", ""],
            ["anna", "", "", "", "boom", ""],
        ];
        for c in cases {
            let p = person(c);
            let name = NameRef::from_person(&p);
            let mut prep = Prep::default();
            let pn = prepare(&name, &mut prep, true);
            for plugin in &static_plugins() {
                let PluginKind::Format { segs, lits } = &plugin.kind else {
                    continue;
                };
                for mode in [Mode::Normal, Mode::LowerFold] {
                    let mut got = Vec::new();
                    resolve(segs, lits, &pn, mode, &mut got);
                    let mut want = Vec::new();
                    ref_resolve(segs, lits, &name, mode, &mut want);
                    assert_eq!(got, want, "{} {c:?} {mode:?}", plugin.name);
                }
            }
        }
    }

    /// With `need_title == false` the title forms are skipped; they must not be
    /// read, and the `fold`/`lower` forms must still be correct.
    #[test]
    fn resolve_without_title_is_still_correct() {
        let cases = [
            ["Anna", "Key", "A", "K", "Boom", "M"],
            ["é", "straße", "", "", "", ""],
        ];
        for c in cases {
            let p = person(c);
            let name = NameRef::from_person(&p);
            let mut prep = Prep::default();
            let pn = prepare(&name, &mut prep, false);
            for plugin in &static_plugins() {
                let PluginKind::Format { segs, lits } = &plugin.kind else {
                    continue;
                };
                let mut got = Vec::new();
                resolve(segs, lits, &pn, Mode::LowerFold, &mut got);
                let mut want = Vec::new();
                ref_resolve(segs, lits, &name, Mode::LowerFold, &mut want);
                assert_eq!(got, want, "{} {c:?}", plugin.name);
            }
        }
    }
}
