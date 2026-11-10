//! The seeder's content origin (ADR 0001 section 1).
//!
//! A small HTTP/1.1 server that the seeder relay's exit-to-self policy points at. It answers exactly two kinds of
//! request and nothing else:
//!
//! - `GET /catalog.json`: the signed catalog (nfe-catalog);
//! - `GET /<sha256>` (64 lowercase hex): a content-addressed file from the store (NFX-05 §6 shape), checked against
//!   its name before it is sent (the "Blossom rule": a corrupt store never serves a wrong byte).
//!
//! `HEAD` is answered like `GET` without a body. Every response closes the connection. Requests are bounded (head
//! 8 KiB, 10 s read timeout) and so are connections ([`MAX_CONNECTIONS`]). It binds wherever it is told; the
//! seeder binds it where only its own relay's exit can reach it (loopback in the lab).

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

pub const MAX_CONNECTIONS: usize = 64;
pub const MAX_HEAD: usize = 8 * 1024;
/// Largest file served (segments are a few MB; a 2-hour hash list is under 2 MB). Bounds memory per connection.
pub const MAX_FILE: u64 = 16 * 1024 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(10);
const WRITE_TIMEOUT: Duration = Duration::from_secs(60);

struct State {
    store: PathBuf,
    catalog: RwLock<Arc<Vec<u8>>>,
    active: AtomicUsize,
    stop: AtomicBool,
}

pub struct Origin {
    listener: TcpListener,
    state: Arc<State>,
}

/// Controls a running origin from another thread.
#[derive(Clone)]
pub struct Handle {
    state: Arc<State>,
    addr: SocketAddr,
}

impl Origin {
    pub fn bind(addr: SocketAddr, store: &Path, catalog: Vec<u8>) -> io::Result<Self> {
        let store = store.canonicalize()?;
        if !store.is_dir() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "store is not a directory"));
        }
        let listener = TcpListener::bind(addr)?;
        let state = Arc::new(State {
            store,
            catalog: RwLock::new(Arc::new(catalog)),
            active: AtomicUsize::new(0),
            stop: AtomicBool::new(false),
        });
        Ok(Self { listener, state })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    pub fn handle(&self) -> io::Result<Handle> {
        Ok(Handle { state: Arc::clone(&self.state), addr: self.local_addr()? })
    }

    /// Accept until [`Handle::stop`]. One thread per connection, at most [`MAX_CONNECTIONS`] at once.
    pub fn run(self) -> io::Result<()> {
        for conn in self.listener.incoming() {
            if self.state.stop.load(Ordering::SeqCst) {
                break;
            }
            let Ok(stream) = conn else { continue };
            if self.state.active.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                self.state.active.fetch_sub(1, Ordering::SeqCst);
                // Never block the accept loop: a short best-effort reply, no drain.
                let _ = stream.set_write_timeout(Some(Duration::from_millis(200)));
                let _ = (&stream)
                    .write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                continue;
            }
            let state = Arc::clone(&self.state);
            thread::spawn(move || {
                let _ = serve(&state, stream);
                state.active.fetch_sub(1, Ordering::SeqCst);
            });
        }
        Ok(())
    }
}

impl Handle {
    pub fn set_catalog(&self, catalog: Vec<u8>) {
        if let Ok(mut c) = self.state.catalog.write() {
            *c = Arc::new(catalog);
        }
    }

    pub fn stop(&self) {
        self.state.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect_timeout(&self.addr, Duration::from_secs(1)); // wake accept()
    }
}

enum Target {
    Catalog,
    File(String),
}

