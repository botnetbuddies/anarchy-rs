//! Candidate generation, global de-duplication, and buffered output.

use crate::fmt::{self, Ctx, Plugin, PluginKind};
use crate::hash::hash_key;
use crate::names::{NameFields, NameRef, Person};
use crate::util;

pub trait SinkTarget {
    fn insert(&mut self, key: &[u8]);
    fn push_line(&mut self, line: &[u8]);
}

/// Global de-duplication set and buffered output.
///
/// Uses an open-addressing table with structure-of-arrays layout:
/// `hashes` stores dense 32-bit hash tags for fast cache-line scanning.
/// `slots` stores `(offset, len)` pointing into `out`.
pub struct FinalSink {
    out: Vec<u8>,
    hashes: Vec<u32>,
    slots: Vec<(u64, u32)>,
    mask: usize,
    filled: usize,
}

impl FinalSink {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self::with_capacity(16384)
    }

    pub fn with_capacity(expected: usize) -> Self {
        let mut cap = 16384usize;
        while cap.saturating_mul(7) / 10 < expected {
            match cap.checked_mul(2) {
                Some(c) => cap = c,
                None => break,
            }
        }
        FinalSink {
            out: Vec::with_capacity(expected.saturating_mul(12)),
            hashes: vec![0u32; cap],
            slots: vec![(0u64, 0u32); cap],
            mask: cap - 1,
            filled: 0,
        }
    }

    #[inline]
    pub fn push_line(&mut self, line: &[u8]) {
        util::push_bytes(&mut self.out, line);
        self.out.push(b'\n');
    }

    #[inline]
    pub fn insert_hashed(&mut self, key: &[u8], mut tag: u32) {
        if tag == 0 {
            tag = 1;
        }
        if self.filled * 10 >= self.hashes.len() * 7 {
            self.grow();
        }
        let mut i = (tag as usize) & self.mask;
        loop {
            let stored_tag = self.hashes[i];
            if stored_tag == 0 {
                let off = self.out.len() as u64;
                let len = u32::try_from(key.len()).expect("candidate exceeds 4 GiB");
                util::push_bytes(&mut self.out, key);
                self.out.push(b'\n');
                self.hashes[i] = tag;
                self.slots[i] = (off, len);
                self.filled += 1;
                return;
            }
            if stored_tag == tag {
                let (off, len) = self.slots[i];
                if len as usize == key.len() {
                    let existing = &self.out[off as usize..off as usize + len as usize];
                    if util::bytes_equal(existing, key) {
                        return;
                    }
                }
            }
            i = (i + 1) & self.mask;
        }
    }

    fn grow(&mut self) {
        let new_cap = self.hashes.len() * 2;
        let new_mask = new_cap - 1;
        let mut new_hashes = vec![0u32; new_cap];
        let mut new_slots = vec![(0u64, 0u32); new_cap];
        for i in 0..self.hashes.len() {
            let tag = self.hashes[i];
            if tag != 0 {
                let slot = self.slots[i];
                let mut idx = (tag as usize) & new_mask;
                while new_hashes[idx] != 0 {
                    idx = (idx + 1) & new_mask;
                }
                new_hashes[idx] = tag;
                new_slots[idx] = slot;
            }
        }
        self.hashes = new_hashes;
        self.slots = new_slots;
        self.mask = new_mask;
    }

    pub fn finish(self) -> Vec<u8> {
        self.out
    }
}

impl SinkTarget for FinalSink {
    #[inline]
    fn insert(&mut self, key: &[u8]) {
        let tag = hash_key(key) as u32;
        self.insert_hashed(key, tag);
    }
    #[inline]
    fn push_line(&mut self, line: &[u8]) {
        self.push_line(line);
    }
}

#[derive(Clone, Copy)]
pub struct BatchEntry {
    pub off: u64,
    pub len: u32,
    /// Zero marks comments; candidate hashes map zero to one, as FinalSink does.
    pub tag: u32,
}

pub struct Batch {
    pub chunk_idx: usize,
    pub out: Vec<u8>,
    pub entries: Vec<BatchEntry>,
}

impl Batch {
    pub fn new(chunk_idx: usize, capacity: usize) -> Self {
        Batch {
            chunk_idx,
            out: Vec::with_capacity(capacity.saturating_mul(12)),
            entries: Vec::with_capacity(capacity),
        }
    }
}

