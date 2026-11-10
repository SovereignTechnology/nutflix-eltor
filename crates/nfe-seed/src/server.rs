//! A small, bounded HTTP/1.1 server for GET and HEAD, shared by the seeder's origin and the viewer's local proxy.
//!
//! One thread per connection, at most [`MAX_CONNECTIONS`] at once; a request head of at most [`MAX_HEAD`] bytes read
//! within [`READ_TIMEOUT`]; every response closes the connection. The handler sees only a parsed [`Route`]: paths
//! that are not routes, other methods and other HTTP versions never reach it.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use crate::route::Route;

pub const MAX_CONNECTIONS: usize = 64;
pub const MAX_HEAD: usize = 8 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(10);
const WRITE_TIMEOUT: Duration = Duration::from_secs(60);

/// What a handler answers.
pub struct Response {
    pub code: u16,
    pub content_type: &'static str,
    pub body: Arc<Vec<u8>>,
    /// Content-addressed bytes: cache for a year (NFX-05 §6.1). Everything else is `no-store`.
    pub immutable: bool,
}

impl Response {
    pub fn ok(content_type: &'static str, body: Arc<Vec<u8>>, immutable: bool) -> Self {
        Self { code: 200, content_type, body, immutable }
    }

    pub fn status(code: u16) -> Self {
        let text: &[u8] = match code {
            404 => b"not found\n",
            502 => b"upstream failed\n",
            _ => b"error\n",
        };
        Self { code, content_type: "text/plain", body: Arc::new(text.to_vec()), immutable: false }
    }
}

pub type Handler = Arc<dyn Fn(&Route) -> Response + Send + Sync>;

/// Per-server policy.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// Send `Access-Control-Allow-Origin: *` (NFX-05 §6 requires it of origins).
    pub cors: bool,
    /// Answer only requests whose `Host` is this server's own loopback address (`127.0.0.1:<port>`,
    /// `localhost:<port>`, `[::1]:<port>`): a local service must not be reachable through DNS rebinding.
    pub loopback_host_only: bool,
}

impl Options {
    /// A public-facing origin.
    pub const ORIGIN: Self = Self { cors: true, loopback_host_only: false };
    /// A service only this machine's player should use.
    pub const LOCAL: Self = Self { cors: false, loopback_host_only: true };
}

struct Ctx {
    handler: Handler,
    opts: Options,
    allowed_hosts: Vec<String>,
}

pub struct Server {
    listener: TcpListener,
    ctx: Arc<Ctx>,
    active: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
}

/// Stops a running [`Server`] from another thread.
#[derive(Clone)]
pub struct StopHandle {
    stop: Arc<AtomicBool>,
    addr: SocketAddr,
}

impl StopHandle {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect_timeout(&self.addr, Duration::from_secs(1)); // wake accept()
    }
}