fn parse_request_line(line: &str) -> Result<(bool, Option<Target>), u16> {
    let mut parts = line.split(' ');
    let (Some(method), Some(path), Some(version), None) = (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(400);
    };
    if version != "HTTP/1.1" && version != "HTTP/1.0" {
        return Err(400);
    }
    let head = match method {
        "GET" => false,
        "HEAD" => true,
        _ => return Err(405),
    };
    let target = match path {
        "/catalog.json" => Some(Target::Catalog),
        p => p
            .strip_prefix('/')
            .filter(|n| n.len() == 64 && n.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
            .map(|n| Target::File(n.to_owned())),
    };
    Ok((head, target))
}

fn serve(state: &State, stream: TcpStream) -> io::Result<()> {
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
    let mut reader = BufReader::new((&stream).take(MAX_HEAD as u64));
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 || !line.ends_with('\n') {
        return respond(&stream, 400, "text/plain", b"bad request\n", false, false);
    }
    // Drain the header block (bounded by MAX_HEAD through `take`); its contents are not needed.
    loop {
        let mut h = String::new();
        let n = reader.read_line(&mut h)?;
        if n == 0 || !h.ends_with('\n') {
            return respond(&stream, 431, "text/plain", b"header too large or truncated\n", false, false);
        }
        if h == "\r\n" || h == "\n" {
            break;
        }
    }
    let (head, target) = match parse_request_line(line.trim_end_matches(['\r', '\n'])) {
        Ok(v) => v,
        Err(code) => return respond(&stream, code, "text/plain", b"bad request\n", false, false),
    };
    match target {
        None => respond(&stream, 404, "text/plain", b"not found\n", head, false),
        Some(Target::Catalog) => {
            let body = state.catalog.read().map(|c| Arc::clone(&*c)).unwrap_or_default();
            respond(&stream, 200, "application/json", &body, head, false)
        }
        Some(Target::File(name)) => match read_verified(&state.store, &name) {
            Some(body) => respond(&stream, 200, "application/octet-stream", &body, head, true),
            None => respond(&stream, 404, "text/plain", b"not found\n", head, false),
        },
    }
}

/// The store file named `name`, only if it is a regular file (not a symlink) whose sha256 equals its name.
fn read_verified(store: &Path, name: &str) -> Option<Vec<u8>> {
    let path = store.join(name);
    let meta = std::fs::symlink_metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > MAX_FILE {
        return None;
    }
    let bytes = std::fs::read(&path).ok()?;
    nfx_proto::hashlist::verify_file(name, &bytes).ok()?;
    Some(bytes)
}

fn respond(mut s: &TcpStream, code: u16, ctype: &str, body: &[u8], head: bool, immutable: bool) -> io::Result<()> {
    let reason = match code {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        431 => "Request Header Fields Too Large",
        _ => "Service Unavailable",
    };
    let cache = if immutable { "public, max-age=31536000, immutable" } else { "no-store" };
    let mut out = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: {cache}\r\n\
         X-Content-Type-Options: nosniff\r\nConnection: close\r\n",
        body.len()
    );
    if code == 405 {
        out.push_str("Allow: GET, HEAD\r\n");
    }
    out.push_str("\r\n");
    s.write_all(out.as_bytes())?;
    if !head {
        s.write_all(body)?;
    }
    s.flush()?;
    if code != 200 {
        drain(s);
    }
    Ok(())
}

/// After an error reply the client may still be sending; closing with unread input makes the kernel send a reset
/// that can destroy the reply in flight. Half-close, then discard a bounded amount for a short while.
fn drain(s: &TcpStream) {
    let _ = s.shutdown(std::net::Shutdown::Write);
    let _ = s.set_read_timeout(Some(Duration::from_millis(500)));
    let _ = io::copy(&mut s.take(64 * 1024), &mut io::sink());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Shutdown;

    fn fetch(addr: SocketAddr, raw: &str) -> (u16, Vec<u8>) {
        let mut s = TcpStream::connect(addr).unwrap();
        s.write_all(raw.as_bytes()).unwrap();
        let _ = s.shutdown(Shutdown::Write);
        let mut buf = Vec::new();
        s.read_to_end(&mut buf).unwrap();
        let split = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let status: u16 = std::str::from_utf8(&buf[9..12]).unwrap().parse().unwrap();
        (status, buf[split + 4..].to_vec())
    }

    fn get(addr: SocketAddr, path: &str) -> (u16, Vec<u8>) {
        fetch(addr, &format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n"))
    }

    #[test]
    fn serves_catalog_and_verified_files_only() {
        let dir = std::env::temp_dir().join(format!("nfe-seed-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let good = b"segment bytes".to_vec();
        let good_name = hex::encode(nfx_proto::sha256(&good));
        std::fs::write(dir.join(&good_name), &good).unwrap();
        let bad_name = hex::encode(nfx_proto::sha256(b"something else"));
        std::fs::write(dir.join(&bad_name), b"corrupted").unwrap();
        std::fs::write(dir.join("secret.txt"), b"no").unwrap();

        let origin = Origin::bind("127.0.0.1:0".parse().unwrap(), &dir, b"{\"v\":0}".to_vec()).unwrap();
        let addr = origin.local_addr().unwrap();
        let h = origin.handle().unwrap();
        let t = thread::spawn(move || origin.run());

        assert_eq!(get(addr, "/catalog.json"), (200, b"{\"v\":0}".to_vec()));
        assert_eq!(get(addr, &format!("/{good_name}")), (200, good.clone()));
        assert_eq!(get(addr, &format!("/{bad_name}")).0, 404, "a corrupt file is never served");
        assert_eq!(get(addr, "/secret.txt").0, 404);
        assert_eq!(get(addr, "/../etc/passwd").0, 404);
        assert_eq!(get(addr, &format!("/{}", good_name.to_uppercase())).0, 404);
        assert_eq!(get(addr, &format!("/{good_name}?x=1")).0, 404);
        assert_eq!(get(addr, "/").0, 404);
        let (code, body) = fetch(addr, &format!("HEAD /{good_name} HTTP/1.1\r\n\r\n"));
        assert_eq!((code, body.len()), (200, 0));
        assert_eq!(fetch(addr, "POST /catalog.json HTTP/1.1\r\n\r\n").0, 405);
        assert_eq!(fetch(addr, "GET /catalog.json HTTP/2\r\n\r\n").0, 400);
        assert_eq!(fetch(addr, &format!("GET /catalog.json HTTP/1.1\r\nX: {}\r\n\r\n", "a".repeat(MAX_HEAD))).0, 431);

        h.set_catalog(b"{\"v\":1}".to_vec());
        assert_eq!(get(addr, "/catalog.json").1, b"{\"v\":1}".to_vec());

        h.stop();
        t.join().unwrap().unwrap();
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
