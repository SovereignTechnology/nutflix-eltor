//! The seeder's content origin (ADR 0001 section 1), plus the HTTP pieces the viewer's local proxy shares.
//!
//! The origin answers the NFX-05 §6 surface ([`route`]) and `/catalog.json` from a content-addressed store, and
//! nothing else. Every file is checked against its sha256 before it is sent (the "Blossom rule": a corrupt store
//! never serves a wrong byte), and symlinks in the store are refused. It binds wherever it is told; the seeder binds
//! it where only its own relay's exit can reach it (loopback in the lab).
//!
//! Because the surface is NFX-05 §6, any NFX origin client can read from it (issue #1).

pub mod route;
pub mod server;

use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use nfx_proto::hashlist::{HashList, Role};

use route::{Route, content_type};
use server::{Handler, Options, Response, Server, StopHandle};

/// Largest file served (segments are a few MB; a 2-hour hash list is under 2 MB). Bounds memory per connection.
pub const MAX_FILE: u64 = 16 * 1024 * 1024;
/// Parsed hash lists kept for `/<root>/…` requests.
const HASH_LIST_CACHE: usize = 16;

struct Store {
    dir: PathBuf,
    catalog: RwLock<Arc<Vec<u8>>>,
    lists: Mutex<HashMap<String, Arc<HashList>>>,
}

impl Store {
    fn hash_list(&self, root: &str) -> Option<Arc<HashList>> {
        if let Some(l) = self.lists.lock().ok()?.get(root) {
            return Some(Arc::clone(l));
        }
        let list: HashList = serde_json::from_slice(&read_verified(&self.dir, root)?).ok()?;
        let list = Arc::new(list);
        let mut cache = self.lists.lock().ok()?;
        if cache.len() >= HASH_LIST_CACHE {
            cache.clear();
        }
        cache.insert(root.to_owned(), Arc::clone(&list));
        Some(list)
    }

    fn file(&self, sha256: &str, role: Option<Role>, ext: Option<&str>) -> Response {
        match read_verified(&self.dir, sha256) {
            Some(b) => Response::ok(content_type(role, ext), Arc::new(b), true),
            None => Response::status(404),
        }
    }

    fn answer(&self, route: &Route) -> Response {
        match route {
            Route::Catalog => {
                let body = self.catalog.read().map(|c| Arc::clone(&*c)).unwrap_or_default();
                Response::ok("application/json", body, false)
            }
            Route::File { sha256, ext } => self.file(sha256, None, ext.as_deref()),
            Route::Master { root } => {
                let Some(list) = self.hash_list(root) else { return Response::status(404) };
                match list.files.iter().find(|f| f.role == Role::PlaylistMaster) {
                    Some(f) => self.file(&f.sha256, Some(Role::PlaylistMaster), None),
                    None => Response::status(404),
                }
            }
            Route::Listed { root, sha256, ext } => {
                let Some(list) = self.hash_list(root) else { return Response::status(404) };
                // NFX-05 §6: an origin MAY refuse hashes the named hash list does not contain; we do.
                match list.files.iter().find(|f| &f.sha256 == sha256) {
                    Some(f) => self.file(sha256, Some(f.role), Some(ext.as_str())),
                    None => Response::status(404),
                }
            }
        }
    }
}

/// The store file named `sha256`, only if it is a regular file (not a symlink) whose sha256 is its name.
pub fn read_verified(dir: &Path, sha256: &str) -> Option<Vec<u8>> {
    let path = dir.join(sha256);
    let meta = std::fs::symlink_metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > MAX_FILE {
        return None;
    }
    let bytes = std::fs::read(&path).ok()?;
    nfx_proto::hashlist::verify_file(sha256, &bytes).ok()?;
    Some(bytes)
}

pub struct Origin {
    server: Server,
    store: Arc<Store>,
}

/// Controls a running origin from another thread.
#[derive(Clone)]
pub struct Handle {
    store: Arc<Store>,
    stop: StopHandle,
}

