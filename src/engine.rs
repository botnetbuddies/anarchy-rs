//! Candidate generation, global de-duplication, and buffered output.

use std::hash::Hasher;

use rustc_hash::FxHasher;

use crate::fmt::{self, Ctx, Plugin, PluginKind};
use crate::names::{NameFields, NameRef, Person};
use crate::util;

#[inline]
fn fxhash(bytes: &[u8]) -> u64 {
    let mut h = FxHasher::default();
    h.write(bytes);
    h.finish()
}

/// One output line: byte range in `out` plus the low 32 bits of its hash.
///
/// The low half of the hash determines the bucket (`hash & mask`) *and* acts as
/// the probe tag, so growth never has to re-read and re-hash the line bytes.
#[derive(Clone, Copy)]
struct Entry {
    off: u32,
    len: u32,
    tag: u32,
}

/// De-duplication set plus ordered output for one worker.
///
/// Every emitted line's bytes are stored once, contiguously, in `out`, followed
/// by its `\n`; `entries` records `(offset, len, tag)` for each line in emission
/// order (comment lines included). `len` excludes the terminator, so duplicate
/// comparisons and merges see exactly the original line. Candidate lines are
/// additionally threaded through a hash-bucket chain (`table`) so that a
/// repeated candidate costs one hash, one byte comparison and *no allocation* —
/// the profiler showed malloc/free dominating a straightforward
/// `HashSet<Vec<u8>>` implementation.
///
/// Workers never share a `Sink`, so no locking is needed on the hot path; the
/// parallel path merges the per-worker sinks at the end.
pub struct Sink {
    out: Vec<u8>,
    entries: Vec<Entry>,
    is_comment: Vec<bool>,
    /// Open-addressed table of `(hash tag, entry index + 1)`; `index + 1 == 0`
    /// marks an empty slot. Storing the tag inline means a probe touches one
    /// cache line instead of two (table + separate hash array), which the
    /// profiler showed to be worth it.
    table: Vec<(u32, u32)>,
    mask: usize,
    filled: usize,
}

impl Sink {
    pub fn new() -> Sink {
        Sink::with_capacity(128)
    }

    /// `expected` is a hint for the number of *unique* lines. Callers cap it
    /// (see [`worker_hint`]) so a bad estimate cannot reserve gigabytes.
    pub fn with_capacity(expected: usize) -> Sink {
        let mut cap = 128usize;
        while cap.saturating_mul(7) / 10 < expected {
            match cap.checked_mul(2) {
                Some(c) => cap = c,
                None => break,
            }
        }
        Sink {
            out: Vec::with_capacity(expected.saturating_mul(12)),
            entries: Vec::with_capacity(expected),
            // Comments are only emitted in verbose mode; let this one grow.
            is_comment: Vec::with_capacity(expected.min(4096)),
            table: vec![(0u32, 0u32); cap],
            mask: cap - 1,
            filled: 0,
        }
    }

    /// Append a line that must not participate in de-duplication (a comment).
    #[inline]
    fn push_line(&mut self, line: &[u8]) {
        let off = self.out.len();
        debug_assert!(off <= u32::MAX as usize, "output exceeded 4 GiB");
        util::push_bytes(&mut self.out, line);
        self.out.push(b'\n');
        self.entries.push(Entry {
            off: off as u32,
            len: line.len() as u32,
            tag: 0,
        });
        self.is_comment.push(true);
    }

    #[inline]
    fn insert(&mut self, key: &[u8]) {
        if self.filled * 10 >= self.table.len() * 7 {
            self.grow();
        }
        let h = fxhash(key);
        let tag = h as u32;
        let mut i = (h as usize) & self.mask;
        loop {
            let (stored_tag, e) = self.table[i];
            if e == 0 {
                break;
            }
            if stored_tag == tag {
                let ent = self.entries[(e - 1) as usize];
                let off = ent.off as usize;
                if ent.len as usize == key.len() && self.out[off..off + key.len()] == *key {
                    return;
                }
            }
            i = (i + 1) & self.mask;
        }
        let off = self.out.len();
        debug_assert!(off <= u32::MAX as usize, "output exceeded 4 GiB");
        util::push_bytes(&mut self.out, key);
        self.out.push(b'\n');
        self.entries.push(Entry {
            off: off as u32,
            len: key.len() as u32,
            tag,
        });
        self.is_comment.push(false);
        self.table[i] = (tag, self.entries.len() as u32);
        self.filled += 1;
    }

