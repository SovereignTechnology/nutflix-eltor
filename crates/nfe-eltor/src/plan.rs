//! The private Foundation client plan eltord reads (`eltord/src/client/runtime_config.rs` at 9f589be).
//!
//! eltord refuses a plan unless: `fingerprints` holds 1–16 unique 40-lowercase-hex ids (not all zero);
//! `scope` is 64 lowercase hex (not all zero); `routing_fee_per_hop_msat ≤ per_round_msat ≤ outstanding_msat
//! ≤ lifetime_msat`; `handshake_msat ≤ outstanding_msat`; a positive `handshake_msat` comes with
//! `handshake_funding_timeout_ms` in 1..=300000; the file is at most 8192 bytes, a regular file, not a symlink, and
//! has no group or other permission bits. [`ClientPlan::new`] checks the same rules, so a plan we write is a plan
//! eltord accepts.
//!
//! The plan is the viewer's spending ceiling. It is account-wide in eltord today (changing it later is refused),
//! so pick limits for the session, not for one video (patch P3 adds per-session caps).

use std::collections::HashSet;
use std::path::Path;

use serde::Serialize;

use crate::marker::is_lower_hex;
use crate::{Error, Result, normalize_fingerprint};

const MAX_FILE: usize = 8192;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClientPlan {
    pub scope: String,
    pub fingerprints: Vec<String>,
    pub lifetime_msat: u64,
    pub outstanding_msat: u64,
    pub per_round_msat: u64,
    pub routing_fee_per_hop_msat: u64,
    pub handshake_msat: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handshake_funding_timeout_ms: Option<u64>,
}

/// Spending limits for a plan, all in millisatoshis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Total the account may ever spend.
    pub lifetime_msat: u64,
    /// Unsettled payments in flight at once.
    pub outstanding_msat: u64,
    /// Ceiling for one round across all paid hops, routing fees included.
    pub per_round_msat: u64,
    /// Routing fee budget per paid hop and round.
    pub routing_fee_per_hop_msat: u64,
}

impl ClientPlan {
    /// `path` is the ordered relay path, last hop last (for nutflix-eltor: guard, middle, seeder).
    pub fn new(scope: &str, path: &[&str], limits: Limits) -> Result<Self> {
        if !is_lower_hex(scope, 64) || scope.bytes().all(|b| b == b'0') {
            return Err(Error::Plan("scope must be 64 lowercase hex characters, not all zero"));
        }
        if path.is_empty() || path.len() > 16 {
            return Err(Error::Plan("a path has 1-16 relays"));
        }
        let fingerprints = path.iter().map(|f| normalize_fingerprint(f)).collect::<Result<Vec<_>>>()?;
        if fingerprints.iter().collect::<HashSet<_>>().len() != fingerprints.len() {
            return Err(Error::Plan("a relay appears twice in the path"));
        }
        let Limits { lifetime_msat, outstanding_msat, per_round_msat, routing_fee_per_hop_msat } = limits;
        if !(routing_fee_per_hop_msat <= per_round_msat
            && per_round_msat <= outstanding_msat
            && outstanding_msat <= lifetime_msat)
        {
            return Err(Error::Plan("need routing fee <= per round <= outstanding <= lifetime"));
        }
        Ok(Self {
            scope: scope.to_owned(),
            fingerprints,
            lifetime_msat,
            outstanding_msat,
            per_round_msat,
            routing_fee_per_hop_msat,
            handshake_msat: 0,
            handshake_funding_timeout_ms: None,
        })
    }

    pub fn to_json(&self) -> Result<Vec<u8>> {
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| Error::Io(e.to_string()))?;
        if bytes.len() > MAX_FILE {
            return Err(Error::Plan("plan larger than 8192 bytes"));
        }
        Ok(bytes)
    }

    /// Write the plan as eltord wants it: a fresh 0600 file (`create_new` is `O_CREAT|O_EXCL`, which never
    /// follows a symlink), then renamed over `dest` (rename replaces a symlink itself, not its target).
    #[cfg(unix)]
    pub fn write_private(&self, dest: &Path) -> Result<()> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;

        let bytes = self.to_json()?;
        let dir = dest.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
        let name = dest.file_name().ok_or(Error::Plan("plan path has no file name"))?;
        let tmp = dir.join(format!(".{}.tmp-{}", name.to_string_lossy(), std::process::id()));
        let io = |e: std::io::Error| Error::Io(e.to_string());
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp).map_err(io)?;
        let res = f.write_all(&bytes).and_then(|()| f.sync_all());
        drop(f);
        if let Err(e) = res.and_then(|()| std::fs::rename(&tmp, dest)) {
            let _ = std::fs::remove_file(&tmp);
            return Err(io(e));
        }
        Ok(())
    }
}

/// A fresh random scope (one per viewer account; eltord keys its ledger on it).
pub fn random_scope() -> Result<String> {
    let mut b = [0u8; 32];
    loop {
        getrandom::fill(&mut b).map_err(|e| Error::Io(e.to_string()))?;
        if b != [0; 32] {
            return Ok(hex::encode(b));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const G: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA1";
    const M: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb2";
    const S: &str = "$CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC3";
    const LIM: Limits = Limits {
        lifetime_msat: 1_000_000,
        outstanding_msat: 20_000,
        per_round_msat: 10_000,
        routing_fee_per_hop_msat: 2_000,
    };

    #[test]
    fn valid_plan_serializes_like_eltord_reads_it() {
        let scope = "11".repeat(32);
        let p = ClientPlan::new(&scope, &[G, M, S], LIM).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&p.to_json().unwrap()).unwrap();
        assert_eq!(v["fingerprints"][0], "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa1");
        assert_eq!(v["fingerprints"][2], "ccccccccccccccccccccccccccccccccccccccc3");
        assert_eq!(v["per_round_msat"], 10_000);
        assert!(v.get("handshake_funding_timeout_ms").is_none());
        let keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
        for k in &keys {
            assert!(
                [
                    "scope",
                    "fingerprints",
                    "lifetime_msat",
                    "outstanding_msat",
                    "per_round_msat",
                    "routing_fee_per_hop_msat",
                    "handshake_msat"
                ]
                .contains(&k.as_str()),
                "eltord denies unknown field {k}"
            );
        }
    }

    #[test]
    fn eltord_rules() {
        let scope = "11".repeat(32);
        assert!(ClientPlan::new(&"00".repeat(32), &[G], LIM).is_err());
        assert!(ClientPlan::new(&"ab".repeat(32).to_uppercase(), &[G], LIM).is_err());
        assert!(ClientPlan::new(&scope, &[], LIM).is_err());
        assert!(ClientPlan::new(&scope, &[G, G], LIM).is_err());
        assert!(ClientPlan::new(&scope, &[G; 17], LIM).is_err());
        assert!(ClientPlan::new(&scope, &[G], Limits { routing_fee_per_hop_msat: 10_001, ..LIM }).is_err());
        assert!(ClientPlan::new(&scope, &[G], Limits { per_round_msat: 20_001, ..LIM }).is_err());
        assert!(ClientPlan::new(&scope, &[G], Limits { outstanding_msat: 1_000_001, ..LIM }).is_err());
    }

    #[test]
    fn written_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("nfe-plan-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("plan.json");
        let p = ClientPlan::new(&random_scope().unwrap(), &[G, M, S], LIM).unwrap();
        p.write_private(&dest).unwrap();
        p.write_private(&dest).unwrap(); // replaces
        let meta = std::fs::symlink_metadata(&dest).unwrap();
        assert!(meta.is_file());
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::read(&dest).unwrap(), p.to_json().unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