impl Origin {
    pub fn bind(addr: SocketAddr, store: &Path, catalog: Vec<u8>) -> io::Result<Self> {
        let dir = store.canonicalize()?;
        if !dir.is_dir() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "store is not a directory"));
        }
        let store = Arc::new(Store { dir, catalog: RwLock::new(Arc::new(catalog)), lists: Mutex::new(HashMap::new()) });
        let s = Arc::clone(&store);
        let handler: Handler = Arc::new(move |route| s.answer(route));
        Ok(Self { server: Server::bind(addr, handler, Options::ORIGIN)?, store })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.server.local_addr()
    }

    pub fn handle(&self) -> io::Result<Handle> {
        Ok(Handle { store: Arc::clone(&self.store), stop: self.server.stop_handle()? })
    }

    pub fn run(self) -> io::Result<()> {
        self.server.run()
    }
}

impl Handle {
    pub fn set_catalog(&self, catalog: Vec<u8>) {
        if let Ok(mut c) = self.store.catalog.write() {
            *c = Arc::new(catalog);
        }
    }

    pub fn stop(&self) {
        self.stop.stop();
    }
}

/// Test fixtures shared with the viewer's tests: a small store holding a valid one-rendition hash list.
#[doc(hidden)]
pub mod fixture {
    use super::*;
    use nfx_proto::hashlist::{FileEntry, Rendition};

    pub const VIDEO: &str = "nfx:eltor:1:video-a";

    pub fn put(dir: &Path, bytes: &[u8]) -> String {
        let name = hex::encode(nfx_proto::sha256(bytes));
        std::fs::write(dir.join(&name), bytes).expect("write fixture");
        name
    }

    pub struct Fixture {
        pub root: String,
        pub list: HashList,
        pub master: String,
        pub playlist: String,
        pub init: String,
        pub segs: Vec<String>,
    }