    fn grow(&mut self) {
        let new_cap = self.table.len() * 2;
        let mut table = vec![(0u32, 0u32); new_cap];
        let mask = new_cap - 1;
        for (ei, ent) in self.entries.iter().enumerate() {
            if self.is_comment[ei] {
                continue;
            }
            // The stored tag is the low half of the hash, which is also what
            // the bucket index is derived from: no re-hash of the bytes.
            let mut i = (ent.tag as usize) & mask;
            while table[i].1 != 0 {
                i = (i + 1) & mask;
            }
            table[i] = (ent.tag, ei as u32 + 1);
        }
        self.table = table;
        self.mask = mask;
    }

    /// Merge another worker's lines into this sink, keeping only candidate lines
    /// not already present (comment lines are preserved verbatim). Used at the
    /// end of a parallel run.
    pub fn absorb(&mut self, other: Sink) {
        for (ei, ent) in other.entries.iter().enumerate() {
            let off = ent.off as usize;
            let line = &other.out[off..off + ent.len as usize];
            if other.is_comment[ei] {
                self.push_line(line);
            } else {
                self.insert(line);
            }
        }
    }

    /// The arena already contains each line followed by `\n`, so this is a move.
    pub fn finish(self) -> Vec<u8> {
        self.out
    }
}

/// Per-worker capacity hint for [`Sink::with_capacity`].
///
/// `total` is an upper bound on candidates (from [`estimate`]); only a fraction
/// of them are unique. The hint is deliberately conservative — see the comment
/// in the body — because oversizing the table costs more (cache, page faults)
/// than the handful of doublings it avoids.
pub fn worker_hint(total: u64, workers: usize) -> usize {
    // Candidates are de-duplicated, so only a fraction become unique lines.
    // The estimate is an upper bound on *candidates*, not on uniques, and
    // measurements show that reserving for it is a net loss once tables stop
    // fitting in cache. A small cap still avoids the first few doublings on
    // medium runs without inflating per-worker RSS.
    const MAX_HINT: u64 = 4096;
    (total / 3 / workers.max(1) as u64).min(MAX_HINT) as usize
}

pub struct GenOpts {
    pub substitute: bool,
    pub max_sub: i64,
    pub verbose: bool,
    pub case_insensitive: bool,
    pub suffix: Option<Vec<u8>>,
}

impl GenOpts {
    #[inline]
    fn limit(&self, total: u64) -> u64 {
        if self.max_sub < 0 {
            total
        } else {
            total.min(self.max_sub as u64 + 1)
        }
    }
}

/// Per-worker emitter: applies case folding and the suffix, then de-duplicates.
pub struct Emitter {
    sink: Sink,
    key: Vec<u8>,
    tmp: Vec<u8>,
    case_insensitive: bool,
    suffix: Option<Vec<u8>>,
}

impl Emitter {
    /// `expected` is the per-worker unique-line hint (see [`worker_hint`]).
    pub fn with_capacity(opts: &GenOpts, expected: usize) -> Emitter {
        Emitter {
            sink: Sink::with_capacity(expected),
            key: Vec::with_capacity(64),
            tmp: Vec::new(),
            case_insensitive: opts.case_insensitive,
            suffix: opts.suffix.clone(),
        }
    }

    #[inline]
    pub fn emit(&mut self, s: &[u8]) {
        if !self.case_insensitive && self.suffix.is_none() {
            self.sink.insert(s);
            return;
        }
        self.key.clear();
        if self.case_insensitive {
            util::downcase_into(s, &mut self.key);
        } else {
            util::push_bytes(&mut self.key, s);
        }
        if let Some(suf) = &self.suffix {
            util::push_bytes(&mut self.key, suf);
        }
        self.sink.insert(&self.key);
    }

