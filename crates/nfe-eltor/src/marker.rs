//! The seeder marker in a relay's ContactInfo (ADR 0001 section 2).
//!
//! `nutflix-eltor:v0 ck=<64 lowercase hex> port=<1-65535>`
//!
//! - `ck` is the x-only BIP-340 public key that signs the seeder's catalog. The relay's descriptor, which carries
//!   ContactInfo verbatim, is signed by the relay identity, so the key is bound to that relay.
//! - `port` is the content port the relay's exit policy accepts on its own address (exit-to-self).
//!
//! ContactInfo is free text, so the marker may sit among other words. Parsing finds the `nutflix-eltor:v0`
//! token and reads the `key=value` tokens right after it. Unknown keys are ignored (later versions may add
//! some), duplicates and missing required keys are errors.

use crate::{Error, Result};

pub const TOKEN: &str = "nutflix-eltor:v0";
/// Tor's descriptor cap is 20 000 bytes; refuse anything that could not have come from one.
const MAX_CONTACT: usize = 20_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marker {
    /// x-only public key, 64 lowercase hex characters.
    pub catalog_key: String,
    pub port: u16,
}

impl Marker {
    pub fn new(catalog_key: &str, port: u16) -> Result<Self> {
        if !is_lower_hex(catalog_key, 64) {
            return Err(Error::Marker("ck must be 64 lowercase hex characters"));
        }
        if port == 0 {
            return Err(Error::Marker("port must be 1-65535"));
        }
        Ok(Self { catalog_key: catalog_key.to_owned(), port })
    }

    /// The text to put in the relay's `ContactInfo` (alone, or appended to an operator contact).
    pub fn format(&self) -> String {
        format!("{TOKEN} ck={} port={}", self.catalog_key, self.port)
    }

    /// `None` if the ContactInfo carries no marker; `Some(Err)` if it carries a malformed one.
    pub fn find(contact: &str) -> Option<Result<Self>> {
        if contact.len() > MAX_CONTACT {
            return Some(Err(Error::Marker("contact info too long")));
        }
        let mut tokens = contact.split_ascii_whitespace();
        tokens.by_ref().find(|t| *t == TOKEN)?;
        let (mut ck, mut port) = (None, None);
        for t in tokens {
            let Some((k, v)) = t.split_once('=') else { break };
            let slot = match k {
                "ck" => &mut ck,
                "port" => &mut port,
                _ => continue,
            };
            if slot.replace(v).is_some() {
                return Some(Err(Error::Marker("duplicate key")));
            }
        }
        let (Some(ck), Some(port)) = (ck, port) else {
            return Some(Err(Error::Marker("ck and port are required")));
        };
        // Digits only: u16::from_str would accept a leading '+'.
        if port.is_empty() || port.len() > 5 || !port.bytes().all(|b| b.is_ascii_digit()) {
            return Some(Err(Error::Marker("port must be 1-65535")));
        }
        let Ok(port) = port.parse::<u16>() else {
            return Some(Err(Error::Marker("port must be 1-65535")));
        };
        Some(Self::new(ck, port))
    }
}

pub(crate) fn is_lower_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CK: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";

    #[test]
    fn round_trip() {
        let m = Marker::new(CK, 8443).unwrap();
        assert_eq!(m.format(), format!("nutflix-eltor:v0 ck={CK} port=8443"));
        assert_eq!(Marker::find(&m.format()).unwrap().unwrap(), m);
    }

    #[test]
    fn among_other_text() {
        let c = format!("ops at example dot org nutflix-eltor:v0 port=443 ck={CK} future=1 trailing words");
        assert_eq!(Marker::find(&c).unwrap().unwrap(), Marker::new(CK, 443).unwrap());
    }

    #[test]
    fn absent_and_malformed() {
        assert!(Marker::find("just an email").is_none());
        assert!(Marker::find("nutflix-eltor:v1 ck=x port=1").is_none());
        assert!(Marker::find(&format!("nutflix-eltor:v0 ck={CK}")).unwrap().is_err());
        assert!(Marker::find(&format!("nutflix-eltor:v0 ck={CK} port=0")).unwrap().is_err());
        assert!(Marker::find(&format!("nutflix-eltor:v0 ck={CK} port=65536")).unwrap().is_err());
        assert!(Marker::find(&format!("nutflix-eltor:v0 ck={CK} port=+443")).unwrap().is_err());
        assert!(Marker::find(&format!("nutflix-eltor:v0 ck={} port=443", CK.to_uppercase())).unwrap().is_err());
        assert!(Marker::find(&format!("nutflix-eltor:v0 ck={CK} ck={CK} port=443")).unwrap().is_err());
        assert!(Marker::find(&"a ".repeat(10_001)).unwrap().is_err());
    }
}
