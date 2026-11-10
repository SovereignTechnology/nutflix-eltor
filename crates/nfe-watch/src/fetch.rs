//! Fetching from a seeder: through SOCKS (El Tor) or directly (lab only), paced, and checked before use.
//!
//! Nothing fetched is trusted: the catalog is verified (nfe-catalog), the hash list against its `root`, address
//! and file count, and every file against its sha256 and listed size (NFX-05 §4).

use std::net::{SocketAddr, TcpStream};
use std::sync::Mutex;
use std::time::Duration;

use nfe_catalog::{Catalog, Video};
use nfx_proto::hashlist::{FileEntry, HashList, verify_file};
use nfx_proto::namespace::VideoAddr;

use crate::pacer::Pacer;
use crate::{Error, client, socks};

/// Hash lists larger than this are refused (a 2-hour title is under 2 MB).
pub const MAX_HASH_LIST: u64 = 8 * 1024 * 1024;

/// How to reach the seeder's content port.
#[derive(Debug, Clone)]
pub struct Remote {
    /// The seeder's content address (an IP literal: exit-to-self, ADR 0001).
    pub origin: SocketAddr,
    /// tor's SocksPort. `None` connects directly, which is for the lab only: it is unpaid and not anonymous.
    pub socks: Option<SocketAddr>,
    /// SOCKS credentials (tor uses them only for stream isolation).
    pub socks_auth: Option<(String, String)>,
    pub timeout: Duration,
}

impl Remote {
    fn open(&self) -> Result<TcpStream, Error> {
        let s = match self.socks {
            Some(proxy) => {
                let auth = self.socks_auth.as_ref().map(|(u, p)| (u.as_str(), p.as_str()));
                socks::connect(proxy, self.origin, auth, self.timeout)?
            }
            None => TcpStream::connect_timeout(&self.origin, self.timeout)?,
        };
        s.set_read_timeout(Some(self.timeout))?;
        s.set_write_timeout(Some(self.timeout))?;
        Ok(s)
    }

    fn get(&self, path: &str, max: u64) -> Result<Vec<u8>, Error> {
        let host = self.origin.ip().to_string();
        match client::get(self.open()?, &host, path, max)? {
            (200, body) => Ok(body),
            (code, _) => Err(Error::Status(code)),
        }
    }
}

pub struct Fetcher {
    remote: Remote,
    pacer: Mutex<Pacer>,
}

impl Fetcher {
    pub fn new(remote: Remote, pacer: Pacer) -> Self {
        Self { remote, pacer: Mutex::new(pacer) }
    }

    pub fn remote(&self) -> &Remote {
        &self.remote
    }

    /// GET `path`, reserving `max` bytes of the round's budget first (or the whole budget, if smaller: a body
    /// bigger than one round's budget cannot be fetched without tripping a quota, so it is refused as too large).
    fn paced_get(&self, path: &str, max: u64) -> Result<Vec<u8>, Error> {
        let reserve = {
            let mut p = self.pacer.lock().map_err(|_| Error::Poisoned)?;
            let reserve = p.budget().map_or(max, |b| b.min(max));
            p.reserve(reserve)?;
            reserve
        };
        let res = self.remote.get(path, reserve);
        let got = res.as_ref().map_or(0, |b| b.len() as u64);
        self.pacer.lock().map_err(|_| Error::Poisoned)?.refund(reserve - got.min(reserve));
        res
    }

    /// `/catalog.json`, verified at `now` (unix seconds). Bind it to the relay with [`Catalog::check_binding`].
    pub fn catalog(&self, now: u64) -> Result<Catalog, Error> {
        let bytes = self.paced_get("/catalog.json", nfe_catalog::MAX_BYTES as u64)?;
        Ok(Catalog::verify(&bytes, now)?)
    }

    /// The video's hash list, verified against its `root`, address and file count, with its exact bytes.
    pub fn hash_list(&self, video: &Video) -> Result<(HashList, Vec<u8>), Error> {
        let mut root = [0u8; 32];
        hex::decode_to_slice(&video.root, &mut root).map_err(|_| Error::Verify("root is not 64 hex".into()))?;
        let addr = VideoAddr::parse(&video.video).map_err(|e| Error::Verify(e.to_string()))?;
        let bytes = self.paced_get(&format!("/{}", video.root), MAX_HASH_LIST)?;
        let list = HashList::verify(&bytes, &root, &addr, video.segs).map_err(|e| Error::Verify(e.to_string()))?;
        Ok((list, bytes))
    }

    /// One listed file: exactly its listed size, and its sha256.
    pub fn file(&self, entry: &FileEntry) -> Result<Vec<u8>, Error> {
        if entry.size > nfe_seed::MAX_FILE {
            return Err(Error::Verify(format!("{} is larger than allowed", entry.name)));
        }
        let bytes = self.paced_get(&format!("/{}", entry.sha256), entry.size)?;
        if bytes.len() as u64 != entry.size {
            return Err(Error::Verify(format!("{} is not its listed size", entry.name)));
        }
        verify_file(&entry.sha256, &bytes).map_err(|e| Error::Verify(e.to_string()))?;
        Ok(bytes)
    }
}
