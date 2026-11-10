//! A minimal HTTP/1.1 GET client for talking to a seeder origin over an already-open stream.
//!
//! The origin always answers with `Content-Length` and closes; anything else (chunked encoding, a body larger than
//! the caller allows, a malformed head) is an error. No redirects, no compression, no cookies.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpStream;

const MAX_HEAD: u64 = 16 * 1024;

fn err(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("http: {msg}"))
}

/// `GET path` with `Host: host`. Returns the status and, for 200, a body of at most `max_body` bytes.
pub fn get(stream: TcpStream, host: &str, path: &str, max_body: u64) -> io::Result<(u16, Vec<u8>)> {
    if !path.starts_with('/') || path.bytes().any(|b| b <= b' ' || b == 0x7f) {
        return Err(err("bad path"));
    }
    if host.bytes().any(|b| b <= b' ' || b == 0x7f) {
        return Err(err("bad host"));
    }
    (&stream).write_all(format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").as_bytes())?;

    let mut reader = BufReader::new((&stream).take(MAX_HEAD));
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.trim_end().splitn(3, ' ');
    let (Some(version), Some(code)) = (parts.next(), parts.next()) else { return Err(err("bad status line")) };
    if !version.starts_with("HTTP/1.") {
        return Err(err("not HTTP/1.x"));
    }
    let code: u16 = code.parse().map_err(|_| err("bad status code"))?;
    let mut length: Option<u64> = None;
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h)? == 0 || !h.ends_with('\n') {
            return Err(err("truncated or oversized head"));
        }
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        let Some((name, value)) = h.split_once(':') else { return Err(err("bad header")) };
        let (name, value) = (name.trim().to_ascii_lowercase(), value.trim());
        match name.as_str() {
            "content-length" => {
                if length.is_some() {
                    return Err(err("duplicate content-length"));
                }
                if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(err("bad content-length"));
                }
                length = Some(value.parse().map_err(|_| err("bad content-length"))?);
            }
            "transfer-encoding" => return Err(err("transfer-encoding is not supported")),
            _ => {}
        }
    }
    if code != 200 {
        return Ok((code, Vec::new()));
    }
    let length = length.ok_or_else(|| err("no content-length"))?;
    if length > max_body {
        return Err(err("body larger than allowed"));
    }
    // What the head reader buffered past the head belongs to the body.
    let n = usize::try_from(length).map_err(|_| err("body too large"))?;
    let mut body = Vec::with_capacity(n);
    let buffered = reader.buffer();
    body.extend_from_slice(&buffered[..buffered.len().min(n)]);
    let rest = length - body.len() as u64;
    (&stream).take(rest).read_to_end(&mut body)?;
    if body.len() as u64 != length {
        return Err(err("body shorter than content-length"));
    }
    Ok((code, body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{SocketAddr, TcpListener};
    use std::thread;

    fn serve_once(reply: &'static [u8]) -> SocketAddr {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        thread::spawn(move || {
            if let Ok((mut c, _)) = l.accept() {
                let mut buf = [0u8; 1024];
                let _ = c.read(&mut buf);
                let _ = c.write_all(reply);
            }
        });
        addr
    }

    fn call(reply: &'static [u8], max: u64) -> io::Result<(u16, Vec<u8>)> {
        let addr = serve_once(reply);
        get(TcpStream::connect(addr).unwrap(), "127.0.0.1", "/x", max)
    }

    #[test]
    fn responses() {
        assert_eq!(call(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello", 10).unwrap(), (200, b"hello".to_vec()));
        assert_eq!(call(b"HTTP/1.1 404 Not Found\r\nContent-Length: 3\r\n\r\nno\n", 10).unwrap().0, 404);
        assert!(call(b"HTTP/1.1 200 OK\r\nContent-Length: 50\r\n\r\nhello", 100).is_err(), "short body");
        assert!(call(b"HTTP/1.1 200 OK\r\nContent-Length: 50\r\n\r\n", 10).is_err(), "over max");
        assert!(call(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n", 10).is_err());
        assert!(call(b"HTTP/1.1 200 OK\r\nContent-Length: +5\r\n\r\nhello", 10).is_err());
        assert!(call(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nContent-Length: 5\r\n\r\nhello", 10).is_err());
        assert!(call(b"HTTP/1.1 200 OK\r\n\r\nhello", 10).is_err(), "no length");
        assert!(call(b"SSH-2.0-x\r\n\r\n", 10).is_err());
    }

    #[test]
    fn refuses_bad_paths() {
        // Requests are refused before anything is sent, so a listener that never accepts is enough.
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        assert!(get(TcpStream::connect(addr).unwrap(), "h", "/a b", 1).is_err());
        assert!(get(TcpStream::connect(addr).unwrap(), "h", "x", 1).is_err());
        assert!(get(TcpStream::connect(addr).unwrap(), "h\r\nX: y", "/a", 1).is_err());
    }
}
