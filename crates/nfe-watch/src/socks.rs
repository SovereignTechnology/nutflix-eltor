//! SOCKS5 CONNECT (RFC 1928), with optional username/password (RFC 1929).
//!
//! This is how the viewer reaches El Tor: tor's SocksPort. Tor uses SOCKS credentials only to isolate streams onto
//! different circuits, never to authenticate. Targets are IP literals here: an exit-to-self seeder is reached by its
//! address (ADR 0001), and no name is ever resolved locally.

use std::io::{self, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::time::Duration;

fn err(msg: &str) -> io::Error {
    io::Error::other(format!("socks5: {msg}"))
}

/// Open a stream to `target` through the SOCKS5 proxy at `proxy`.
pub fn connect(
    proxy: SocketAddr,
    target: SocketAddr,
    auth: Option<(&str, &str)>,
    timeout: Duration,
) -> io::Result<TcpStream> {
    let mut s = TcpStream::connect_timeout(&proxy, timeout)?;
    s.set_read_timeout(Some(timeout))?;
    s.set_write_timeout(Some(timeout))?;
    s.set_nodelay(true)?;

    // Greeting: offer exactly the method we will use.
    let method = if auth.is_some() { 2u8 } else { 0u8 };
    s.write_all(&[5, 1, method])?;
    let mut reply = [0u8; 2];
    s.read_exact(&mut reply)?;
    if reply[0] != 5 {
        return Err(err("not a SOCKS5 proxy"));
    }
    if reply[1] != method {
        return Err(err("proxy refused the offered method"));
    }
    if let Some((user, pass)) = auth {
        let (u, p) = (user.as_bytes(), pass.as_bytes());
        let (Ok(ul), Ok(pl)) = (u8::try_from(u.len()), u8::try_from(p.len())) else {
            return Err(err("credentials longer than 255 bytes"));
        };
        if ul == 0 || pl == 0 {
            return Err(err("empty credentials"));
        }
        let mut msg = vec![1, ul];
        msg.extend_from_slice(u);
        msg.push(pl);
        msg.extend_from_slice(p);
        s.write_all(&msg)?;
        s.read_exact(&mut reply)?;
        if reply != [1, 0] {
            return Err(err("credentials refused"));
        }
    }

    // CONNECT to an IP literal.
    let mut req = vec![5, 1, 0];
    match target.ip() {
        IpAddr::V4(ip) => {
            req.push(1);
            req.extend_from_slice(&ip.octets());
        }
        IpAddr::V6(ip) => {
            req.push(4);
            req.extend_from_slice(&ip.octets());
        }
    }
    req.extend_from_slice(&target.port().to_be_bytes());
    s.write_all(&req)?;

    let mut head = [0u8; 4];
    s.read_exact(&mut head)?;
    if head[0] != 5 {
        return Err(err("bad reply version"));
    }
    if head[1] != 0 {
        return Err(err(match head[1] {
            1 => "general failure",
            2 => "not allowed by ruleset",
            3 => "network unreachable",
            4 => "host unreachable",
            5 => "connection refused",
            6 => "TTL expired",
            7 => "command not supported",
            8 => "address type not supported",
            _ => "unknown failure",
        }));
    }
    // Skip the bound address.
    let skip = match head[3] {
        1 => 4 + 2,
        4 => 16 + 2,
        3 => {
            let mut len = [0u8; 1];
            s.read_exact(&mut len)?;
            usize::from(len[0]) + 2
        }
        _ => return Err(err("bad bound address type")),
    };
    let mut rest = vec![0u8; skip];
    s.read_exact(&mut rest)?;
    Ok(s)
}

/// A minimal SOCKS5 server for tests: no-auth or one fixed user/password, CONNECT to IP literals, then a byte pipe.
#[doc(hidden)]
pub mod testing {
    use super::*;
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;

    /// Starts on a free loopback port; returns its address and a counter of CONNECTs served.
    pub fn server(auth: Option<(&'static str, &'static str)>) -> (SocketAddr, Arc<AtomicUsize>) {
        let l = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = l.local_addr().expect("addr");
        let count = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&count);
        thread::spawn(move || {
            for conn in l.incoming().flatten() {
                let c = Arc::clone(&c);
                thread::spawn(move || {
                    let _ = handle(conn, auth, &c);
                });
            }
        });
        (addr, count)
    }

    fn handle(mut s: TcpStream, auth: Option<(&str, &str)>, count: &AtomicUsize) -> io::Result<()> {
        let mut b = [0u8; 2];
        s.read_exact(&mut b)?;
        let mut methods = vec![0u8; usize::from(b[1])];
        s.read_exact(&mut methods)?;
        let want = if auth.is_some() { 2 } else { 0 };
        if !methods.contains(&want) {
            return s.write_all(&[5, 0xff]);
        }
        s.write_all(&[5, want])?;
        if let Some((user, pass)) = auth {
            let mut v = [0u8; 2];
            s.read_exact(&mut v)?;
            let mut u = vec![0u8; usize::from(v[1])];
            s.read_exact(&mut u)?;
            let mut pl = [0u8; 1];
            s.read_exact(&mut pl)?;
            let mut p = vec![0u8; usize::from(pl[0])];
            s.read_exact(&mut p)?;
            let ok = u == user.as_bytes() && p == pass.as_bytes();
            s.write_all(&[1, if ok { 0 } else { 1 }])?;
            if !ok {
                return Ok(());
            }
        }
        let mut h = [0u8; 4];
        s.read_exact(&mut h)?;
        let ip: IpAddr = match h[3] {
            1 => {
                let mut a = [0u8; 4];
                s.read_exact(&mut a)?;
                a.into()
            }
            4 => {
                let mut a = [0u8; 16];
                s.read_exact(&mut a)?;
                a.into()
            }
            _ => return s.write_all(&[5, 8, 0, 1, 0, 0, 0, 0, 0, 0]),
        };
        let mut port = [0u8; 2];
        s.read_exact(&mut port)?;
        let Ok(up) = TcpStream::connect(SocketAddr::new(ip, u16::from_be_bytes(port))) else {
            return s.write_all(&[5, 5, 0, 1, 0, 0, 0, 0, 0, 0]);
        };
        count.fetch_add(1, Ordering::SeqCst);
        s.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])?;
        let (mut a, mut b2) = (s.try_clone()?, up.try_clone()?);
        let (mut s2, mut up2) = (s, up);
        let t = thread::spawn(move || {
            let _ = io::copy(&mut a, &mut up2);
            let _ = up2.shutdown(std::net::Shutdown::Write);
        });
        let _ = io::copy(&mut b2, &mut s2);
        let _ = s2.shutdown(std::net::Shutdown::Write);
        let _ = t.join();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    fn echo_once() -> SocketAddr {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        thread::spawn(move || {
            if let Ok((mut c, _)) = l.accept() {
                let mut b = [0u8; 5];
                if c.read_exact(&mut b).is_ok() {
                    let _ = c.write_all(&b);
                }
            }
        });
        addr
    }

    #[test]
    fn connect_no_auth_and_auth() {
        let t = Duration::from_secs(5);
        let (proxy, count) = testing::server(None);
        let mut s = connect(proxy, echo_once(), None, t).unwrap();
        s.write_all(b"hello").unwrap();
        let mut b = [0u8; 5];
        s.read_exact(&mut b).unwrap();
        assert_eq!(&b, b"hello");
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);

        let (proxy, _) = testing::server(Some(("u", "p")));
        assert!(connect(proxy, echo_once(), Some(("u", "p")), t).is_ok());
        assert!(connect(proxy, echo_once(), Some(("u", "wrong")), t).is_err());
        assert!(connect(proxy, echo_once(), None, t).is_err(), "method mismatch");
    }

    #[test]
    fn refused_target_is_an_error() {
        let (proxy, _) = testing::server(None);
        let closed = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap()
        };
        let e = connect(proxy, closed, None, Duration::from_secs(5)).unwrap_err();
        assert!(e.to_string().contains("connection refused"), "{e}");
    }
}
