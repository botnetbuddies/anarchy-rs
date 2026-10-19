//! Candidate-count estimates for capacity and worker selection.
use super::{AutoNames, GenOpts, Scratch};
use crate::fmt::{self, Ctx, Plugin, PluginKind};
use crate::names::{NameFields, NameRef, Person};

/// Per-person candidate count for one representative name.
fn per_person_cost<N: NameFields + ?Sized>(
    name: &N,
    plugins: &[Plugin],
    ctx: &Ctx,
    opts: &GenOpts,
) -> u64 {
    let mut sc = Scratch::new();
    let mut per_person: u64 = 0;
    let (plain, clean) = fmt::field_props(name);
    for p in plugins {
        if p.ascii_duplicate && plain && clean && opts.case_insensitive && !opts.substitute {
            continue;
        }
        match &p.kind {
            PluginKind::TruncFirstLast8 | PluginKind::TruncFirst4Last4 => {
                per_person = per_person.saturating_add(1);
            }
            PluginKind::Format { segs, lits } => {
                fmt::resolve_string(segs, lits, name, &mut sc.resolved);
                let occ = fmt::tokenize(&sc.resolved);
                if !occ.is_empty() && !opts.substitute {
                    continue;
                }
                let total = fmt::expansion_total(&occ, ctx);
                per_person = per_person.saturating_add(total.min(opts.limit(total)));
            }
        }
    }
    per_person
}

/// Rough upper bound on the number of generated candidates.
pub fn estimate<'a>(people: &[Person<'a>], plugins: &[Plugin], ctx: &Ctx, opts: &GenOpts) -> u64 {
    let n_people = people.len().max(1) as u64;
    let sample: Person<'a> = people
        .iter()
        .find(|p| p.iter().any(|x| x.is_some()))
        .copied()
        .unwrap_or_default();
    let name = NameRef::from_person(&sample);
    per_person_cost(&name, plugins, ctx, opts).saturating_mul(n_people)
}

/// Estimate for the streamed `--auto` cross-product.
pub fn estimate_auto<'a>(
    head: &[Person<'a>],
    names: AutoNames<'a>,
    plugins: &[Plugin],
    ctx: &Ctx,
    opts: &GenOpts,
) -> u64 {
    let empty = NameRef::default();
    let head_cost = per_person_cost(&empty, plugins, ctx, opts);
    let sample = NameRef::from_first_last(
        names
            .given
            .iter()
            .find_map(|x| x.as_deref().filter(|s| !s.is_empty())),
        names
            .family
            .iter()
            .find_map(|x| x.as_deref().filter(|s| !s.is_empty())),
    );
    let cross_cost = per_person_cost(&sample, plugins, ctx, opts);
    let cross = (names.given.len() as u64)
        .saturating_mul(names.family.len() as u64)
        .saturating_mul(cross_cost);
    (head.len() as u64)
        .saturating_mul(head_cost)
        .saturating_add(cross)
}
