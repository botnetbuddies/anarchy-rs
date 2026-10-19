//! Case folding, suffixes and verbose comments.

use super::SinkTarget;
use crate::{engine::GenOpts, util};

/// Emitter: applies case folding and the suffix, then writes to the underlying SinkTarget.
pub struct Emitter<S: SinkTarget> {
    sink: S,
    key: Vec<u8>,
    tmp: Vec<u8>,
    case_insensitive: bool,
    suffix: Option<Vec<u8>>,
}

impl<S: SinkTarget> Emitter<S> {
    pub fn new(opts: &GenOpts, sink: S) -> Emitter<S> {
        Emitter {
            sink,
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
