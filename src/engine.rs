//! Candidate generation, global de-duplication, and buffered output.

use crate::fmt::{self, Ctx, Plugin, PluginKind};
use crate::hash::hash_key;
use crate::names::{NameFields, NameRef, Person, Row};
use crate::util;

/// Log2 of the size of one output arena chunk (256 KiB).
const CHUNK_SHIFT: u32 = 18;
const CHUNK_SIZE: usize = 1 << CHUNK_SHIFT;
const CHUNK_MASK: u64 = (CHUNK_SIZE - 1) as u64;

/// The full 64-bit hash of a candidate line, remapped so it is never zero.
///
/// Zero is reserved as the "comment" marker in [`Entry`] and as the "empty
/// slot" marker in the de-duplication table.
#[inline]
fn line_hash(key: &[u8]) -> u64 {
    let h = hash_key(key);
    if h == 0 {
        1
    } else {
        h
    }
}

/// One output line: byte range in the arena plus its full 64-bit hash.
///
/// `off` packs the chunk index and the offset within it
/// (`(chunk << CHUNK_SHIFT) | offset`), so a single `u64` addresses the whole
/// arena. That removes the old flat-`u32` 4 GiB ceiling without widening every
/// comparison. `hash == 0` marks a comment, which never enters the table.
#[derive(Clone, Copy)]
struct Entry {
    off: u64,
    hash: u64,
    len: u32,
}

/// Borrow the line (excluding its terminator) addressed by `ent`.
#[inline]
fn entry_bytes<'a>(chunks: &'a [Vec<u8>], ent: &Entry) -> &'a [u8] {
    let ci = (ent.off >> CHUNK_SHIFT) as usize;
    let off = (ent.off & CHUNK_MASK) as usize;
    &chunks[ci][off..off + ent.len as usize]
}

/// De-duplication set plus ordered output for one worker.
///
/// Every emitted line's bytes are stored once, contiguously, in a list of fixed
/// arenas (`chunks`), followed by a `\n`; `entries` records `(offset, hash,
/// len)` for each line in emission order (comment lines included). `len`
/// excludes the terminator, so duplicate comparisons and merges see exactly the
/// original line. Candidate lines are additionally threaded through an
/// open-addressed hash set so a repeated candidate costs one hash, one byte
/// comparison and *no allocation* - the profiler showed malloc/free dominating
/// a straightforward `HashSet<Vec<u8>>` implementation.
///
/// The table stores its `u64` hashes and `u32` entry indices in two parallel
/// arrays: probing a miss touches only the hash array, which is the common case
/// for unique lines, and the wide hash makes a false tag match effectively
/// impossible before the byte comparison runs.
///
/// Workers never share a `Sink`, so no locking is needed on the hot path; the
/// parallel path merges the per-worker sinks at the end.
pub struct Sink {
    chunks: Vec<Vec<u8>>,
    entries: Vec<Entry>,
    /// Open-addressed set: `hashes[i] == 0` marks an empty slot and
    /// `slots[i]` holds `entry index + 1` for the matching line.
    hashes: Vec<u64>,
    slots: Vec<u32>,
    mask: usize,
    filled: usize,
    /// Capacity of the very first chunk; later chunks are `CHUNK_SIZE`.
    first_cap: usize,
}