impl SinkTarget for Batch {
    #[inline]
    fn insert(&mut self, key: &[u8]) {
        let off = self.out.len() as u64;
        let len = u32::try_from(key.len()).expect("candidate exceeds 4 GiB");
        let tag = (hash_key(key) as u32).max(1);
        util::push_bytes(&mut self.out, key);
        self.out.push(b'\n');
        self.entries.push(BatchEntry { off, len, tag });
    }

    #[inline]
    fn push_line(&mut self, line: &[u8]) {
        let off = self.out.len() as u64;
        let len = u32::try_from(line.len()).expect("comment exceeds 4 GiB");
        util::push_bytes(&mut self.out, line);
        self.out.push(b'\n');
        self.entries.push(BatchEntry { off, len, tag: 0 });
    }
}

#[inline]
pub fn merge_batch(batch: &Batch, sink: &mut FinalSink) {
    for entry in &batch.entries {
        let off = entry.off as usize;
        let len = entry.len as usize;
        let line = &batch.out[off..off + len];
        if entry.tag == 0 {
            sink.push_line(line);
        } else {
            sink.insert_hashed(line, entry.tag);
        }
    }
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

/// Emitter: applies case folding and the suffix, then writes to the underlying SinkTarget.
pub struct Emitter<'a, S: SinkTarget> {
    sink: S,
    key: Vec<u8>,
    tmp: Vec<u8>,
    case_insensitive: bool,
    suffix: Option<Vec<u8>>,
    _opts: &'a GenOpts,
}

impl<'a, S: SinkTarget> Emitter<'a, S> {
    pub fn new(opts: &'a GenOpts, sink: S) -> Emitter<'a, S> {
        Emitter {
            sink,
            key: Vec::with_capacity(64),
            tmp: Vec::new(),
            case_insensitive: opts.case_insensitive,
            suffix: opts.suffix.clone(),
            _opts: opts,
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

    /// Emit a string already known to be lowercase ASCII, skipping the
    /// uppercase/non-ASCII guard in [`Self::emit_lower`].
    #[inline]
    pub fn emit_known_lower(&mut self, s: &[u8]) {
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

    pub fn comment(&mut self, name: &str, count: Option<u64>) {
        self.tmp.clear();
        self.tmp.extend_from_slice(b"# Plugin: ");
        self.tmp.extend_from_slice(name.as_bytes());
        if let Some(n) = count {
            self.tmp.extend_from_slice(b" substituted ");
            self.tmp.extend_from_slice(n.to_string().as_bytes());
        }
        self.sink.push_line(&self.tmp);
    }

    pub fn comment_raw(&mut self, line: &[u8]) {
        self.sink.push_line(line);
    }

    pub fn into_sink(self) -> S {
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

/// Generate every candidate for one person into the emitter.
pub fn gen_person<N: NameFields + ?Sized, S: SinkTarget>(
    plugins: &[Plugin],
    name: &N,
    ctx: &Ctx,
    opts: &GenOpts,
    em: &mut Emitter<'_, S>,
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
    em: &mut Emitter<'_, S>,
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
    em: &mut Emitter<'_, S>,
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
    em: &mut Emitter<'_, S>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedupe_compares_bytes_on_hash_collisions_and_survives_growth() {
        let mut sink = FinalSink::new();
        let mut expected = Vec::new();
        // More than the initial table's load limit, with deliberately shared
        // tags and bucket collisions. Different strings must remain distinct.
        for i in 0u32..12_000 {
            let key = format!("user-{i}");
            let tag = (i / 2) * 128;
            sink.insert_hashed(key.as_bytes(), tag);
            expected.extend_from_slice(key.as_bytes());
            expected.push(b'\n');
        }
        for i in (0u32..12_000).rev() {
            let key = format!("user-{i}");
            sink.insert_hashed(key.as_bytes(), (i / 2) * 128);
        }
        assert_eq!(sink.finish(), expected);
    }

    #[test]
    fn batch_comments_are_not_deduplicated_with_candidates() {
        let mut sink = FinalSink::new();
        for idx in 0..3 {
            let mut batch = Batch::new(idx, 4);
            batch.push_line(b"# comment");
            batch.insert(b"anna.key");
            batch.insert(b"# comment");
            batch.insert(b"anna.key");
            merge_batch(&batch, &mut sink);
        }
        assert_eq!(
            sink.finish(),
            b"# comment\nanna.key\n# comment\n# comment\n# comment\n"
        );
    }
}
