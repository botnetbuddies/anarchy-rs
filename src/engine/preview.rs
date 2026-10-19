//! Collect format examples and recognition candidates.
use super::Scratch;
use crate::fmt::{self, Ctx, Plugin, PluginKind};
use crate::names::NameFields;
use crate::util;

/// The three shapes a plugin's `generate` can return.
pub enum GenRet {
    Str(Vec<u8>),
    Arr(Vec<Vec<u8>>),
    Nil,
}

/// Reproduce a single `generate(n)` call (used by `--list-formats` and
/// `--recognise`).
pub fn all_outputs<N: NameFields + ?Sized>(
    p: &Plugin,
    name: &N,
    ctx: &Ctx,
    substitute: bool,
    sc: &mut Scratch,
) -> GenRet {
    match &p.kind {
        PluginKind::TruncFirstLast8 => match (name.firstname(), name.lastname()) {
            (Some(f), Some(l)) => {
                sc.chars.clear();
                sc.chars.extend_from_slice(f);
                sc.chars.extend_from_slice(l);
                let end = util::nth_char_boundary(&sc.chars, 8);
                GenRet::Str(sc.chars[..end].to_vec())
            }
            _ => GenRet::Nil,
        },
        PluginKind::TruncFirst4Last4 => match (name.firstname(), name.lastname()) {
            (Some(f), Some(l)) => {
                let fe = util::nth_char_boundary(f, 4);
                let le = util::nth_char_boundary(l, 4);
                let mut v = Vec::with_capacity(fe + le);
                v.extend_from_slice(&f[..fe]);
                v.extend_from_slice(&l[..le]);
                GenRet::Str(v)
            }
            _ => GenRet::Nil,
        },
        PluginKind::Format { segs, lits } => {
            if let Some(source) = &p.sequential_source {
                fmt::resolve_sequential(source, name, &mut sc.resolved, &mut sc.tmp);
            } else {
                fmt::resolve_string(segs, lits, name, &mut sc.resolved);
            }
            let occ = fmt::tokenize(&sc.resolved);
            if !occ.is_empty() && !substitute {
                return GenRet::Nil;
            }
            let mut out: Vec<Vec<u8>> = Vec::new();
            if p.strict {
                let arr = fmt::expand_strict(vec![std::mem::take(&mut sc.resolved)], ctx);
                for mut e in arr {
                    util::strip_spaces_and_apostrophes(&mut e);
                    out.push(e);
                }
            } else {
                let total = fmt::expansion_total(&occ, ctx);
                fmt::enumerate(&sc.resolved, &occ, ctx, total, &mut sc.tmp, |s| {
                    out.push(s.to_vec())
                });
            }
            GenRet::Arr(out)
        }
    }
}
