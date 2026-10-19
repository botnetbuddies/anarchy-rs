//! Selectable 64-bit hash for candidate keys.
//!
//! The full 64 bits are stored per entry and used for probing, table growth and
//! merge partitioning; the low bits also select the bucket (`hash & mask`).
//! That makes the hash a pure per-candidate cost, with no effect on output, so
//! it is worth choosing empirically.
//!
//! The default is `hash-fx`. To benchmark an alternative:
//!
//! ```sh
//! cargo build --release --no-default-features --features hash-wy
//! cargo build --release --no-default-features --features hash-rapid
//! ```
//!
//! The `hash-wy` implementation is a wyhash-style mixer (16-byte blocks with an
//! overlapping final block); `hash-rapid` is a rapidhash-style short-input
//! mixer. Both are self-contained, so comparing them costs no dependencies.

#[cfg(all(
    feature = "hash-fx",
    not(any(feature = "hash-wy", feature = "hash-rapid"))
))]
mod imp {
    use std::hash::Hasher;

    use rustc_hash::FxHasher;

    #[inline]
    pub fn hash_key(bytes: &[u8]) -> u64 {
        let mut h = FxHasher::default();
        h.write(bytes);
        h.finish()
    }
}

#[cfg(all(feature = "hash-wy", not(feature = "hash-rapid")))]
mod imp {
    const P0: u64 = 0xa076_1d64_78bd_642f;
    const P1: u64 = 0xe703_7ed1_a0b4_28db;
    const P2: u64 = 0x8ebc_6af0_9c88_c6e3;
    const P3: u64 = 0x5899_65cc_7537_4cc3;

    #[inline(always)]
    fn read_u64(b: &[u8]) -> u64 {
        u64::from_le_bytes(b[..8].try_into().unwrap())
    }

    #[inline(always)]
    fn read_u32(b: &[u8]) -> u32 {
        u32::from_le_bytes(b[..4].try_into().unwrap())
    }

    #[inline(always)]
    fn mix(a: u64, b: u64) -> u64 {
        let r = (a as u128).wrapping_mul(b as u128);
        (r as u64) ^ ((r >> 64) as u64)
    }

    #[inline]
    pub fn hash_key(bytes: &[u8]) -> u64 {
        let len = bytes.len();
        if len <= 16 {
            return if len >= 8 {
                mix(
                    read_u64(bytes) ^ P0,
                    read_u64(&bytes[len - 8..]) ^ P1 ^ len as u64,
                )
            } else if len >= 4 {
                mix(
                    read_u32(bytes) as u64 ^ P0,
                    read_u32(&bytes[len - 4..]) as u64 ^ P1 ^ len as u64,
                )
            } else if len == 0 {
                mix(P0, P1)
            } else {
                let a =
                    (bytes[0] as u64) << 16 | (bytes[len / 2] as u64) << 8 | bytes[len - 1] as u64;
                mix(a ^ P0, len as u64 ^ P1)
            };
        }
        let mut seed = mix(P0 ^ len as u64, P1);
        let mut i = 0;
        while i + 16 <= len {
            seed = mix(read_u64(&bytes[i..]) ^ P0, read_u64(&bytes[i + 8..]) ^ P1) ^ seed;
            i += 16;
        }
        // The final 16 bytes overlap the last full block, so this covers any
        // tail without a length switch.
        mix(
            read_u64(&bytes[len - 16..]) ^ P2,
            read_u64(&bytes[len - 8..]) ^ P3 ^ len as u64,
        ) ^ seed
    }
}

#[cfg(feature = "hash-rapid")]
mod imp {
    const SEED: u64 = 0xbdd8_9aa9_8270_4029;
    const S0: u64 = 0x2d35_8dcc_aa6c_78a5;
    const S1: u64 = 0x8bb8_4b93_962e_acc9;
    const S2: u64 = 0x4b33_a62e_d433_d4a3;

    #[inline(always)]
    fn read_u64(b: &[u8]) -> u64 {
        u64::from_le_bytes(b[..8].try_into().unwrap())
    }

    #[inline(always)]
    fn read_u32(b: &[u8]) -> u32 {
        u32::from_le_bytes(b[..4].try_into().unwrap())
    }

    #[inline(always)]
    fn mix(a: u64, b: u64) -> u64 {
        let r = (a as u128).wrapping_mul(b as u128);
        (r as u64) ^ ((r >> 64) as u64)
    }

    #[inline]
    pub fn hash_key(bytes: &[u8]) -> u64 {
        let len = bytes.len();
        let mut seed = SEED ^ len as u64;
        if len <= 16 {
            if len >= 4 {
                let a = read_u32(bytes) as u64;
                let b = read_u32(&bytes[len - 4..]) as u64;
                seed ^= mix(a ^ S0, b ^ S1) ^ len as u64;
            } else if len > 0 {
                let a =
                    (bytes[0] as u64) << 16 | (bytes[len / 2] as u64) << 8 | bytes[len - 1] as u64;
                seed ^= mix(a ^ S0, len as u64 ^ S1);
            }
            return mix(seed ^ S2, len as u64);
        }
        seed ^= mix(read_u64(bytes) ^ S0, read_u64(&bytes[8..]) ^ S1);
        let mut i = 16;
        while i + 16 <= len {
            seed = mix(read_u64(&bytes[i..]) ^ S0, read_u64(&bytes[i + 8..]) ^ S1) ^ seed;
            i += 16;
        }
        mix(
            read_u64(&bytes[len - 16..]) ^ S0,
            read_u64(&bytes[len - 8..]) ^ S1,
        ) ^ seed
            ^ len as u64
    }
}

#[cfg(not(any(feature = "hash-fx", feature = "hash-wy", feature = "hash-rapid")))]
compile_error!("select one of the hash-fx, hash-wy or hash-rapid features");

pub use imp::hash_key;
