//! What a minute of watching costs over a pinned path, and how fast a viewer may pull.
//!
//! In Foundation mode every hop charges its own `rate_msat` per round, and a circuit is closed the moment any hop's
//! per-round byte quota is exceeded (eltor `payment_circuit.c:448-465`). Hops count wire bytes on their links in
//! both directions, so guard and middle see about twice the payload and the last hop (the seeder) about once. Those
//! multipliers come from reading the code; M0 spike S2 measures them, and [`COUNT_FACTOR_INNER`] and
//! [`COUNT_FACTOR_LAST`] change with it.

use crate::terms::Terms;
use crate::{Error, Result};

/// Bytes an inner hop (guard, middle) counts per payload byte.
pub const COUNT_FACTOR_INNER: u64 = 2;
/// Bytes the last hop counts per payload byte.
pub const COUNT_FACTOR_LAST: u64 = 1;

/// Price of one round (one minute) across `path` (last hop last), in millisatoshis: every hop's rate, plus
/// `routing_fee_msat` for each hop that charges (zero-priced hops dispatch no payment).
pub fn per_round_msat(path: &[Terms], routing_fee_msat: u64) -> Result<u64> {
    path.iter().try_fold(0u64, |acc, t| {
        let fee = if t.rate_msat > 0 { routing_fee_msat } else { 0 };
        acc.checked_add(t.rate_msat).and_then(|v| v.checked_add(fee)).ok_or(Error::Overflow)
    })
}

/// The most payload bytes a viewer may pull in one round without exceeding any hop's quota, after keeping
/// `margin_permille` back (100 = 10 %) for TLS, cell and retry overhead. `None` means no hop sets a quota.
pub fn payload_budget_per_round(path: &[Terms], margin_permille: u64) -> Option<u64> {
    let last = path.len().checked_sub(1)?;
    let budget = path
        .iter()
        .enumerate()
        .filter(|(_, t)| t.paid_quota_bytes > 0)
        .map(|(i, t)| t.paid_quota_bytes / if i == last { COUNT_FACTOR_LAST } else { COUNT_FACTOR_INNER })
        .min()?;
    let keep = 1000u64.saturating_sub(margin_permille.min(1000));
    Some(budget / 1000 * keep + budget % 1000 * keep / 1000)
}

/// Whether one round's payload budget can carry `bits_per_second` of video in real time.
pub fn sustains(path: &[Terms], margin_permille: u64, bits_per_second: u64) -> bool {
    match payload_budget_per_round(path, margin_permille) {
        None => true,
        Some(budget) => bits_per_second.saturating_mul(60).div_ceil(8) <= budget,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hop(rate_msat: u64, paid_quota_bytes: u64) -> Terms {
        Terms { rate_msat, paid_quota_bytes, ..Terms::default() }
    }

    #[test]
    fn price_counts_fees_only_for_paid_hops() {
        let path = [hop(1000, 0), hop(0, 0), hop(5000, 0)];
        assert_eq!(per_round_msat(&path, 2000).unwrap(), 1000 + 2000 + 5000 + 2000);
        assert_eq!(per_round_msat(&[], 2000).unwrap(), 0);
        assert!(per_round_msat(&[hop(u64::MAX, 0), hop(1, 0)], 0).is_err());
    }

    #[test]
    fn budget_takes_the_tightest_hop() {
        // seeder 28.125 MB, middle 100 MB (counts 2x => 50 MB), guard unlimited.
        let path = [hop(1000, 0), hop(1000, 100_000_000), hop(5000, 28_125_000)];
        assert_eq!(payload_budget_per_round(&path, 0), Some(28_125_000));
        assert_eq!(payload_budget_per_round(&path, 100), Some(25_312_500));
        // a stingy middle (40 MB, counts 2x) becomes the limit.
        let path = [hop(1000, 0), hop(1000, 40_000_000), hop(5000, 28_125_000)];
        assert_eq!(payload_budget_per_round(&path, 0), Some(20_000_000));
        assert_eq!(payload_budget_per_round(&[hop(1, 0), hop(1, 0)], 100), None);
        assert_eq!(payload_budget_per_round(&[], 0), None);
    }

    #[test]
    fn sustains_720p() {
        let path = [hop(1000, 0), hop(1000, 0), hop(5000, 28_125_000)];
        assert!(sustains(&path, 100, 3_000_000)); // 22.5 MB/min <= 25.3 MB
        assert!(!sustains(&path, 100, 4_000_000)); // 30 MB/min > 25.3 MB
        assert!(sustains(&[hop(1, 0)], 100, u64::MAX));
    }
}
