//! Token discovery and ordered substitution expansion.
use super::context::{exp_len, write_exp};
use super::{Ctx, ExpKind};
use crate::util;

/// Expansion tokens recognised by the original's post-substitution loop.
const EXP_TOKENS: [(&[u8], ExpKind); 8] = [
    (b"%givenname", ExpKind::Given),
    (b"%Givenname", ExpKind::GivenUp),
    (b"%surname", ExpKind::Surname),
    (b"%Surname", ExpKind::SurnameUp),
    (b"%a-z", ExpKind::AZ),
    (b"%A-Z", ExpKind::AZUp),
    (b"%DD", ExpKind::DD),
    (b"%D", ExpKind::D),
];

/// One expansion-token occurrence in a resolved string.
pub struct Occ {
    pub off: usize,
    pub len: usize,
    pub kind: ExpKind,
    /// Position in the rank-sorted odometer order (0 = outermost loop).
    pub odo: usize,
}

/// Whether `s` contains at least one expansion token.
///
/// Equivalent to `!tokenize(s).is_empty()`, but allocation-free; the common
/// no-substitution path only needs the yes/no answer.
pub fn has_token(s: &[u8]) -> bool {
    let mut i = 0usize;
    while i < s.len() {
        if s[i] == b'%' {
            for (tok, _) in EXP_TOKENS {
                if s[i..].starts_with(tok) {
                    return true;
                }
            }
        }
        i += 1;
    }
    false
}

/// Find every expansion token in the resolved string, in left-to-right order.
pub fn tokenize(s: &[u8]) -> Vec<Occ> {
    let mut raw: Vec<(usize, usize, ExpKind)> = Vec::new();
    let mut i = 0usize;
    while i < s.len() {
        if s[i] == b'%' {
            let mut matched = false;
            for (tok, kind) in EXP_TOKENS {
                if s[i..].starts_with(tok) {
                    raw.push((i, tok.len(), kind));
                    i += tok.len();
                    matched = true;
                    break;
                }
            }
            if !matched {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    // Odometer order: by (branch rank, offset).
    let mut order: Vec<usize> = (0..raw.len()).collect();
    order.sort_by(|&a, &b| {
        raw[a]
            .2
            .rank()
            .cmp(&raw[b].2.rank())
            .then(raw[a].0.cmp(&raw[b].0))
    });
    let mut odo_of = vec![0usize; raw.len()];
    for (k, &oi) in order.iter().enumerate() {
        odo_of[oi] = k;
    }
    raw.iter()
        .enumerate()
        .map(|(i, &(off, len, kind))| Occ {
            off,
            len,
            kind,
            odo: odo_of[i],
        })
        .collect()
}

pub fn expansion_total(occ: &[Occ], ctx: &Ctx) -> u64 {
    let mut total: u64 = 1;
    for o in occ {
        total = total.saturating_mul(exp_len(o.kind, ctx) as u64);
    }
    total
}

/// Enumerate expanded strings in the original's exact order.
pub fn enumerate<F: FnMut(&[u8])>(
    s: &[u8],
    occ: &[Occ],
    ctx: &Ctx,
    limit: u64,
    scratch: &mut Vec<u8>,
    mut f: F,
) {
    if occ.is_empty() {
        if limit == 0 {
            return;
        }
        scratch.clear();
        util::push_bytes(scratch, s);
        util::strip_spaces_and_apostrophes(scratch);
        f(scratch);
        return;
    }
    let lens: Vec<usize> = occ.iter().map(|o| exp_len(o.kind, ctx)).collect();
    if lens.contains(&0) {
        return;
    }
    let mut idx = vec![0usize; occ.len()];
    for _ in 0..limit {
        scratch.clear();
        let mut pos = 0usize;
        for o in occ.iter() {
            util::push_bytes(scratch, &s[pos..o.off]);
            write_exp(o.kind, idx[o.odo], ctx, scratch);
            pos = o.off + o.len;
        }
        util::push_bytes(scratch, &s[pos..]);
        util::strip_spaces_and_apostrophes(scratch);
        f(scratch);
        let mut k = occ.len();
        while k > 0 {
            k -= 1;
            let oi = occ[k].odo;
            idx[oi] += 1;
            if idx[oi] < lens[oi] {
                break;
            }
            idx[oi] = 0;
        }
    }
}

/// Locate the single token the original's `format` loop would expand next:
/// the one with the lowest branch rank, breaking ties by leftmost offset.
/// This is exactly the branch that `while (e =~ /…/)` selects each round.
pub fn first_branch(s: &[u8]) -> Option<(usize, usize, ExpKind)> {
    let mut best: Option<(u8, usize, usize, ExpKind)> = None;
    let mut i = 0usize;
    while i < s.len() {
        if s[i] == b'%' {
            let mut matched = false;
            for (tok, kind) in EXP_TOKENS {
                if s[i..].starts_with(tok) {
                    let rank = kind.rank();
                    let better = match best {
                        None => true,
                        Some((br, bo, _, _)) => (rank, i) < (br, bo),
                    };
                    if better {
                        best = Some((rank, i, tok.len(), kind));
                    }
                    i += tok.len();
                    matched = true;
                    break;
                }
            }
            if matched {
                continue;
            }
        }
        i += 1;
    }
    best.map(|(_, off, len, k)| (off, len, k))
}

/// The original's expansion loop, reproduced literally: repeatedly expand the
/// next branch token across the whole array, one round at a time. Fragments
/// produced by one round can reassemble a token for a later round (this is the
/// quirky `-F '%givenname'` case), so this cannot be reduced to a fixed-order
/// odometer. Only custom formats take this path.
pub fn expand_strict(mut arr: Vec<Vec<u8>>, ctx: &Ctx) -> Vec<Vec<u8>> {
    loop {
        let mut next: Vec<Vec<u8>> = Vec::with_capacity(arr.len().saturating_mul(2));
        let mut expanded = false;
        for e in arr {
            match first_branch(&e) {
                Some((off, len, kind)) => {
                    expanded = true;
                    let n = exp_len(kind, ctx);
                    for j in 0..n {
                        let mut s = Vec::with_capacity(e.len() + 8);
                        s.extend_from_slice(&e[..off]);
                        write_exp(kind, j, ctx, &mut s);
                        s.extend_from_slice(&e[off + len..]);
                        next.push(s);
                    }
                }
                // Finished entries move to the next round instead of being
                // cloned once per remaining expansion round.
                None => next.push(e),
            }
        }
        if !expanded {
            return next;
        }
        arr = next;
    }
}