    /// Emit a string that a static template has already produced in final,
    /// case-folded form (see `fmt::resolve_string_lower`).
    ///
    /// The uppercase/non-ASCII guard keeps this an optimization rather than a
    /// semantic shortcut: anything that is not known-lowercase falls back to
    /// [`Self::emit`], which reproduces Ruby's `downcase!` exactly.
    #[inline]
    pub fn emit_lower(&mut self, s: &[u8]) {
        if s.iter().any(|&b| b.is_ascii_uppercase() || b >= 0x80) {
            self.emit(s);
            return;
        }
        match &self.suffix {
            None => self.sink.insert(s),
            Some(suf) => {
                self.key.clear();
                util::push_bytes(&mut self.key, s);
                util::push_bytes(&mut self.key, suf);
                self.sink.insert(&self.key);
            }
        }
    }

    fn comment(&mut self, name: &str, count: Option<u64>) {
        self.tmp.clear();
        self.tmp.extend_from_slice(b"# Plugin: ");
        self.tmp.extend_from_slice(name.as_bytes());
        if let Some(n) = count {
            self.tmp.extend_from_slice(b" substituted ");
            self.tmp.extend_from_slice(n.to_string().as_bytes());
        }
        self.sink.push_line(&self.tmp);
    }

    fn comment_raw(&mut self, line: &[u8]) {
        self.sink.push_line(line);
    }

    pub fn finish(self) -> Vec<u8> {
        self.sink.finish()
    }

    pub fn into_sink(self) -> Sink {
        self.sink
    }
}

/// Scratch buffers reused across people and plugins.
pub struct Scratch {
    pub resolved: Vec<u8>,
    pub tmp: Vec<u8>,
    pub chars: Vec<u8>,
}

impl Scratch {
    pub fn new() -> Scratch {
        Scratch {
            resolved: Vec::with_capacity(64),
            tmp: Vec::with_capacity(256),
            chars: Vec::with_capacity(256),
        }
    }
}

/// Ruby `pp [@firstname, @lastname, @firstinitial, @lastinitial, @middlename,
/// @middleinitial]`, printed by `Name#init` in verbose mode.
pub fn pp_line<N: NameFields + ?Sized>(name: &N) -> Vec<u8> {
    let fields = [
        name.firstname(),
        name.lastname(),
        name.firstinitial(),
        name.lastinitial(),
        name.middlename(),
        name.middleinitial(),
    ];
    let mut out = Vec::with_capacity(64);
    out.push(b'[');
    for (i, f) in fields.iter().enumerate() {
        if i > 0 {
            out.extend_from_slice(b", ");
        }
        match f {
            Some(v) => ruby_inspect(v, &mut out),
            None => out.extend_from_slice(b"nil"),
        }
    }
    out.push(b']');
    out
}

fn ruby_inspect(s: &[u8], out: &mut Vec<u8>) {
    out.push(b'"');
    for &b in s {
        match b {
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            0x07 => out.extend_from_slice(b"\\a"),
            0x08 => out.extend_from_slice(b"\\b"),
            0x09 => out.extend_from_slice(b"\\t"),
            0x0a => out.extend_from_slice(b"\\n"),
            0x0b => out.extend_from_slice(b"\\v"),
            0x0c => out.extend_from_slice(b"\\f"),
            0x0d => out.extend_from_slice(b"\\r"),
            0x1b => out.extend_from_slice(b"\\e"),
            0x00 => out.extend_from_slice(b"\\0"),
            _ if b < 0x20 || b == 0x7f => {
                out.extend_from_slice(format!("\\x{:02X}", b).as_bytes());
            }
            _ => out.push(b),
        }
    }
    out.push(b'"');
}

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
            fmt::resolve_string(segs, lits, name, &mut sc.resolved);
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