impl Server {
    pub fn bind(addr: SocketAddr, handler: Handler, opts: Options) -> io::Result<Self> {
        let listener = TcpListener::bind(addr)?;
        let port = listener.local_addr()?.port();
        let allowed_hosts = vec![format!("127.0.0.1:{port}"), format!("localhost:{port}"), format!("[::1]:{port}")];
        Ok(Self {
            listener,
            ctx: Arc::new(Ctx { handler, opts, allowed_hosts }),
            active: Arc::new(AtomicUsize::new(0)),
            stop: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    pub fn stop_handle(&self) -> io::Result<StopHandle> {
        Ok(StopHandle { stop: Arc::clone(&self.stop), addr: self.local_addr()? })
    }

    /// Accept until [`StopHandle::stop`].
    pub fn run(self) -> io::Result<()> {
        for conn in self.listener.incoming() {
            if self.stop.load(Ordering::SeqCst) {
                break;
            }
            let Ok(stream) = conn else { continue };
            if self.active.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                self.active.fetch_sub(1, Ordering::SeqCst);
                // Never block the accept loop: a short best-effort reply, no drain.
                let _ = stream.set_write_timeout(Some(Duration::from_millis(200)));
                let _ = (&stream)
                    .write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                continue;
            }
            let (ctx, active) = (Arc::clone(&self.ctx), Arc::clone(&self.active));
            thread::spawn(move || {
                let _ = serve(&ctx, stream);
                active.fetch_sub(1, Ordering::SeqCst);
            });
        }
        Ok(())
    }
}

/// `Ok((head_only, route))`, `Ok((_, None))` for a well-formed request that names no route, `Err(status)` otherwise.
fn parse_request_line(line: &str) -> Result<(bool, Option<Route>), u16> {
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
    Ok((head, Route::parse(path)))
}

fn serve(ctx: &Ctx, stream: TcpStream) -> io::Result<()> {
    let cors = ctx.opts.cors;
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
    let mut reader = BufReader::new((&stream).take(MAX_HEAD as u64));
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 || !line.ends_with('\n') {
        return respond(&stream, &Response::status(400), false, cors);
    }
    // Read the header block (bounded by MAX_HEAD through `take`); only `Host` is used.
    let mut hosts = Vec::new();
    loop {
        let mut h = String::new();
        let n = reader.read_line(&mut h)?;
        if n == 0 || !h.ends_with('\n') {
            return respond(&stream, &Response::status(431), false, cors);
        }
        if h == "\r\n" || h == "\n" {
            break;
        }
        if let Some((name, value)) = h.split_once(':')
            && name.trim().eq_ignore_ascii_case("host")
        {
            hosts.push(value.trim().to_owned());
        }
    }
    if ctx.opts.loopback_host_only && !(hosts.len() == 1 && ctx.allowed_hosts.contains(&hosts[0])) {
        return respond(&stream, &Response::status(403), false, cors);
    }
    match parse_request_line(line.trim_end_matches(['\r', '\n'])) {
        Err(code) => respond(&stream, &Response::status(code), false, cors),
        Ok((head, None)) => respond(&stream, &Response::status(404), head, cors),
        Ok((head, Some(route))) => respond(&stream, &(ctx.handler)(&route), head, cors),
    }
}

fn respond(mut s: &TcpStream, r: &Response, head: bool, cors: bool) -> io::Result<()> {
    let reason = match r.code {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        431 => "Request Header Fields Too Large",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Error",
    };
    let cache = if r.code == 200 && r.immutable { "public, max-age=31536000, immutable" } else { "no-store" };
    let mut out = format!(
        "HTTP/1.1 {} {reason}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: {cache}\r\n\
         X-Content-Type-Options: nosniff\r\nConnection: close\r\n",
        r.code,
        r.content_type,
        r.body.len()
    );
    if cors {
        out.push_str("Access-Control-Allow-Origin: *\r\n");
    }
    if r.code == 405 {
        out.push_str("Allow: GET, HEAD\r\n");
    }
    out.push_str("\r\n");
    s.write_all(out.as_bytes())?;
    if !head {
        s.write_all(&r.body)?;
    }
    s.flush()?;
    if r.code != 200 {
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

/// Test helpers, shared with the viewer's tests.
#[doc(hidden)]
pub mod testing {
    use super::*;
    use std::net::Shutdown;

    /// Send `raw`, return (status, headers, body).
    pub fn fetch(addr: SocketAddr, raw: &str) -> (u16, String, Vec<u8>) {
        let mut s = TcpStream::connect(addr).unwrap();
        s.write_all(raw.as_bytes()).unwrap();
        let _ = s.shutdown(Shutdown::Write);
        let mut buf = Vec::new();
        s.read_to_end(&mut buf).unwrap();
        let split = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let status: u16 = std::str::from_utf8(&buf[9..12]).unwrap().parse().unwrap();
        (status, String::from_utf8_lossy(&buf[..split]).into_owned(), buf[split + 4..].to_vec())
    }

    pub fn get(addr: SocketAddr, path: &str) -> (u16, String, Vec<u8>) {
        fetch(addr, &format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\n\r\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::testing::fetch;
    use super::*;

    #[test]
    fn protocol_errors_never_reach_the_handler() {
        let handler: Handler = Arc::new(|_| panic!("handler must not run"));
        let server = Server::bind("127.0.0.1:0".parse().unwrap(), handler, Options::ORIGIN).unwrap();
        let (addr, stop) = (server.local_addr().unwrap(), server.stop_handle().unwrap());
        let t = thread::spawn(move || server.run());
        assert_eq!(fetch(addr, "POST /catalog.json HTTP/1.1\r\n\r\n").0, 405);
        assert_eq!(fetch(addr, "GET /catalog.json HTTP/2\r\n\r\n").0, 400);
        assert_eq!(fetch(addr, "GET  /catalog.json HTTP/1.1\r\n\r\n").0, 400);
        assert_eq!(fetch(addr, "GET /nope HTTP/1.1\r\n\r\n").0, 404);
        let big = format!("GET /catalog.json HTTP/1.1\r\nX: {}\r\n\r\n", "a".repeat(MAX_HEAD));
        assert_eq!(fetch(addr, &big).0, 431);
        stop.stop();
        t.join().unwrap().unwrap();
    }

    #[test]
    fn local_servers_refuse_foreign_hosts_and_send_no_cors() {
        let handler: Handler = Arc::new(|_| Response::ok("text/plain", Arc::new(b"ok".to_vec()), false));
        let server = Server::bind("127.0.0.1:0".parse().unwrap(), handler, Options::LOCAL).unwrap();
        let (addr, stop) = (server.local_addr().unwrap(), server.stop_handle().unwrap());
        let t = thread::spawn(move || server.run());
        let port = addr.port();
        let req = |host: &str| fetch(addr, &format!("GET /catalog.json HTTP/1.1\r\nHost: {host}\r\n\r\n"));
        let (code, headers, body) = req(&format!("127.0.0.1:{port}"));
        assert_eq!((code, body), (200, b"ok".to_vec()));
        assert!(!headers.contains("Access-Control-Allow-Origin"));
        assert_eq!(req(&format!("localhost:{port}")).0, 200);
        assert_eq!(req("evil.example").0, 403, "DNS rebinding");
        assert_eq!(req(&format!("127.0.0.1:{}", port.wrapping_add(1))).0, 403);
        assert_eq!(fetch(addr, "GET /catalog.json HTTP/1.1\r\n\r\n").0, 403, "no Host");
        let two = format!("GET /catalog.json HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nHost: evil\r\n\r\n");
        assert_eq!(fetch(addr, &two).0, 403, "two Hosts");
        stop.stop();
        t.join().unwrap().unwrap();
    }
}
