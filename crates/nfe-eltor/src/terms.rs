//! `ElTorFoundationTerms`: canonical finite v1 policy terms.
//!
//! 55 bytes, unsigned big-endian, written as 110 lowercase hex characters. Layout and rules from El Tor:
//! eltor `src/feature/payment/payment_profile.c` (encoder 55-75, decoder 77-102, validity 48-53 and 86-89),
//! eltord `spec/01a_circuit_renewal_wire.md` and `spec/vectors/payment_renewal_v1.json`.
//!
//! | offset | size | field |
//! |---:|---:|---|
//! | 0 | 1 | version = 1 |
//! | 1 | 1 | mode = 1 |
//! | 2 | 1 | batch_count = 10 (rounds per period) |
//! | 3 | 8 | rate_msat per 60 s round |
//! | 11 | 4 | round_ms = 60000 |
//! | 15 | 4 | lead_ms = 60000 |
//! | 19 | 4 | grace_ms ≤ 60000 |
//! | 23 | 8 | paid_quota_bytes per round (0 = unlimited) |
//! | 31 | 8 | max_age_ms > 0 |
//! | 39 | 4 | max_periods ≥ 2 |
//! | 43 | 8 | initial_handshake_fee_msat |
//! | 51 | 4 | setup_timeout_ms > 0 |

use crate::{Error, Result};

pub const TERMS_LEN: usize = 55;
pub const ROUND_MS: u32 = 60_000;
pub const LEAD_MS: u32 = 60_000;
pub const ROUNDS_PER_PERIOD: u8 = 10;
pub const PERIOD_MS: u64 = ROUND_MS as u64 * ROUNDS_PER_PERIOD as u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Terms {
    /// Price per 60 s round, in millisatoshis. 0 = zero-priced.
    pub rate_msat: u64,
    /// Bytes one paid round buys at this relay. 0 = unlimited.
    pub paid_quota_bytes: u64,
    pub grace_ms: u32,
    pub max_age_ms: u64,
    /// 10-round allocations a circuit may use, period 0 included.
    pub max_periods: u32,
    pub handshake_fee_msat: u64,
    pub setup_timeout_ms: u32,
}

impl Default for Terms {
    /// The zero-price testing policy a relay uses when `ElTorFoundationTerms` is omitted
    /// (`payment_profile.c:10-16`).
    fn default() -> Self {
        Self {
            rate_msat: 0,
            paid_quota_bytes: 0,
            grace_ms: 15_000,
            max_age_ms: 1_800_000,
            max_periods: 3,
            handshake_fee_msat: 0,
            setup_timeout_ms: 30_000,
        }
    }
}

impl Terms {
    pub fn validate(&self) -> Result<()> {
        if self.grace_ms > 60_000 {
            return Err(Error::Terms("grace_ms must be at most 60000"));
        }
        if self.max_age_ms == 0 {
            return Err(Error::Terms("max_age_ms must be positive"));
        }
        if self.max_periods < 2 {
            return Err(Error::Terms("max_periods must be at least 2"));
        }
        if self.setup_timeout_ms == 0 {
            return Err(Error::Terms("setup_timeout_ms must be positive"));
        }
        Ok(())
    }

    pub fn to_bytes(&self) -> Result<[u8; TERMS_LEN]> {
        self.validate()?;
        let mut b = Vec::with_capacity(TERMS_LEN);
        b.extend_from_slice(&[1, 1, ROUNDS_PER_PERIOD]);
        b.extend_from_slice(&self.rate_msat.to_be_bytes());
        b.extend_from_slice(&ROUND_MS.to_be_bytes());
        b.extend_from_slice(&LEAD_MS.to_be_bytes());
        b.extend_from_slice(&self.grace_ms.to_be_bytes());
        b.extend_from_slice(&self.paid_quota_bytes.to_be_bytes());
        b.extend_from_slice(&self.max_age_ms.to_be_bytes());
        b.extend_from_slice(&self.max_periods.to_be_bytes());
        b.extend_from_slice(&self.handshake_fee_msat.to_be_bytes());
        b.extend_from_slice(&self.setup_timeout_ms.to_be_bytes());
        b.try_into().map_err(|_| Error::Terms("encoded length is not 55"))
    }

    /// 110 lowercase hex characters, the value of `ElTorFoundationTerms` in a relay's torrc.
    pub fn encode(&self) -> Result<String> {
        Ok(hex::encode(self.to_bytes()?))
    }

    pub fn from_bytes(b: &[u8; TERMS_LEN]) -> Result<Self> {
        let u32_at = |i: usize| u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        let u64_at = |i: usize| {
            let mut a = [0u8; 8];
            a.copy_from_slice(&b[i..i + 8]);
            u64::from_be_bytes(a)
        };
        if b[0] != 1 || b[1] != 1 || b[2] != ROUNDS_PER_PERIOD || u32_at(11) != ROUND_MS || u32_at(15) != LEAD_MS {
            return Err(Error::Terms("not canonical v1 terms"));
        }
        let t = Self {
            rate_msat: u64_at(3),
            grace_ms: u32_at(19),
            paid_quota_bytes: u64_at(23),
            max_age_ms: u64_at(31),
            max_periods: u32_at(39),
            handshake_fee_msat: u64_at(43),
            setup_timeout_ms: u32_at(51),
        };
        t.validate()?;
        Ok(t)
    }

