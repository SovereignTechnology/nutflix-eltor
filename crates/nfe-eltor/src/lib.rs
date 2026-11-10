//! El Tor glue for nutflix-eltor (ADR 0001).
//!
//! - [`terms`]: `ElTorFoundationTerms`, the 55-byte canonical v1 policy a relay signs into its descriptor
//!   (rate per 60 s round, byte quota per round, max age, periods). eltord ships only a decoder; this is the
//!   encoder our seeder needs (patch P4 offers one upstream).
//! - [`marker`]: the seeder marker carried in a relay's ContactInfo (`nutflix-eltor:v0 ck=… port=…`).
//! - [`plan`]: the private Foundation client plan eltord reads (`src/client/runtime_config.rs`), with the same
//!   validation, written 0600.
//! - [`price`]: what a minute costs across a pinned path, and how many payload bytes a viewer may pull per round
//!   without tripping any hop's quota.
//!
//! Pure except [`plan::ClientPlan::write_private`] and [`plan::random_scope`].

pub mod marker;
pub mod plan;
pub mod price;
pub mod terms;

/// Errors from this crate. Messages name the rule broken, never secret material.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("invalid terms: {0}")]
    Terms(&'static str),
    #[error("invalid seeder marker: {0}")]
    Marker(&'static str),
    #[error("invalid client plan: {0}")]
    Plan(&'static str),
    #[error("invalid relay fingerprint")]
    Fingerprint,
    #[error("arithmetic overflow")]
    Overflow,
    #[error("i/o: {0}")]
    Io(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// A relay identity fingerprint as eltord's plan wants it: 40 lowercase hex characters, not all zero.
/// Accepts Tor's usual uppercase form and an optional leading `$`.
pub fn normalize_fingerprint(s: &str) -> Result<String> {
    let s = s.strip_prefix('$').unwrap_or(s);
    if s.len() != 40 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Error::Fingerprint);
    }
    let lower = s.to_ascii_lowercase();
    if lower.bytes().all(|b| b == b'0') {
        return Err(Error::Fingerprint);
    }
    Ok(lower)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprints_normalize() {
        let upper = "$ABCDEF0123456789ABCDEF0123456789ABCDEF01";
        assert_eq!(normalize_fingerprint(upper).unwrap(), "abcdef0123456789abcdef0123456789abcdef01");
        assert!(normalize_fingerprint(&"0".repeat(40)).is_err());
        assert!(normalize_fingerprint("abc").is_err());
        assert!(normalize_fingerprint(&"g".repeat(40)).is_err());
    }
}
