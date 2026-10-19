//! Hash candidate bytes for the deduplication table.

use std::hash::Hasher;

use rustc_hash::FxHasher;

#[inline]
pub fn hash_key(bytes: &[u8]) -> u64 {
    let mut hasher = FxHasher::default();
    hasher.write(bytes);
    hasher.finish()
}
