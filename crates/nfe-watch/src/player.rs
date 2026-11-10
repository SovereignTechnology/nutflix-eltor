//! The viewer's local HLS proxy: a player on this machine points at `http://127.0.0.1:<port>/<root>/master.m3u8`.
//!
//! It answers the same NFX-05 §6 surface as the seeder, for one hash list only, from a small cache or by fetching
//! through the [`Fetcher`] (paced, over El Tor). Every file is checked against the hash list before it is served,
//! and every playlist must name only listed content (`HashList::check_playlist`), so a bad seeder can make playback
//! fail but cannot make the player load anything that is not in the signed catalog's hash list.

use std::collections::{HashMap, VecDeque};
use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use nfe_seed::route::{Route, content_type};
use nfe_seed::server::{Handler, Options, Response, Server};
use nfx_proto::hashlist::{FileEntry, HashList, Role};

use crate::fetch::Fetcher;

/// Bytes of fetched files kept in memory.
pub const CACHE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Default)]
struct Cache {
    map: HashMap<String, Arc<Vec<u8>>>,
    order: VecDeque<String>,
    bytes: u64,
}

impl Cache {
    fn get(&self, sha256: &str) -> Option<Arc<Vec<u8>>> {
        self.map.get(sha256).cloned()
    }

    fn put(&mut self, sha256: &str, body: Arc<Vec<u8>>) {
        if self.map.contains_key(sha256) {
            return;
        }
        self.bytes += body.len() as u64;
        self.map.insert(sha256.to_owned(), body);
        self.order.push_back(sha256.to_owned());
        while self.bytes > CACHE_BYTES {
            let Some(old) = self.order.pop_front() else { break };
            if let Some(b) = self.map.remove(&old) {
                self.bytes -= b.len() as u64;
            }
        }
    }
}

pub struct Player {
    fetcher: Arc<Fetcher>,
    root: String,
    list: HashList,
    list_bytes: Arc<Vec<u8>>,
    cache: Mutex<Cache>,
}

impl Player {
    /// `list` and `list_bytes` must be the verified hash list whose sha256 is `root` (from [`Fetcher::hash_list`]).
    pub fn new(fetcher: Arc<Fetcher>, root: &str, list: HashList, list_bytes: Vec<u8>) -> Self {
        let list_bytes = Arc::new(list_bytes);
        Self { fetcher, root: root.to_owned(), list, list_bytes, cache: Mutex::new(Cache::default()) }
    }

    fn entry(&self, sha256: &str) -> Option<&FileEntry> {
        self.list.files.iter().find(|f| f.sha256 == sha256)
    }

    fn load(&self, entry: &FileEntry) -> Option<Arc<Vec<u8>>> {
        if let Some(b) = self.cache.lock().ok()?.get(&entry.sha256) {
            return Some(b);
        }
        let bytes = self.fetcher.file(entry).ok()?;
        if matches!(entry.role, Role::PlaylistMaster | Role::Playlist) && self.list.check_playlist(&bytes).is_err() {
            return None;
        }
        let bytes = Arc::new(bytes);
        self.cache.lock().ok()?.put(&entry.sha256, Arc::clone(&bytes));
        Some(bytes)
    }

    fn serve(&self, entry: &FileEntry, ext: Option<&str>) -> Response {
        match self.load(entry) {
            Some(b) => Response::ok(content_type(Some(entry.role), ext), b, true),
            None => Response::status(502),
        }
    }

    pub fn answer(&self, route: &Route) -> Response {
        match route {
            Route::File { sha256, ext: None } if *sha256 == self.root => {
                Response::ok("application/json", Arc::clone(&self.list_bytes), true)
            }
            Route::Master { root } if *root == self.root => {
                match self.list.files.iter().find(|f| f.role == Role::PlaylistMaster) {
                    Some(f) => self.serve(f, None),
                    None => Response::status(404),
                }
            }
            Route::Listed { root, sha256, ext } if *root == self.root => match self.entry(sha256) {
                Some(f) => self.serve(f, Some(ext.as_str())),
                None => Response::status(404),
            },
            _ => Response::status(404),
        }
    }

    /// Bind the proxy on a loopback address. Run it with [`Server::run`].
    ///
    /// Every fetch spends money, so the proxy is for this machine's player only: it refuses non-loopback
    /// addresses, answers only its own loopback `Host` (no DNS rebinding) and sends no CORS header.
    pub fn bind(self, addr: SocketAddr) -> io::Result<Server> {
        if !addr.ip().is_loopback() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "the player proxy binds to loopback only"));
        }
        let me = Arc::new(self);
        let handler: Handler = Arc::new(move |route| me.answer(route));
        Server::bind(addr, handler, Options::LOCAL)
    }
}