impl Sink {
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
            chunks: Vec::new(),
            entries: Vec::with_capacity(expected),
            hashes: vec![0u64; cap],
            slots: vec![0u32; cap],
            mask: cap - 1,
            filled: 0,
            first_cap: expected.saturating_mul(16).clamp(256, CHUNK_SIZE),
        }
    }

    /// Start a fresh arena chunk and return its index.
    #[cold]
    fn new_chunk(&mut self) -> usize {
        let cap = if self.chunks.is_empty() {
            self.first_cap
        } else {
            CHUNK_SIZE
        };
        self.chunks.push(Vec::with_capacity(cap));
        self.chunks.len() - 1
    }

    /// Append `line` plus its terminator, returning its packed offset and
    /// length. A line never spans two chunks, so a chunk that would overflow is
    /// closed early (and a line longer than a chunk simply gets its own).
    #[inline]
    fn append(&mut self, line: &[u8]) -> (u64, u32) {
        let need = line.len() + 1;
        let ci = match self.chunks.last() {
            Some(c) if c.len() + need <= CHUNK_SIZE => self.chunks.len() - 1,
            _ => self.new_chunk(),
        };
        let c = &mut self.chunks[ci];
        let off = c.len();
        c.extend_from_slice(line);
        c.push(b'\n');
        (((ci as u64) << CHUNK_SHIFT) | off as u64, line.len() as u32)
    }

    /// Append a line that must not participate in de-duplication (a comment).
    #[inline]
    fn push_line(&mut self, line: &[u8]) {
        let (off, len) = self.append(line);
        self.entries.push(Entry { off, hash: 0, len });
    }

    #[inline]
    fn insert(&mut self, key: &[u8]) {
        self.insert_hashed(key, line_hash(key));
    }

    /// Insert with a precomputed 64-bit hash.
    ///
    /// The hash is both the probe value and the bucket index (`hash & mask`),
    /// so a merge can reuse the hash each entry already stores instead of
    /// hashing the bytes again.
    #[inline]
    fn insert_hashed(&mut self, key: &[u8], hash: u64) {
        if self.filled * 10 >= self.hashes.len() * 7 {
            self.grow();
        }
        let mut i = (hash as usize) & self.mask;
        loop {
            let h = self.hashes[i];
            if h == 0 {
                break;
            }
            if h == hash {
                let ent = self.entries[(self.slots[i] - 1) as usize];
                let b = entry_bytes(&self.chunks, &ent);
                if b.len() == key.len() && b == key {
                    return;
                }
            }
            i = (i + 1) & self.mask;
        }
        self.push_unique(key, hash, i);
    }

    /// Append a line known not to be present and register it in slot `i`.
    #[inline]
    fn push_unique(&mut self, key: &[u8], hash: u64, i: usize) {
        let (off, len) = self.append(key);
        self.entries.push(Entry { off, hash, len });
        self.hashes[i] = hash;
        self.slots[i] = self.entries.len() as u32;
        self.filled += 1;
    }

    fn grow(&mut self) {
        let new_cap = self.hashes.len() * 2;
        let mut hashes = vec![0u64; new_cap];
        let mut slots = vec![0u32; new_cap];
        let mask = new_cap - 1;
        for (ei, ent) in self.entries.iter().enumerate() {
            if ent.hash == 0 {
                continue;
            }
            let mut i = (ent.hash as usize) & mask;
            while hashes[i] != 0 {
                i = (i + 1) & mask;
            }
            hashes[i] = ent.hash;
            slots[i] = ei as u32 + 1;
        }
        self.hashes = hashes;
        self.slots = slots;
        self.mask = mask;
    }
}

/// A reference to one line in one worker sink, used by the merged output.
#[derive(Clone, Copy)]
struct Ref {
    sink: u32,
    entry: u32,
}

/// The result of generation: either ready-to-write chunk arenas, or a merge
/// descriptor that references the per-worker sinks without copying their bytes.
pub struct Output {
    kind: OutputKind,
}

enum OutputKind {
    Chunks(Vec<Vec<u8>>),
    Merged { sinks: Vec<Sink>, refs: Vec<Ref> },
}

impl Output {
    fn chunks(chunks: Vec<Vec<u8>>) -> Output {
        Output {
            kind: OutputKind::Chunks(chunks),
        }
    }

    /// Write every line, including the terminating newline, to `w`.
    pub fn write_all<W: std::io::Write>(&self, w: &mut W) -> std::io::Result<()> {
        match &self.kind {
            OutputKind::Chunks(chunks) => {
                for c in chunks {
                    w.write_all(c)?;
                }
            }
            OutputKind::Merged { sinks, refs } => {
                for r in refs {
                    let sink = &sinks[r.sink as usize];
                    let ent = &sink.entries[r.entry as usize];
                    let ci = (ent.off >> CHUNK_SHIFT) as usize;
                    let off = (ent.off & CHUNK_MASK) as usize;
                    // Include the terminator: one write per line.
                    w.write_all(&sink.chunks[ci][off..off + ent.len as usize + 1])?;
                }
            }
        }
        Ok(())
    }
}

/// De-duplicate one partition's refs.
///
/// Refs arrive in `(sink, entry)` order; the first occurrence of a key is kept
/// and later ones dropped. Comments (`hash == 0`) are never de-duplicated.
fn dedup_partition(refs: Vec<Ref>, sinks: &[Sink]) -> Vec<Ref> {
    let mut keep: Vec<Ref> = Vec::with_capacity(refs.len());
    let mut hashes = vec![0u64; 256];
    let mut slots = vec![0u32; 256];
    let mut mask = 255usize;
    let mut filled = 0usize;
    for r in refs {
        let ent = sinks[r.sink as usize].entries[r.entry as usize];
        if ent.hash == 0 {
            keep.push(r);
            continue;
        }
        let cur = entry_bytes(&sinks[r.sink as usize].chunks, &ent);
        if filled * 10 >= hashes.len() * 7 {
            rehash(&mut hashes, &mut slots, &mut mask, &keep, sinks);
        }
        let mut i = (ent.hash as usize) & mask;
        let mut dup = false;
        loop {
            let h = hashes[i];
            if h == 0 {
                break;
            }
            if h == ent.hash {
                let k = keep[(slots[i] - 1) as usize];
                let kb = entry_bytes(
                    &sinks[k.sink as usize].chunks,
                    &sinks[k.sink as usize].entries[k.entry as usize],
                );
                if kb == cur {
                    dup = true;
                    break;
                }
            }
            i = (i + 1) & mask;
        }
        if dup {
            continue;
        }
        hashes[i] = ent.hash;
        slots[i] = keep.len() as u32 + 1;
        keep.push(r);
        filled += 1;
    }
    keep
}

