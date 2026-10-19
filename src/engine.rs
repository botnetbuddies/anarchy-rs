//! Candidate generation, global de-duplication, and buffered output.

use std::hash::Hasher;

use rustc_hash::FxHasher;

use crate::fmt::{self, Ctx, Plugin, PluginKind};
use crate::names::{Name, Person};
use crate::util;

#[inline]
fn fxhash(bytes: &[u8]) -> u64 {
    let mut h = FxHasher::default();
    h.write(bytes);
    h.finish()
}

/// De-duplication set plus ordered output for one worker.
///
/// Every emitted line's bytes are stored once, contiguously, in `out`; `entries`
/// records `(offset, len)` for each line in emission order (comment lines
/// included). Candidate lines are additionally threaded through a hash-bucket
/// chain (`heads`/`nexts`) so that a repeated candidate costs one hash, one byte
/// comparison and *no allocation* — the profiler showed malloc/free dominating a
/// straightforward `HashSet<Vec<u8>>` implementation.
///
/// Workers never share a `Sink`, so no locking is needed on the hot path; the
/// parallel path merges the per-worker sinks at the end.
pub struct Sink {
    out: Vec<u8>,
    entries: Vec<(u32, u32)>,
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

    /// `expected` is a hint for the number of *unique* lines.
    pub fn with_capacity(expected: usize) -> Sink {
        let mut cap = 128usize;
        while cap * 7 / 10 < expected {
            cap *= 2;
        }
        Sink {
            out: Vec::new(),
            entries: Vec::new(),
            is_comment: Vec::new(),
            table: vec![(0u32, 0u32); cap],
            mask: cap - 1,
            filled: 0,
        }
    }

    #[inline]
    fn hash_of(&self, off: usize, len: usize) -> u64 {
        fxhash(&self.out[off..off + len])
    }

    /// Append a line that must not participate in de-duplication (a comment).
    #[inline]
    fn push_line(&mut self, line: &[u8]) {
        let off = self.out.len();
        debug_assert!(off <= u32::MAX as usize, "output exceeded 4 GiB");
        util::push_bytes(&mut self.out, line);
        self.entries.push((off as u32, line.len() as u32));
        self.is_comment.push(true);
    }

    #[inline]
    fn insert(&mut self, key: &[u8]) {
        if self.filled * 10 >= self.table.len() * 7 {
            self.grow();
        }
        let h = fxhash(key);
        let tag = (h >> 32) as u32;
        let mut i = (h as usize) & self.mask;
        loop {
            let (stored_tag, e) = self.table[i];
            if e == 0 {
                break;
            }
            if stored_tag == tag {
                let ei = (e - 1) as usize;
                let (off, len) = self.entries[ei];
                let off = off as usize;
                if len as usize == key.len() && self.out[off..off + key.len()] == *key {
                    return;
                }
            }
            i = (i + 1) & self.mask;
        }
        let off = self.out.len();
        debug_assert!(off <= u32::MAX as usize, "output exceeded 4 GiB");
        util::push_bytes(&mut self.out, key);
        self.entries.push((off as u32, key.len() as u32));
        self.is_comment.push(false);
        self.table[i] = (tag, self.entries.len() as u32);
        self.filled += 1;
    }

    fn grow(&mut self) {
        let new_cap = self.table.len() * 2;
        let mut table = vec![(0u32, 0u32); new_cap];
        let mask = new_cap - 1;
        for ei in 0..self.entries.len() {
            if self.is_comment[ei] {
                continue;
            }
            let (off, len) = self.entries[ei];
            let h = self.hash_of(off as usize, len as usize);
            let tag = (h >> 32) as u32;
            let mut i = (h as usize) & mask;
            while table[i].1 != 0 {
                i = (i + 1) & mask;
            }
            table[i] = (tag, ei as u32 + 1);
        }
        self.table = table;
        self.mask = mask;
    }

    /// Merge another worker's lines into this sink, keeping only candidate lines
    /// not already present (comment lines are preserved verbatim). Used once,
    /// single-threaded, at the end of a parallel run.
    pub fn absorb(&mut self, other: Sink) {
        for (ei, &(off, len)) in other.entries.iter().enumerate() {
            let off = off as usize;
            let line = &other.out[off..off + len as usize];
            if other.is_comment[ei] {
                self.push_line(line);
            } else {
                self.insert(line);
            }
        }
    }