    /// Write a store with `n_segs` segments of `seg_len` bytes each, and its hash list.
    pub fn store(dir: &Path, n_segs: usize, seg_len: usize) -> Fixture {
        std::fs::create_dir_all(dir).expect("fixture dir");
        let init = put(dir, b"init bytes");
        let segs: Vec<String> = (0..n_segs)
            .map(|i| {
                let mut b = vec![u8::try_from(i % 251).unwrap_or(0); seg_len];
                b[..8].copy_from_slice(&(i as u64).to_be_bytes());
                put(dir, &b)
            })
            .collect();
        let mut pl = format!("#EXTM3U\n#EXT-X-MAP:URI=\"{init}.mp4\"\n");
        for s in &segs {
            pl.push_str(&format!("#EXTINF:2.000,\n{s}.m4s\n"));
        }
        let playlist = put(dir, pl.as_bytes());
        let master = put(dir, format!("#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=800000\n{playlist}.m3u8\n").as_bytes());
        let entry = |name: String, role, sha: &str, size: usize, dur_ms| FileEntry {
            name,
            role,
            sha256: sha.into(),
            size: size as u64,
            dur_ms,
        };
        let mut files = vec![
            entry("master.m3u8".into(), Role::PlaylistMaster, &master, 0, None),
            entry("360p.m3u8".into(), Role::Playlist, &playlist, pl.len(), None),
            entry("360p/init.mp4".into(), Role::Init, &init, 10, None),
        ];
        for (i, s) in segs.iter().enumerate() {
            files.push(entry(format!("360p/{i}.m4s"), Role::Segment, s, seg_len, Some(2000)));
        }
        let master_len = std::fs::metadata(dir.join(&master)).map(|m| m.len()).unwrap_or(0);
        files[0].size = master_len;
        let list = HashList {
            v: 1,
            video: VIDEO.into(),
            files,
            renditions: vec![Rendition {
                id: "360p".into(),
                playlist: "360p.m3u8".into(),
                bandwidth: 800_000,
                codecs: None,
                resolution: Some("640x360".into()),
            }],
        };
        let root = put(dir, &list.render());
        Fixture { root, list, master, playlist, init, segs }
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::{put, store};
    use super::server::testing::{fetch, get};
    use super::*;
    use std::thread;

    #[test]
    fn nfx_05_origin_surface() {
        let dir = std::env::temp_dir().join(format!("nfe-seed-{}", std::process::id()));
        let fx = store(&dir, 2, 100);
        let (seg, root) = (&fx.segs[0], &fx.root);
        let bad = hex::encode(nfx_proto::sha256(b"something else"));
        std::fs::write(dir.join(&bad), b"corrupted").unwrap();
        std::fs::write(dir.join("secret.txt"), b"no").unwrap();
        let _ = put(&dir, b"not a hash list");

        let origin = Origin::bind("127.0.0.1:0".parse().unwrap(), &dir, b"{\"v\":0}".to_vec()).unwrap();
        let addr = origin.local_addr().unwrap();
        let h = origin.handle().unwrap();
        let t = thread::spawn(move || origin.run());

        let (code, headers, body) = get(addr, "/catalog.json");
        assert_eq!((code, body), (200, b"{\"v\":0}".to_vec()));
        assert!(headers.contains("Cache-Control: no-store"));
        let (code, headers, body) = get(addr, &format!("/{seg}"));
        assert_eq!((code, body.len()), (200, 100));
        assert!(headers.contains("immutable") && headers.contains("Access-Control-Allow-Origin: *"));
        let (code, headers, _) = get(addr, &format!("/{seg}.m4s"));
        assert!(code == 200 && headers.contains("video/iso.segment"));
        assert_eq!(get(addr, &format!("/{root}")).2, fx.list.render());
        let (code, headers, body) = get(addr, &format!("/{root}/master.m3u8"));
        assert!(code == 200 && headers.contains("mpegurl"));
        assert_eq!(body, std::fs::read(dir.join(&fx.master)).unwrap());
        let (code, headers, _) = get(addr, &format!("/{root}/{}.mp4", fx.init));
        assert!(code == 200 && headers.contains("Content-Type: video/mp4"));
        assert_eq!(get(addr, &format!("/{root}/{bad}.m4s")).0, 404, "not in that hash list");
        assert_eq!(get(addr, &format!("/{bad}")).0, 404, "a corrupt file is never served");
        assert_eq!(get(addr, &format!("/{seg}/master.m3u8")).0, 404, "not a hash list");
        assert_eq!(get(addr, "/secret.txt").0, 404);
        assert_eq!(get(addr, &format!("/../{seg}")).0, 404);
        let (code, headers, body) = fetch(addr, &format!("HEAD /{seg} HTTP/1.1\r\n\r\n"));
        assert!(code == 200 && body.is_empty() && headers.contains("Content-Length: 100"));

        h.set_catalog(b"{\"v\":1}".to_vec());
        assert_eq!(get(addr, "/catalog.json").2, b"{\"v\":1}".to_vec());
        h.stop();
        t.join().unwrap().unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn fixture_hash_list_verifies() {
        let dir = std::env::temp_dir().join(format!("nfe-seed-fx-{}", std::process::id()));
        let fx = store(&dir, 3, 64);
        let bytes = std::fs::read(dir.join(&fx.root)).unwrap();
        let mut root = [0u8; 32];
        hex::decode_to_slice(&fx.root, &mut root).unwrap();
        let addr = nfx_proto::namespace::VideoAddr::parse(fixture::VIDEO).unwrap();
        HashList::verify(&bytes, &root, &addr, fx.list.files.len() as u64).unwrap();
        let pl = std::fs::read(dir.join(&fx.playlist)).unwrap();
        fx.list.check_playlist(&pl).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn symlinks_are_not_followed() {
        let dir = std::env::temp_dir().join(format!("nfe-seed-link-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let body = b"target".to_vec();
        let name = hex::encode(nfx_proto::sha256(&body));
        std::fs::write(dir.join("real"), &body).unwrap();
        std::os::unix::fs::symlink(dir.join("real"), dir.join(&name)).unwrap();
        assert!(read_verified(&dir.canonicalize().unwrap(), &name).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
