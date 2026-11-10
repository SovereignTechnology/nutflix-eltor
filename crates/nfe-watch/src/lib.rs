//! The nutflix-eltor viewer (ADR 0001 section 1), without the El Tor controller (M2 adds it in the lab).
//!
//! - [`socks`]: SOCKS5 to tor's SocksPort.
//! - [`client`]: a strict HTTP/1.1 GET over that stream.
//! - [`pacer`]: keep pulls under every hop's per-round quota.
//! - [`fetch`]: catalog, hash list and files from the seeder, each verified before use.
//! - [`player`]: the localhost HLS proxy a player points at.

pub mod client;
pub mod fetch;
pub mod pacer;
pub mod player;
pub mod socks;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("seeder answered HTTP {0}")]
    Status(u16),
    #[error("verification failed: {0}")]
    Verify(String),
    #[error(transparent)]
    Catalog(#[from] nfe_catalog::Error),
    #[error(transparent)]
    Pace(#[from] pacer::PaceError),
    #[error("internal lock poisoned")]
    Poisoned,
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::sync::Arc;
    use std::sync::atomic::Ordering;
    use std::thread;
    use std::time::Duration;

    use nfe_catalog::{Catalog, Video};
    use nfe_seed::fixture;
    use nfe_seed::server::testing::get;

    use crate::fetch::{Fetcher, Remote};
    use crate::pacer::Pacer;
    use crate::player::Player;
    use crate::socks;

    const FP: &str = "abcdef0123456789abcdef0123456789abcdef01";

    /// Seeder origin with a signed catalog for the fixture video; returns (origin addr, fixture, catalog).
    fn seeder(dir: &std::path::Path) -> (SocketAddr, fixture::Fixture) {
        let fx = fixture::store(dir, 4, 1000);
        let mut root = [0u8; 32];
        hex::decode_to_slice(&fx.root, &mut root).unwrap();
        let video = Video::from_hash_list(&fx.list, &root, "Fixture", "").unwrap();
        let catalog = Catalog::sign(&[7u8; 32], FP, 1, 1_800_000_000, 3600, vec![video]).unwrap();
        let origin = nfe_seed::Origin::bind("127.0.0.1:0".parse().unwrap(), dir, catalog.to_json().unwrap()).unwrap();
        let addr = origin.local_addr().unwrap();
        thread::spawn(move || origin.run());
        (addr, fx)
    }

    #[test]
    fn end_to_end_through_socks_with_verification() {
        let dir = std::env::temp_dir().join(format!("nfe-watch-{}", std::process::id()));
        let (origin, fx) = seeder(&dir);
        let (proxy, connects) = socks::testing::server(Some(("iso", "1")));
        let remote = Remote {
            origin,
            socks: Some(proxy),
            socks_auth: Some(("iso".into(), "1".into())),
            timeout: Duration::from_secs(5),
        };
        let fetcher = Arc::new(Fetcher::new(remote, Pacer::new(Some(50_000_000))));
        let catalog = fetcher.catalog(1_800_000_100).unwrap();
        let video = catalog.videos[0].clone();
        let (list, bytes) = fetcher.hash_list(&video).unwrap();
        assert_eq!(list, fx.list);

        let player =
            Player::new(Arc::clone(&fetcher), &video.root, list, bytes).bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let paddr = player.local_addr().unwrap();
        let stop = player.stop_handle().unwrap();
        let t = thread::spawn(move || player.run());

        let root = &fx.root;
        let (code, _, master) = get(paddr, &format!("/{root}/master.m3u8"));
        assert_eq!(code, 200);
        assert!(String::from_utf8(master).unwrap().contains(&format!("{}.m3u8", fx.playlist)));
        assert_eq!(get(paddr, &format!("/{root}")).2, std::fs::read(dir.join(root)).unwrap(), "exact hash-list bytes");
        assert_eq!(get(paddr, &format!("/{root}/{}.m3u8", fx.playlist)).0, 200);
        assert_eq!(get(paddr, &format!("/{root}/{}.mp4", fx.init)).0, 200);
        for s in &fx.segs {
            let (code, headers, body) = get(paddr, &format!("/{root}/{s}.m4s"));
            assert_eq!((code, body.len()), (200, 1000));
            assert!(headers.contains("video/iso.segment"));
        }
        let before = connects.load(Ordering::SeqCst);
        assert_eq!(get(paddr, &format!("/{root}/{}.m4s", fx.segs[0])).0, 200);
        assert_eq!(connects.load(Ordering::SeqCst), before, "served from cache");
        assert_eq!(get(paddr, &format!("/{}/master.m3u8", "c".repeat(64))).0, 404, "another root");
        assert_eq!(get(paddr, &format!("/{root}/{}.m4s", "d".repeat(64))).0, 404, "not listed");
        assert_eq!(get(paddr, "/catalog.json").0, 404, "the proxy serves one video only");
        stop.stop();
        t.join().unwrap().unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_tampered_seeder_cannot_get_bytes_past_the_viewer() {
        let dir = std::env::temp_dir().join(format!("nfe-watch-bad-{}", std::process::id()));
        let (origin, fx) = seeder(&dir);
        let remote = Remote { origin, socks: None, socks_auth: None, timeout: Duration::from_secs(5) };
        let fetcher = Arc::new(Fetcher::new(remote, Pacer::new(None)));
        let video = fetcher.catalog(1_800_000_100).unwrap().videos[0].clone();
        let (list, _) = fetcher.hash_list(&video).unwrap();
        // Corrupt one segment in the store: the origin refuses it (it checks hashes too) and so would the viewer.
        std::fs::write(dir.join(&fx.segs[1]), vec![0u8; 1000]).unwrap();
        let entry = list.files.iter().find(|f| f.sha256 == fx.segs[1]).unwrap().clone();
        assert!(fetcher.file(&entry).is_err());
        // A catalog naming a root that is not the hash list's fails hash-list verification.
        let mut wrong = video.clone();
        wrong.root = fx.segs[0].clone();
        assert!(fetcher.hash_list(&wrong).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pacing_caps_what_one_round_may_carry() {
        let dir = std::env::temp_dir().join(format!("nfe-watch-pace-{}", std::process::id()));
        let (origin, fx) = seeder(&dir);
        let remote = Remote { origin, socks: None, socks_auth: None, timeout: Duration::from_secs(5) };
        // Budget smaller than one segment: the fetch is refused rather than tripping a quota.
        let fetcher = Fetcher::new(remote, Pacer::new(Some(999)));
        let entry = fx.list.files.iter().find(|f| f.sha256 == fx.segs[0]).unwrap().clone();
        assert!(fetcher.file(&entry).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