    pub fn finish(self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.out.len() + self.entries.len());
        for &(off, len) in &self.entries {
            let off = off as usize;
            util::push_bytes(&mut out, &self.out[off..off + len as usize]);
            out.push(b'\n');
        }
        out
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

/// Per-worker emitter: applies case folding and the suffix, then de-duplicates.
pub struct Emitter {
    sink: Sink,
    key: Vec<u8>,
    tmp: Vec<u8>,
    case_insensitive: bool,
    suffix: Option<Vec<u8>>,
}

impl Emitter {
    pub fn new(opts: &GenOpts) -> Emitter {
        Emitter {
            sink: Sink::new(),
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
pub fn pp_line(name: &Name) -> Vec<u8> {
    let fields = [
        &name.firstname,
        &name.lastname,
        &name.firstinitial,
        &name.lastinitial,
        &name.middlename,
        &name.middleinitial,
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
pub fn all_outputs(
    p: &Plugin,
    name: &Name,
    ctx: &Ctx,
    substitute: bool,
    sc: &mut Scratch,
) -> GenRet {
    match &p.kind {
        PluginKind::TruncFirstLast8 => match (&name.firstname, &name.lastname) {
            (Some(f), Some(l)) => {
                sc.chars.clear();
                sc.chars.extend_from_slice(f);
                sc.chars.extend_from_slice(l);
                let end = util::nth_char_boundary(&sc.chars, 8);
                GenRet::Str(sc.chars[..end].to_vec())
            }
            _ => GenRet::Nil,
        },
        PluginKind::TruncFirst4Last4 => match (&name.firstname, &name.lastname) {
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
pub fn gen_person(
    plugins: &[Plugin],
    name: &Name,
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
                if let (Some(f), Some(l)) = (&name.firstname, &name.lastname) {
                    sc.chars.clear();
                    sc.chars.extend_from_slice(f);
                    sc.chars.extend_from_slice(l);
                    let end = util::nth_char_boundary(&sc.chars, 8);
                    let v: Vec<u8> = sc.chars[..end].to_vec();
                    em.emit(&v);
                }
            }
            PluginKind::TruncFirst4Last4 => {
                if opts.verbose {
                    em.comment(&p.name, None);
                }
                if let (Some(f), Some(l)) = (&name.firstname, &name.lastname) {
                    let fe = util::nth_char_boundary(f, 4);
                    let le = util::nth_char_boundary(l, 4);
                    sc.tmp.clear();
                    sc.tmp.extend_from_slice(&f[..fe]);
                    sc.tmp.extend_from_slice(&l[..le]);
                    let v = sc.tmp.clone();
                    em.emit(&v);
                }
            }
            PluginKind::Format { segs, lits } => {
                fmt::resolve_string(segs, lits, name, &mut sc.resolved);
                let occ = fmt::tokenize(&sc.resolved);
                if !occ.is_empty() && !opts.substitute {
                    if opts.verbose {
                        em.comment(&p.name, None);
                    }
                    continue;
                }
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
                    fmt::enumerate(&sc.resolved, &occ, ctx, limit, &mut sc.tmp, |s| em.emit(s));
                }
            }
        }
    }
    Ok(())
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
        let name = Name::init(p);
        if opts.verbose {
            em.comment_raw(&pp_line(&name));
        }
        gen_person(plugins, &name, ctx, opts, em, sc)?;
    }
    Ok(())
}

/// Rough upper bound on the number of generated candidates.
pub fn estimate(people: &[Person], plugins: &[Plugin], ctx: &Ctx, opts: &GenOpts) -> u64 {
    let n_people = people.len().max(1) as u64;
    // In `-a` mode the first person is the empty `{}` sentinel; use a real row
    // when one exists so the parallel-size heuristic is not fooled.
    let sample = people
        .iter()
        .find(|p| p.iter().any(|x| x.is_some()))
        .cloned()
        .unwrap_or_default();
    let name = Name::init(&sample);
    let mut sc = Scratch::new();
    let mut per_person: u64 = 0;
    for p in plugins {
        match &p.kind {
            PluginKind::TruncFirstLast8 | PluginKind::TruncFirst4Last4 => {
                per_person = per_person.saturating_add(1);
            }
            PluginKind::Format { segs, lits } => {
                fmt::resolve_string(segs, lits, &name, &mut sc.resolved);
                let occ = fmt::tokenize(&sc.resolved);
                if !occ.is_empty() && !opts.substitute {
                    continue;
                }
                let total = fmt::expansion_total(&occ, ctx);
                per_person = per_person.saturating_add(total.min(opts.limit(total)));
            }
        }
    }
    per_person.saturating_mul(n_people)
}