/// Rebuild a partition's probe table at twice the size.
fn rehash(
    hashes: &mut Vec<u64>,
    slots: &mut Vec<u32>,
    mask: &mut usize,
    keep: &[Ref],
    sinks: &[Sink],
) {
    let new_cap = hashes.len() * 2;
    let mut nh = vec![0u64; new_cap];
    let mut ns = vec![0u32; new_cap];
    let nm = new_cap - 1;
    for (k, r) in keep.iter().enumerate() {
        let ent = sinks[r.sink as usize].entries[r.entry as usize];
        if ent.hash == 0 {
            continue;
        }
        let mut i = (ent.hash as usize) & nm;
        while nh[i] != 0 {
            i = (i + 1) & nm;
        }
        nh[i] = ent.hash;
        ns[i] = k as u32 + 1;
    }
    *hashes = nh;
    *slots = ns;
    *mask = nm;
}

/// Merge per-worker sinks into a single writable output.
///
/// Phase 1 routes every entry to the partition that owns its hash - one linear
/// pass that moves no bytes. Phase 2 de-duplicates each partition in parallel;
/// because a key's copies all share a hash, they all land in the same partition
/// and no cross-partition comparison is needed. The result references the
/// original sinks, so merging never copies the output.
///
/// A single sink needs no merge at all and is handed back as its chunk arenas.
pub fn merge(sinks: Vec<Sink>, partitions: usize) -> Output {
    if sinks.len() <= 1 {
        return Output::chunks(sinks.into_iter().next().map_or_else(Vec::new, |s| s.chunks));
    }
    // Size the partition set to the work available: one partition per ~4k
    // entries, never more than the cores (or 16). Spawning the full 16 threads
    // for a merge that only has a few hundred refs costs more in setup and
    // join than the parallel dedup saves.
    let cap = partitions.clamp(1, 16).next_power_of_two();
    let total: usize = sinks.iter().map(|s| s.entries.len()).sum();
    let p = (total / 4096).clamp(1, cap).next_power_of_two().min(cap);
    let mask = (p - 1) as u64;

    // Phase 1: route once. Comments go to partition 0 (they are never
    // de-duplicated and must all be preserved).
    let mut buckets: Vec<Vec<Ref>> = (0..p).map(|_| Vec::new()).collect();
    for (si, sink) in sinks.iter().enumerate() {
        for (ei, ent) in sink.entries.iter().enumerate() {
            let pid = if ent.hash == 0 {
                0
            } else {
                (ent.hash & mask) as usize
            };
            buckets[pid].push(Ref {
                sink: si as u32,
                entry: ei as u32,
            });
        }
    }

    // A single partition is the serial fold; don't pay for a thread to run it.
    if p == 1 {
        let refs = dedup_partition(buckets.pop().unwrap_or_default(), &sinks);
        return Output {
            kind: OutputKind::Merged { sinks, refs },
        };
    }

    let mut parts: Vec<Vec<Ref>> = Vec::with_capacity(p);
    std::thread::scope(|s| {
        let mut handles = Vec::with_capacity(p);
        for bucket in buckets {
            let sinks = &sinks;
            handles.push(s.spawn(move || dedup_partition(bucket, sinks)));
        }
        for h in handles {
            match h.join() {
                Ok(part) => parts.push(part),
                Err(_) => panic!("merge worker panicked"),
            }
        }
    });
    let refs: Vec<Ref> = parts.into_iter().flatten().collect();
    Output {
        kind: OutputKind::Merged { sinks, refs },
    }
}

/// Per-worker capacity hint for [`Sink::with_capacity`].
///
/// `total` is an upper bound on candidates (from [`estimate`]); only a fraction
/// of them are unique. The hint is deliberately conservative - see the comment
/// in the body - because oversizing the table costs more (cache, page faults)
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
    /// case-folded form (see `fmt::resolve`).
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

    /// Emit a string already known to be lowercase ASCII, skipping the
    /// uppercase/non-ASCII guard in [`Self::emit_lower`].
    ///
    /// Only call this when the resolved candidate came from fields that passed
    /// `field_props` (no uppercase or non-ASCII bytes) and a non-strict
    /// template (whose literals are `.`), or from an equivalent scan.
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

    pub fn finish(self) -> Output {
        Output::chunks(self.sink.chunks)
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
    pub prep: fmt::Prep,
}