/// Generate every candidate for one person into the emitter.
pub fn gen_person<N: NameFields + ?Sized>(
    plugins: &[Plugin],
    name: &N,
    ctx: &Ctx,
    opts: &GenOpts,
    em: &mut Emitter,
    sc: &mut Scratch,
) -> Result<(), String> {
    for p in plugins {
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
                    em.emit(&sc.tmp);
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
                    em.emit(&sc.tmp);
                }
            }
            PluginKind::Format { segs, lits } => {
                // The built-in templates have no letters in their literals, so
                // a case-insensitive run can resolve straight to final form and
                // skip `Emitter::emit`'s second case-folding pass.
                let lower_ready = opts.case_insensitive && !p.strict;
                if lower_ready {
                    fmt::resolve_string_lower(segs, lits, name, &mut sc.resolved);
                } else {
                    fmt::resolve_string(segs, lits, name, &mut sc.resolved);
                }
                // With substitutions off (the default) a template that still
                // contains a token is skipped outright. Detect that with a
                // scan: building `occ` only to discard it allocated three
                // vectors per missing-middle-name candidate.
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
                    if lower_ready {
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
fn gen_named<N: NameFields + ?Sized>(
    plugins: &[Plugin],
    name: &N,
    ctx: &Ctx,
    opts: &GenOpts,
    em: &mut Emitter,
    sc: &mut Scratch,
) -> Result<(), String> {
    if opts.verbose {
        em.comment_raw(&pp_line(name));
    }
    gen_person(plugins, name, ctx, opts, em, sc)
}

pub fn gen_people(
    people: &[Person],
    plugins: &[Plugin],
    ctx: &Ctx,
    opts: &GenOpts,
    em: &mut Emitter,
    sc: &mut Scratch,
) -> Result<(), String> {
    for p in people {
        let name = NameRef::from_person(p);
        gen_named(plugins, &name, ctx, opts, em, sc)?;
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
///
/// `head` (the positional people plus the empty `{}` sentinel the original
/// keeps in `-a` mode) is generated once, by the worker that receives the first
/// family chunk. Names are borrowed from the two dictionaries, so no `Person`
/// or per-field `Vec` is allocated per combination.
pub fn gen_auto(
    head: &[Person],
    names: AutoNames,
    plugins: &[Plugin],
    ctx: &Ctx,
    opts: &GenOpts,
    em: &mut Emitter,
    sc: &mut Scratch,
) -> Result<(), String> {
    for p in head {
        let name = NameRef::from_person(p);
        gen_named(plugins, &name, ctx, opts, em, sc)?;
    }
    for fam in names.family {
        for giv in names.given {
            let name = NameRef::from_first_last(giv.as_deref(), fam.as_deref());
            gen_named(plugins, &name, ctx, opts, em, sc)?;
        }
    }
    Ok(())
}

/// Per-person candidate count for one representative name.
fn per_person_cost<N: NameFields + ?Sized>(
    name: &N,
    plugins: &[Plugin],
    ctx: &Ctx,
    opts: &GenOpts,
) -> u64 {
    let mut sc = Scratch::new();
    let mut per_person: u64 = 0;
    for p in plugins {
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
pub fn estimate(people: &[Person], plugins: &[Plugin], ctx: &Ctx, opts: &GenOpts) -> u64 {
    let n_people = people.len().max(1) as u64;
    // Sample a real row: empty/sentinel rows expand missing fields to the whole
    // dictionaries and would wildly overestimate the per-person cost.
    let sample = people
        .iter()
        .find(|p| p.iter().any(|x| x.is_some()))
        .cloned()
        .unwrap_or_default();
    let name = NameRef::from_person(&sample);
    per_person_cost(&name, plugins, ctx, opts).saturating_mul(n_people)
}

/// Estimate for the streamed `--auto` cross-product.
///
/// The sentinel (which expands missing fields to the whole dictionaries) and
/// the concrete `(given, family)` pairs have very different costs, so they are
/// counted separately.
pub fn estimate_auto(
    head: &[Person],
    names: AutoNames,
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
