//! Resolve fields to bytes, preserving Ruby casing and replacement order.
use super::{template::TOKENS, Seg};
use crate::names::NameFields;
use crate::util;

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
/// already in final form, so [`crate::output::Emitter::emit_lower`] can insert
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
