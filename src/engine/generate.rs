//! Generate candidates for fixed records and automatic name products.
use super::{GenOpts, Scratch};
use crate::fmt::{self, Ctx, Plugin, PluginKind};
use crate::names::{NameFields, NameRef, Person};
use crate::output::{pp_line, Emitter, SinkTarget};
use crate::util;

/// Generate every candidate for one person into the emitter.
pub fn gen_person<N: NameFields + ?Sized, S: SinkTarget>(
    plugins: &[Plugin],
    name: &N,
    ctx: &Ctx,
    opts: &GenOpts,
    em: &mut Emitter<S>,
    sc: &mut Scratch,
    present_mask: u8,
) -> Result<(), String> {
    // One scan per person replaces a scan of every field in every template.
    let (plain, clean) = fmt::field_props(name);
    let ascii_fields = if plain && clean && opts.case_insensitive && !opts.substitute {
        Some([
            name.firstname().unwrap_or_default(),
            name.lastname().unwrap_or_default(),
            name.firstinitial().unwrap_or_default(),
            name.lastinitial().unwrap_or_default(),
            name.middlename().unwrap_or_default(),
            name.middleinitial().unwrap_or_default(),
            b".",
            b"",
        ])
    } else {
        None
    };
    for p in plugins {
        if !opts.substitute && (present_mask & p.required_mask) != p.required_mask {
            if opts.verbose {
                em.comment(&p.name, None);
            }
            continue;
        }
        if let (Some(fields), Some(parts)) = (&ascii_fields, p.ascii_parts) {
            if opts.verbose {
                em.comment(&p.name, Some(1));
            }
            if p.ascii_duplicate {
                continue;
            }
            sc.resolved.clear();
            for i in parts {
                util::push_bytes(&mut sc.resolved, fields[i as usize]);
            }
            em.emit_known_lower(&sc.resolved);
            continue;
        }
        match &p.kind {
            PluginKind::TruncFirstLast8 => {
                if opts.verbose {
                    em.comment(&p.name, None);
                }
                if let (Some(f), Some(l)) = (name.firstname(), name.lastname()) {
                    sc.chars.clear();
                    sc.chars.extend_from_slice(f);
                    sc.chars.extend_from_slice(l);
                    let end = util::nth_char_boundary(&sc.chars, 8);
                    sc.tmp.clear();
                    sc.tmp.extend_from_slice(&sc.chars[..end]);
                    if plain {
                        em.emit_known_lower(&sc.tmp);
                    } else {
                        em.emit(&sc.tmp);
                    }
                }
            }
            PluginKind::TruncFirst4Last4 => {
                if opts.verbose {
                    em.comment(&p.name, None);
                }
                if let (Some(f), Some(l)) = (name.firstname(), name.lastname()) {
                    let fe = util::nth_char_boundary(f, 4);
                    let le = util::nth_char_boundary(l, 4);
                    sc.tmp.clear();
                    sc.tmp.extend_from_slice(&f[..fe]);
                    sc.tmp.extend_from_slice(&l[..le]);
                    if plain {
                        em.emit_known_lower(&sc.tmp);
                    } else {
                        em.emit(&sc.tmp);
                    }
                }
            }
            PluginKind::Format { segs, lits } => {
                // The built-in templates have no letters in their literals, so
                // a case-insensitive run can resolve straight to final form and
                // skip `Emitter::emit`'s second case-folding pass.
                let lower_ready = opts.case_insensitive && !p.strict;
                if let Some(source) = &p.sequential_source {
                    fmt::resolve_sequential(source, name, &mut sc.resolved, &mut sc.tmp);
                } else if lower_ready {
                    if plain {
                        fmt::resolve_string_lower_raw(segs, lits, name, &mut sc.resolved);
                    } else {
                        fmt::resolve_string_lower(segs, lits, name, &mut sc.resolved);
                    }
                } else {
                    fmt::resolve_string(segs, lits, name, &mut sc.resolved);
                }
                // With substitutions off (the default) a template that still
                // contains a token is skipped outright.
                let occ = if opts.substitute {
                    fmt::tokenize(&sc.resolved)
                } else if fmt::has_token(&sc.resolved) {
                    if opts.verbose {
                        em.comment(&p.name, None);
                    }
                    continue;
                } else {
                    Vec::new()
                };
                if opts.substitute {
                    if occ.iter().any(|o| o.kind.is_given()) && ctx.given().last_nil {
                        return Err(
                            "given-names wordlist: last line has no trailing newline; the original raises here"
                                .into(),
                        );
                    }
                    if occ.iter().any(|o| o.kind.is_surname()) && ctx.family().last_nil {
                        return Err(
                            "family-names wordlist: last line has no trailing newline; the original raises here"
                                .into(),
                        );
                    }
                }
                if p.strict {
                    let arr = fmt::expand_strict(vec![std::mem::take(&mut sc.resolved)], ctx);
                    if opts.verbose {
                        em.comment(&p.name, Some(arr.len() as u64));
                    }
                    let limit = opts.limit(arr.len() as u64) as usize;
                    for e in arr.iter().take(limit) {
                        sc.tmp.clear();
                        sc.tmp.extend_from_slice(e);
                        util::strip_spaces_and_apostrophes(&mut sc.tmp);
                        em.emit(&sc.tmp);
                    }
                } else {
                    let total = fmt::expansion_total(&occ, ctx);
                    if opts.verbose {
                        em.comment(&p.name, Some(total));
                    }
                    let limit = opts.limit(total);
                    if occ.is_empty() && limit > 0 {
                        if !clean {
                            util::strip_spaces_and_apostrophes(&mut sc.resolved);
                        }
                        if plain {
                            em.emit_known_lower(&sc.resolved);
                        } else if lower_ready {
                            em.emit_lower(&sc.resolved);
                        } else {
                            em.emit(&sc.resolved);
                        }
                    } else if lower_ready {
                        fmt::enumerate(&sc.resolved, &occ, ctx, limit, &mut sc.tmp, |s| {
                            em.emit_lower(s)
                        });
                    } else {
                        fmt::enumerate(&sc.resolved, &occ, ctx, limit, &mut sc.tmp, |s| em.emit(s));
                    }
                }
            }
        }
    }
    Ok(())
}

