//! nutflix-eltor catalog v0 (ADR 0002).
//!
//! A seeder serves one signed JSON catalog at `/catalog.json` on its content port. Viewers fetch it over the paid
//! circuit that ends at the seeder, then check it against what the El Tor directory says about that relay:
//!
//! - `pubkey` must equal the `ck` in the relay's ContactInfo marker (the descriptor is signed by the relay);
//! - `seeder` must equal the relay's fingerprint and `port` the marker's port;
//! - the signature, the shapes and the validity window must check.
//!
//! The signature is BIP-340 (the same keys and code as Nostr and NFX, via `nfx-proto`) over
//! `sha256("nutflix-eltor/catalog/v0" || 0x00 || canon(catalog without "sig"))`, where `canon` is NFX-11 §9
//! canonical JSON. Prices are not in the catalog: they are the relays' own signed Foundation terms.

use nfe_eltor::marker::Marker;
use nfx_proto::canon;
use nfx_proto::event::{public_key_hex, sign_digest, verify_digest};
use nfx_proto::hashlist::{HashList, Role};
use nfx_proto::namespace::VideoAddr;
use nfx_proto::sha256;
use serde::{Deserialize, Serialize};

pub const VERSION: u64 = 0;
pub const DOMAIN: &[u8] = b"nutflix-eltor/catalog/v0\0";
/// Limits a viewer enforces before parsing further.
pub const MAX_BYTES: usize = 1 << 20;
pub const MAX_VIDEOS: usize = 1000;
pub const MAX_TITLE: usize = 200;
pub const MAX_DESCRIPTION: usize = 5000;
pub const MAX_RENDITIONS: usize = 16;
/// Clock skew allowed on `issued_at`, in seconds.
pub const MAX_SKEW: u64 = 15 * 60;
/// Longest validity window a viewer accepts, in seconds.
pub const MAX_VALIDITY: u64 = 30 * 24 * 3600;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("catalog too large")]
    TooLarge,
    #[error("catalog is not valid JSON of the v0 shape")]
    Shape,
    #[error("invalid field: {0}")]
    Field(&'static str),
    #[error("catalog is not valid now")]
    Expired,
    #[error("bad signature")]
    Signature,
    #[error("catalog does not match the relay: {0}")]
    Binding(&'static str),
    #[error("{0}")]
    Proto(String),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenditionInfo {
    pub id: String,
    pub bandwidth: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Video {
    /// NFX-01 video address `nfx:<network>:<specver>:<video-id>`, the hash list's `video`.
    pub video: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// sha256 of the NFX-05 hash list bytes, which the seeder serves at `/<root>`.
    pub root: String,
    /// Content-addressed files in the hash list (NFX-02 `segs`).
    pub segs: u64,
    pub duration_ms: u64,
    pub renditions: Vec<RenditionInfo>,
}

impl Video {
    /// Describe a packaged video from its hash list (as `nfx-media` produced it).
    pub fn from_hash_list(hash_list: &HashList, root: &[u8; 32], title: &str, description: &str) -> Result<Self> {
        let segment_ms: u64 = hash_list.files.iter().filter(|f| f.role == Role::Segment).filter_map(|f| f.dur_ms).sum();
        // NFX-05: every rendition shares one GOP grid, so each carries the full duration.
        let duration_ms = segment_ms / (hash_list.renditions.len().max(1) as u64);
        let v = Self {
            video: hash_list.video.clone(),
            title: title.to_owned(),
            description: description.to_owned(),
            root: hex::encode(root),
            segs: hash_list.files.len() as u64,
            duration_ms,
            renditions: hash_list
                .renditions
                .iter()
                .map(|r| RenditionInfo { id: r.id.clone(), bandwidth: r.bandwidth, resolution: r.resolution.clone() })
                .collect(),
        };
        v.check()?;
        Ok(v)
    }

    fn check(&self) -> Result<()> {
        VideoAddr::parse(&self.video).map_err(|_| Error::Field("video"))?;
        if self.title.is_empty() || self.title.chars().count() > MAX_TITLE || self.title.chars().any(char::is_control) {
            return Err(Error::Field("title"));
        }
        if self.description.chars().count() > MAX_DESCRIPTION {
            return Err(Error::Field("description"));
        }
        if !is_lower_hex(&self.root, 64) {
            return Err(Error::Field("root"));
        }
        if self.segs == 0 || self.renditions.is_empty() || self.renditions.len() > MAX_RENDITIONS {
            return Err(Error::Field("renditions"));
        }
        if self.renditions.iter().any(|r| r.id.is_empty() || r.id.len() > 32 || r.bandwidth == 0) {
            return Err(Error::Field("renditions"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub v: u64,
    /// The seeder relay's fingerprint, 40 lowercase hex.
    pub seeder: String,
    /// The content port its exit policy accepts on its own address.
    pub port: u16,
    pub issued_at: u64,
    pub expires_at: u64,
    pub videos: Vec<Video>,
    /// x-only BIP-340 key, 64 lowercase hex; equals `ck` in the relay's marker.
    pub pubkey: String,
    #[serde(default)]
    pub sig: String,
}

impl Catalog {
    /// Build and sign a catalog.
    pub fn sign(
        secret_key: &[u8; 32],
        seeder_fingerprint: &str,
        port: u16,
        issued_at: u64,
        valid_for: u64,
        videos: Vec<Video>,
    ) -> Result<Self> {
        let seeder = nfe_eltor::normalize_fingerprint(seeder_fingerprint).map_err(|_| Error::Field("seeder"))?;
        let expires_at = issued_at.checked_add(valid_for).ok_or(Error::Field("expires_at"))?;
        let pubkey = public_key_hex(secret_key).map_err(|e| Error::Proto(e.to_string()))?;
        let mut c = Self { v: VERSION, seeder, port, issued_at, expires_at, videos, pubkey, sig: String::new() };
        c.check_shape()?;
        let digest = c.digest()?;
        c.sig = hex::encode(sign_digest(secret_key, &digest).map_err(|_| Error::Signature)?);
        Ok(c)
    }

    pub fn to_json(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(self).map_err(|_| Error::Shape)
    }

    /// Parse and verify received bytes: size, shape, signature, then the validity window at `now` (unix seconds).
    /// The result still has to be bound to a relay with [`Catalog::check_binding`].
    pub fn verify(bytes: &[u8], now: u64) -> Result<Self> {
        if bytes.len() > MAX_BYTES {
            return Err(Error::TooLarge);
        }
        // Canonicalizable input only (no duplicate keys, floats or huge integers), as NFX does.
        let text = std::str::from_utf8(bytes).map_err(|_| Error::Shape)?;
        canon::Value::parse(text).map_err(|_| Error::Shape)?;
        let c: Self = serde_json::from_str(text).map_err(|_| Error::Shape)?;
        c.check_shape()?;
        if !is_lower_hex(&c.sig, 128) {
            return Err(Error::Signature);
        }
        let digest = c.digest()?;
        verify_digest(&c.pubkey, &digest, &c.sig).map_err(|_| Error::Signature)?;
        if c.issued_at > now.saturating_add(MAX_SKEW) || now > c.expires_at {
            return Err(Error::Expired);
        }
        Ok(c)
    }

    /// Bind a verified catalog to the relay the viewer is about to pay: its fingerprint and its ContactInfo marker.
    pub fn check_binding(&self, relay_fingerprint: &str, marker: &Marker) -> Result<()> {
        let fp = nfe_eltor::normalize_fingerprint(relay_fingerprint).map_err(|_| Error::Binding("fingerprint"))?;
        if self.seeder != fp {
            return Err(Error::Binding("seeder fingerprint"));
        }
        if self.pubkey != marker.catalog_key {
            return Err(Error::Binding("catalog key"));
        }
        if self.port != marker.port {
            return Err(Error::Binding("content port"));
        }
        Ok(())
    }

    fn check_shape(&self) -> Result<()> {
        if self.v != VERSION {
            return Err(Error::Field("v"));
        }
        if !is_lower_hex(&self.seeder, 40) {
            return Err(Error::Field("seeder"));
        }
        if self.port == 0 {
            return Err(Error::Field("port"));
        }
        if self.expires_at <= self.issued_at || self.expires_at - self.issued_at > MAX_VALIDITY {
            return Err(Error::Field("expires_at"));
        }
        if !is_lower_hex(&self.pubkey, 64) {
            return Err(Error::Field("pubkey"));
        }
        if self.videos.len() > MAX_VIDEOS {
            return Err(Error::Field("videos"));
        }
        let mut seen = std::collections::HashSet::new();
        for v in &self.videos {
            v.check()?;
            if !seen.insert(v.video.as_str()) {
                return Err(Error::Field("videos (duplicate address)"));
            }
        }
        Ok(())
    }

    /// `sha256(DOMAIN || canon(self without sig))`.
    fn digest(&self) -> Result<[u8; 32]> {
        let mut value = serde_json::to_value(self).map_err(|_| Error::Shape)?;
        value.as_object_mut().ok_or(Error::Shape)?.remove("sig");
        let canon = canon::Value::from_json(&value).map_err(|e| Error::Proto(e.to_string()))?.to_canon();
        let mut msg = Vec::with_capacity(DOMAIN.len() + canon.len());
        msg.extend_from_slice(DOMAIN);
        msg.extend_from_slice(canon.as_bytes());
        Ok(sha256(&msg))
    }
}

fn is_lower_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SK: [u8; 32] = [7u8; 32];
    const FP: &str = "ABCDEF0123456789ABCDEF0123456789ABCDEF01";

    fn video(id: &str) -> Video {
        Video {
            video: format!("nfx:eltor:1:video-{id}"),
            title: format!("Title {id}"),
            description: "A test video.".into(),
            root: "ab".repeat(32),
            segs: 12,
            duration_ms: 20_000,
            renditions: vec![RenditionInfo {
                id: "720p".into(),
                bandwidth: 3_000_000,
                resolution: Some("1280x720".into()),
            }],
        }
    }

    fn catalog() -> Catalog {
        Catalog::sign(&SK, FP, 8443, 1_800_000_000, 86_400, vec![video("a"), video("b")]).unwrap()
    }

    #[test]
    fn sign_verify_bind() {
        let c = catalog();
        let bytes = c.to_json().unwrap();
        let v = Catalog::verify(&bytes, 1_800_000_100).unwrap();
        assert_eq!(v, c);
        let m = Marker::new(&c.pubkey, 8443).unwrap();
        v.check_binding(FP, &m).unwrap();
        assert_eq!(v.check_binding(&FP.replace('A', "B"), &m), Err(Error::Binding("seeder fingerprint")));
        assert_eq!(v.check_binding(FP, &Marker::new(&c.pubkey, 443).unwrap()), Err(Error::Binding("content port")));
        let other = Marker::new(&public_key_hex(&[9u8; 32]).unwrap(), 8443).unwrap();
        assert_eq!(v.check_binding(FP, &other), Err(Error::Binding("catalog key")));
    }

    #[test]
    fn signature_is_deterministic_and_covers_everything() {
        assert_eq!(catalog().sig, catalog().sig);
        let c = catalog();
        let mut tampered = c.clone();
        tampered.videos[0].title = "Other".into();
        assert_eq!(Catalog::verify(&tampered.to_json().unwrap(), 1_800_000_100), Err(Error::Signature));
        let mut tampered = c.clone();
        tampered.port = 8444;
        assert_eq!(Catalog::verify(&tampered.to_json().unwrap(), 1_800_000_100), Err(Error::Signature));
        let mut unsigned = c;
        unsigned.sig = String::new();
        assert_eq!(Catalog::verify(&unsigned.to_json().unwrap(), 1_800_000_100), Err(Error::Signature));
    }

    #[test]
    fn validity_window() {
        let bytes = catalog().to_json().unwrap();
        assert!(Catalog::verify(&bytes, 1_800_000_000 - MAX_SKEW).is_ok());
        assert_eq!(Catalog::verify(&bytes, 1_800_000_000 - MAX_SKEW - 1), Err(Error::Expired));
        assert!(Catalog::verify(&bytes, 1_800_086_400).is_ok());
        assert_eq!(Catalog::verify(&bytes, 1_800_086_401), Err(Error::Expired));
        assert!(Catalog::sign(&SK, FP, 8443, 1, MAX_VALIDITY + 1, vec![]).is_err());
        assert!(Catalog::sign(&SK, FP, 8443, 1, 0, vec![]).is_err());
    }

    #[test]
    fn strict_parsing() {
        let c = catalog();
        let text = String::from_utf8(c.to_json().unwrap()).unwrap();
        // unknown field
        let extra = text.replacen("{\"v\":0", "{\"v\":0,\"x\":1", 1);
        assert_eq!(Catalog::verify(extra.as_bytes(), 1_800_000_100), Err(Error::Shape));
        // duplicate key
        let dup = text.replacen("{\"v\":0", "{\"v\":0,\"v\":0", 1);
        assert_eq!(Catalog::verify(dup.as_bytes(), 1_800_000_100), Err(Error::Shape));
        // float
        let float = text.replacen("\"port\":8443", "\"port\":8443.0", 1);
        assert_eq!(Catalog::verify(float.as_bytes(), 1_800_000_100), Err(Error::Shape));
        assert_eq!(Catalog::verify(&vec![b' '; MAX_BYTES + 1], 0), Err(Error::TooLarge));
    }

    #[test]
    fn video_fields() {
        let mut v = video("a");
        v.video = "not-an-address".into();
        assert!(Catalog::sign(&SK, FP, 8443, 1, 10, vec![v]).is_err());
        let mut v = video("a");
        v.title = "x".repeat(MAX_TITLE + 1);
        assert!(Catalog::sign(&SK, FP, 8443, 1, 10, vec![v]).is_err());
        let mut v = video("a");
        v.title = "bell\u{7}".into();
        assert!(Catalog::sign(&SK, FP, 8443, 1, 10, vec![v]).is_err());
        assert!(Catalog::sign(&SK, FP, 8443, 1, 10, vec![video("a"), video("a")]).is_err());
        assert!(Catalog::sign(&SK, FP, 0, 1, 10, vec![]).is_err());
    }
}
