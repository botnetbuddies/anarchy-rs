//! Small byte copies and exact short-key comparisons.

/// Exact equality for short usernames without a call to musl's bytewise memcmp.
/// Overlapping first/last words cover every byte, including unaligned slices.
#[inline(always)]
pub fn bytes_equal(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let n = a.len();
    if n > 16 {
        a == b
    } else if n >= 8 {
        u64::from_ne_bytes(a[..8].try_into().unwrap())
            == u64::from_ne_bytes(b[..8].try_into().unwrap())
            && u64::from_ne_bytes(a[n - 8..].try_into().unwrap())
                == u64::from_ne_bytes(b[n - 8..].try_into().unwrap())
    } else if n >= 4 {
        u32::from_ne_bytes(a[..4].try_into().unwrap())
            == u32::from_ne_bytes(b[..4].try_into().unwrap())
            && u32::from_ne_bytes(a[n - 4..].try_into().unwrap())
                == u32::from_ne_bytes(b[n - 4..].try_into().unwrap())
    } else if n >= 2 {
        u16::from_ne_bytes(a[..2].try_into().unwrap())
            == u16::from_ne_bytes(b[..2].try_into().unwrap())
            && a[n - 1] == b[n - 1]
    } else {
        n == 0 || a[0] == b[0]
    }
}

/// Append `s` to `out`.
///
/// Copies name fragments and format literals up to 16 bytes with inlined,
/// constant-size operations. Larger slices use `Vec::extend_from_slice`.
///
/// ## Safety
///
/// This is the *only* unsafe block in the program. It is sound because:
/// * `out.reserve(s.len())` guarantees spare capacity for `s.len()` more bytes,
/// * the 8/4/2/1-byte sub-copies use the overlapping-tail trick, so every byte
///   written lies within `[len, len + s.len())` which is reserved, and
/// * `s` must not alias `out` (all call sites pass independent buffers).
///
/// The raw pointer arithmetic never reads or writes out of bounds, and
/// `set_len` only exposes bytes that were just initialised.
#[inline(always)]
pub fn push_bytes(out: &mut Vec<u8>, s: &[u8]) {
    let len = out.len();
    if s.len() > 16 {
        out.extend_from_slice(s);
        return;
    }
    out.reserve(s.len());
    // SAFETY: `reserve` above made room for `s.len()` bytes at `out[len]`, and
    // every sub-copy below writes only inside that region. `s` is a distinct
    // allocation from `out`, so the non-overlapping copies are well-defined.
    unsafe {
        let n = s.len();
        let src = s.as_ptr();
        let dst = out.as_mut_ptr().add(len);
        if n >= 8 {
            std::ptr::copy_nonoverlapping(src, dst, 8);
            std::ptr::copy_nonoverlapping(src.add(n - 8), dst.add(n - 8), 8);
        } else if n >= 4 {
            std::ptr::copy_nonoverlapping(src, dst, 4);
            std::ptr::copy_nonoverlapping(src.add(n - 4), dst.add(n - 4), 4);
        } else if n >= 2 {
            std::ptr::copy_nonoverlapping(src, dst, 2);
            std::ptr::copy_nonoverlapping(src.add(n - 2), dst.add(n - 2), 2);
        } else if n == 1 {
            std::ptr::copy_nonoverlapping(src, dst, 1);
        }
        out.set_len(len + n);
    }
}

#[cfg(test)]
mod tests {
    use super::bytes_equal;

    #[test]
    fn short_equality_checks_every_byte_and_length() {
        for n in 0..=64 {
            let bytes: Vec<u8> = (0..n + 1).map(|i| (i * 71) as u8).collect();
            let a = &bytes[1..]; // Include unaligned word reads.
            assert!(bytes_equal(a, a));
            assert!(!bytes_equal(a, &bytes));
            for i in 0..n {
                let mut different = a.to_vec();
                different[i] ^= 1;
                assert!(!bytes_equal(a, &different), "length {n}, byte {i}");
            }
        }
    }
}