    /// Strict, like El Tor's parser: exactly 110 lowercase hex characters.
    pub fn decode(s: &str) -> Result<Self> {
        if s.len() != 2 * TERMS_LEN || !s.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)) {
            return Err(Error::Terms("terms must be 110 lowercase hex characters"));
        }
        let mut b = [0u8; TERMS_LEN];
        hex::decode_to_slice(s, &mut b).map_err(|_| Error::Terms("bad hex"))?;
        Self::from_bytes(&b)
    }

    /// Minutes one circuit can be funded for. Renewing period `p` needs `p < max_periods` and
    /// `p * 600000 + 600000 <= max_age_ms` (`payment_profile.c:397-400`).
    pub fn usable_minutes(&self) -> u64 {
        u64::from(self.max_periods).min(self.max_age_ms / PERIOD_MS) * u64::from(ROUNDS_PER_PERIOD)
    }

    /// Terms whose max age and periods cover `minutes` of play, in whole 10-round periods.
    pub fn for_long_play(rate_msat: u64, minutes: u64, paid_quota_bytes: u64) -> Result<Self> {
        let periods = minutes.div_ceil(u64::from(ROUNDS_PER_PERIOD)).max(2);
        let max_periods = u32::try_from(periods).map_err(|_| Error::Overflow)?;
        let max_age_ms = periods.checked_mul(PERIOD_MS).ok_or(Error::Overflow)?;
        let t = Self { rate_msat, paid_quota_bytes, max_age_ms, max_periods, ..Self::default() };
        t.validate()?;
        Ok(t)
    }
}

/// Per-round byte quota for a seeder whose top rendition peaks at `bits_per_second`, with `headroom_permille`
/// (1250 = 1.25×) so a viewer can buffer ahead. The last hop counts payload bytes about once.
pub fn quota_for_bitrate(bits_per_second: u64, headroom_permille: u64) -> Result<u64> {
    if bits_per_second == 0 || headroom_permille < 1000 {
        return Err(Error::Terms("bitrate must be positive and headroom at least 1000 permille"));
    }
    let seconds = u64::from(ROUND_MS / 1000);
    let num =
        bits_per_second.checked_mul(seconds).and_then(|v| v.checked_mul(headroom_permille)).ok_or(Error::Overflow)?;
    Ok(num.div_ceil(8 * 1000))
}

#[cfg(test)]
mod tests {
    use super::*;

    // eltord spec/vectors/payment_renewal_v1.json at 9f589be, fixtures[0..2].terms_hex.
    const SPEC_HOP1: &str = "01010a00000000000003e80000ea600000ea6000003a9800000000000000000000000005265c0000000090000000000000000000007530";
    const SPEC_HOP2: &str = "01010a00000000000007d00000ea600000ea6000003a9800000000000000000000000005265c0000000090000000000000000000007530";
    // payment_profile.c:10-16, the zero-price default.
    const ZERO_DEFAULT: &str = "01010a00000000000000000000ea600000ea6000003a98000000000000000000000000001b774000000003000000000000000000007530";

    #[test]
    fn spec_vectors() {
        let day = 24 * 3600 * 1000;
        let hop1 = Terms { rate_msat: 1000, max_age_ms: day, max_periods: 144, ..Terms::default() };
        let hop2 = Terms { rate_msat: 2000, ..hop1 };
        assert_eq!(hop1.encode().unwrap(), SPEC_HOP1);
        assert_eq!(hop2.encode().unwrap(), SPEC_HOP2);
        assert_eq!(Terms::default().encode().unwrap(), ZERO_DEFAULT);
        for h in [SPEC_HOP1, SPEC_HOP2, ZERO_DEFAULT] {
            assert_eq!(Terms::decode(h).unwrap().encode().unwrap(), h);
        }
    }

    #[test]
    fn decode_is_strict() {
        assert!(Terms::decode(&SPEC_HOP1.to_uppercase()).is_err());
        assert!(Terms::decode(&SPEC_HOP1[..108]).is_err());
        assert!(Terms::decode(&format!("02{}", &SPEC_HOP1[2..])).is_err());
        assert!(Terms::decode(&format!("{}09{}", &SPEC_HOP1[..4], &SPEC_HOP1[6..])).is_err());
        // round_ms 59999
        assert!(Terms::decode(&format!("{}0000ea5f{}", &SPEC_HOP1[..22], &SPEC_HOP1[30..])).is_err());
    }

    #[test]
    fn validation() {
        assert!(Terms { grace_ms: 60_001, ..Terms::default() }.encode().is_err());
        assert!(Terms { max_periods: 1, ..Terms::default() }.encode().is_err());
        assert!(Terms { max_age_ms: 0, ..Terms::default() }.encode().is_err());
        assert!(Terms { setup_timeout_ms: 0, ..Terms::default() }.encode().is_err());
    }

    #[test]
    fn quota_and_long_play() {
        assert_eq!(quota_for_bitrate(3_000_000, 1250).unwrap(), 28_125_000);
        assert!(quota_for_bitrate(0, 1250).is_err());
        assert!(quota_for_bitrate(1, 999).is_err());
        let t = Terms::for_long_play(1000, 120, 28_125_000).unwrap();
        assert_eq!((t.max_periods, t.max_age_ms, t.usable_minutes()), (12, 7_200_000, 120));
        assert_eq!(Terms::decode(&t.encode().unwrap()).unwrap(), t);
        assert_eq!(Terms::default().usable_minutes(), 30);
        assert_eq!(Terms::for_long_play(1, 5, 0).unwrap().max_periods, 2);
    }
}