#[inline]
fn gen_named<N: NameFields + ?Sized, S: SinkTarget>(
    plugins: &[Plugin],
    name: &N,
    ctx: &Ctx,
    opts: &GenOpts,
    em: &mut Emitter<S>,
    sc: &mut Scratch,
    present_mask: u8,
) -> Result<(), String> {
    if opts.verbose {
        em.comment_raw(&pp_line(name));
    }
    gen_person(plugins, name, ctx, opts, em, sc, present_mask)
}

pub fn gen_people<'a, S: SinkTarget>(
    people: &[Person<'a>],
    plugins: &[Plugin],
    ctx: &Ctx,
    opts: &GenOpts,
    em: &mut Emitter<S>,
    sc: &mut Scratch,
) -> Result<(), String> {
    for p in people {
        let name = NameRef::from_person(p);
        gen_named(plugins, &name, ctx, opts, em, sc, name.present_mask)?;
    }
    Ok(())
}

/// The two `--auto` dictionaries.
#[derive(Clone, Copy)]
pub struct AutoNames<'a> {
    pub given: &'a [Option<Vec<u8>>],
    pub family: &'a [Option<Vec<u8>>],
}

/// Generate the `--auto` cross-product for a slice of the family-name range.
pub fn gen_auto<'a, S: SinkTarget>(
    head: &[Person<'a>],
    names: AutoNames<'a>,
    plugins: &[Plugin],
    ctx: &Ctx,
    opts: &GenOpts,
    em: &mut Emitter<S>,
    sc: &mut Scratch,
) -> Result<(), String> {
    for p in head {
        let name = NameRef::from_person(p);
        gen_named(plugins, &name, ctx, opts, em, sc, name.present_mask)?;
    }
    for fam in names.family {
        for giv in names.given {
            let name = NameRef::from_first_last(giv.as_deref(), fam.as_deref());
            gen_named(plugins, &name, ctx, opts, em, sc, name.present_mask)?;
        }
    }
    Ok(())
}