impl Scratch {
    pub fn new() -> Scratch {
        Scratch {
            resolved: Vec::with_capacity(64),
            tmp: Vec::with_capacity(256),
            chars: Vec::with_capacity(256),
            prep: fmt::Prep::default(),
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
    // Item 1: memoize every casing of every field once per person instead of
    // recomputing them inside each template's resolution.
    let need_title = !opts.case_insensitive || plugins.iter().any(|p| p.strict);
    let pn = fmt::prepare(name, &mut sc.prep, need_title);
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
                    if pn.plain {
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
                    if pn.plain {
                        em.emit_known_lower(&sc.tmp);
                    } else {
                        em.emit(&sc.tmp);
                    }
                }
            }
            PluginKind::Format { segs, lits } => {
                // Item 5: with substitutions off, a template that is guaranteed
                // to contain a token - either one of its own `Seg::Tok` segments
                // or a missing required field - is skipped without resolving or
                // scanning the result.
                if !opts.substitute && (p.has_tok || (p.required & !pn.present) != 0) {
                    if opts.verbose {
                        em.comment(&p.name, None);
                    }
                    continue;
                }
                let lower_ready = opts.case_insensitive && !p.strict;
                let mode = if lower_ready {
                    fmt::Mode::LowerFold
                } else {
                    fmt::Mode::Normal
                };
                // Fast path (items 1 + 2 + 6): no token can appear in the
                // resolved string (no `Seg::Tok`, no missing field, no `%` in
                // literals or field values), so the single candidate is built
                // straight from the memoized segments, stripped in place, and
                // de-duplicated without a tokenize pass or a temporary copy.
                let fast = !p.strict
                    && !p.has_tok
                    && !p.lit_pct
                    && !pn.has_pct
                    && (p.required & !pn.present) == 0;
                if fast {
                    fmt::resolve(segs, lits, &pn, mode, &mut sc.resolved);
                    if opts.verbose {
                        em.comment(&p.name, Some(1));
                    }
                    if !pn.clean {
                        util::strip_spaces_and_apostrophes(&mut sc.resolved);
                    }
                    if pn.plain {
                        em.emit_known_lower(&sc.resolved);
                    } else if lower_ready {
                        em.emit_lower(&sc.resolved);
                    } else {
                        em.emit(&sc.resolved);
                    }
                    continue;
                }
                // Faithful slow path: custom formats and templates whose
                // resolved string may contain expansion tokens.
                fmt::resolve(segs, lits, &pn, mode, &mut sc.resolved);
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
                        if !pn.clean {
                            util::strip_spaces_and_apostrophes(&mut sc.resolved);
                        }
                        if pn.plain {
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

/// Generate for borrowed input rows (the `-i` path).
///
/// `head` holds the positional people (always tiny) and is emitted first; the
/// rows borrow their fields straight out of `data`, so no `Person` or per-field
/// `Vec` is allocated for the (potentially huge) file.
#[allow(clippy::too_many_arguments)]
pub fn gen_rows(
    data: &[u8],
    rows: &[Row],
    head: &[Person],
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
    for row in rows {
        let name = NameRef::from_ranges(data, row);
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

/// Estimate for the borrowed-row `-i` path.
///
/// Samples one row that has at least one present field, falling back to the
/// empty person (which expands missing fields to the whole dictionaries) when
/// every row is blank. The positional head is counted separately and only when
/// it is non-empty, so a bare `-i file` run is not inflated by a phantom
/// sentinel.
pub fn estimate_file(
    head: &[Person],
    data: &[u8],
    rows: &[Row],
    plugins: &[Plugin],
    ctx: &Ctx,
    opts: &GenOpts,
) -> u64 {
    let n_head = head.len() as u64;
    let n_rows = rows.len() as u64;
    if n_rows == 0 {
        return if n_head == 0 {
            0
        } else {
            estimate(head, plugins, ctx, opts)
        };
    }
    let sample = rows
        .iter()
        .find(|r| r.iter().any(|(a, b)| a != b))
        .map(|r| NameRef::from_ranges(data, r))
        .unwrap_or_default();
    let row_cost = per_person_cost(&sample, plugins, ctx, opts);
    let head_cost = if n_head == 0 {
        0
    } else {
        estimate(head, plugins, ctx, opts)
    };
    head_cost.saturating_add(n_rows.saturating_mul(row_cost))
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
