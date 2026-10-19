//! Exact deduplication and ordered worker batches.

use crate::{hash::hash_key, util};

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
    #[cfg(test)]
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
