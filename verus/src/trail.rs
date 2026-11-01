// Physarum — the trail never blows up.
//
//   verus verus/src/trail.rs
//
// Every texel outside the holes moves `spread` of the way toward the mean of
// itself and its served neighbours, then loses `decay` of itself; storage
// rounds and clamps into a byte (docs/SPEC.md §3.2–3.3). This file proves, in
// integers:
//
//   3.2  a mean of values that are each at most M is at most M; a blend of
//        two values at most M is at most M; and decay keeps it there: one
//        pass never raises the highest texel;
//   3.3  the stored byte is always in 0..=255.
//
// ── The modelling assumptions ───────────────────────────────────────────────
//
// The runtime works in f32. Here a texel is a nat in thousandths, the spread
// is p thousandths and the decay d thousandths (0 <= p, d <= 1000), and the
// mean of a neighbourhood is the floor of its sum over its count (1..=9). Rounding in f32 is not modelled;
// the property test `diffusion_never_raises_the_peak` runs the f32 code
// against the same claim.

use vstd::prelude::*;

verus! {

pub open spec fn sum(s: Seq<nat>) -> nat
    decreases s.len(),
{
    if s.len() == 0 { 0 } else { sum(s.drop_last()) + s.last() }
}

proof fn lemma_sum_bounded(s: Seq<nat>, m: nat)
    requires
        forall|i: int| 0 <= i < s.len() ==> #[trigger] s[i] <= m,
    ensures
        sum(s) <= s.len() * m,
    decreases s.len(),
{
    if s.len() > 0 {
        assert forall|i: int| 0 <= i < s.drop_last().len() implies #[trigger] s.drop_last()[i] <= m by {
            assert(s.drop_last()[i] == s[i]);
        }
        lemma_sum_bounded(s.drop_last(), m);
        assert(s.last() <= m);
        assert(sum(s) == sum(s.drop_last()) + s.last());
        assert((s.len() - 1) * m + m == s.len() * m) by (nonlinear_arith);
    }
}

/// One texel's new value: `here` moved `p` of the way toward the mean of its
/// served neighbourhood `s`, then decayed by `d`.
pub open spec fn next(here: nat, s: Seq<nat>, p: nat, d: nat) -> nat
    recommends
        s.len() > 0,
        p <= 1000,
        d <= 1000,
{
    let blend = (((1000 - p) as nat) * here + p * (sum(s) / s.len())) / 1000;
    (blend * ((1000 - d) as nat)) / 1000
}

/// SPEC 3.2. A pass never raises the peak.
pub proof fn theorem_no_blow_up(here: nat, s: Seq<nat>, m: nat, p: nat, d: nat)
    requires
        1 <= s.len() <= 9,
        p <= 1000,
        d <= 1000,
        here <= m,
        forall|i: int| 0 <= i < s.len() ==> #[trigger] s[i] <= m,
    ensures
        next(here, s, p, d) <= m,
{
    lemma_sum_bounded(s, m);
    let n = s.len();
    let mean = sum(s) / n;
    assert(mean <= m) by (nonlinear_arith)
        requires sum(s) <= n * m, n >= 1, mean == sum(s) / n;
    let q = (1000 - p) as nat;
    let mix = q * here + p * mean;
    assert(q * here <= q * m) by (nonlinear_arith)
        requires here <= m;
    assert(p * mean <= p * m) by (nonlinear_arith)
        requires mean <= m;
    assert(q * m + p * m == 1000 * m) by (nonlinear_arith)
        requires q + p == 1000;
    assert(mix <= 1000 * m);
    let blend = mix / 1000;
    assert(blend <= m) by (nonlinear_arith)
        requires mix <= 1000 * m, blend == mix / 1000;
    let keep = (1000 - d) as nat;
    assert(blend * keep <= m * 1000) by (nonlinear_arith)
        requires blend <= m, keep <= 1000;
    assert((blend * keep) / 1000 <= m) by (nonlinear_arith)
        requires blend * keep <= m * 1000;
}

/// SPEC 3.3. The stored byte: round thousandths to units, clamp to 255.
pub fn store(v: u64) -> (b: u8)
    ensures
        b as int == if (v as int + 500) / 1000 > 255 { 255 } else { (v as int + 500) / 1000 },
{
    let units = v / 1000 + if v % 1000 >= 500 { 1 } else { 0 };
    assert(units as int == (v as int + 500) / 1000) by (nonlinear_arith)
        requires units == v / 1000 + if v % 1000 >= 500 { 1u64 } else { 0u64 };
    if units > 255 { 255 } else { units as u8 }
}

fn main() {
}

} // verus!
