//! nutflix-eltor: video over El Tor, paid per minute by the circuit itself (ADR 0001).
//!
//! The commands here need no El Tor network of their own: packaging, keys, catalogs, terms, the seeder marker,
//! prices, the content origin, and `watch`, which reaches a seeder through tor's SocksPort. Running tor with
//! Foundation payment controllers (`seed`, and the viewer's plan) arrives with milestones M1 and M2, in the lab.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use nfe_catalog::{Catalog, Video};
use nfe_eltor::marker::Marker;
use nfe_eltor::price;
use nfe_eltor::terms::{Terms, quota_for_bitrate};
use nfx_media::package::{PackageOptions, SystemRunner, package};
use nfx_proto::namespace::{Namespace, VideoAddr};

const USAGE: &str = "\
nutflix-eltor <command> [options]

  package <input> --store DIR --video-id ID [--work DIR] [--namespace nfx:eltor:1]
          [--max-height N] [--title T] [--description D] [--entry FILE]
      Package a video into sha256-named CMAF files (NFX-05) and write its catalog entry.
  key new --out FILE           New catalog key (64 hex, written 0600); prints the public key.
  key pub --key FILE           Print the public key of a catalog key.
  catalog sign --key FILE --seeder FINGERPRINT --port N [--valid-for SECS] --out FILE ENTRY.json...
  catalog verify FILE [--seeder FINGERPRINT --contact CONTACTINFO]
  terms encode --rate-msat N [--quota-bytes N | --bitrate BPS] [--minutes M]
  terms decode HEX
  marker --key FILE --port N   The ContactInfo text for the seeder relay.
  price --terms HEX [--terms HEX ...] [--routing-fee-msat N] [--bitrate BPS] [--margin-permille N]
      Per-minute price and payload budget for a path (last hop last).
  serve --store DIR --catalog FILE [--listen 127.0.0.1:8443]
      The seeder's content origin: /catalog.json and the NFX-05 section 6 file surface.
  watch --origin IP:PORT (--socks ADDR [--socks-user U --socks-pass P] | --direct yes)
        [--seeder FINGERPRINT --contact CONTACTINFO] [--video ADDR] [--terms HEX ...] [--listen 127.0.0.1:0]
      Fetch and verify the seeder's catalog and a video's hash list, then serve a local HLS proxy for a player.
      --terms (each hop's, last hop last) paces pulls under every quota. --direct skips tor: lab only, unpaid,
      not anonymous.
";

type Res<T> = Result<T, String>;

struct Args {
    pos: Vec<String>,
    opts: HashMap<String, Vec<String>>,
}

impl Args {
    fn parse(raw: impl Iterator<Item = String>) -> Res<Self> {
        let (mut pos, mut opts) = (Vec::new(), HashMap::<String, Vec<String>>::new());
        let mut it = raw;
        while let Some(a) = it.next() {
            if let Some(name) = a.strip_prefix("--") {
                let v = it.next().ok_or_else(|| format!("--{name} needs a value"))?;
                opts.entry(name.to_owned()).or_default().push(v);
            } else {
                pos.push(a);
            }
        }
        Ok(Self { pos, opts })
    }
    fn opt(&self, name: &str) -> Res<Option<&str>> {
        match self.opts.get(name).map(Vec::as_slice) {
            None => Ok(None),
            Some([v]) => Ok(Some(v)),
            Some(_) => Err(format!("--{name} given more than once")),
        }
    }
    fn req(&self, name: &str) -> Res<&str> {
        self.opt(name)?.ok_or_else(|| format!("--{name} is required"))
    }
    fn num<T: std::str::FromStr>(&self, name: &str) -> Res<Option<T>> {
        self.opt(name)?.map(|v| v.parse().map_err(|_| format!("--{name}: not a number: {v}"))).transpose()
    }
    fn many(&self, name: &str) -> &[String] {
        self.opts.get(name).map_or(&[], Vec::as_slice)
    }
    fn check_known(&self, known: &[&str]) -> Res<()> {
        match self.opts.keys().find(|k| !known.contains(&k.as_str())) {
            Some(k) => Err(format!("unknown option --{k}")),
            None => Ok(()),
        }
    }
}

fn main() -> ExitCode {
    let mut raw = std::env::args().skip(1);
    let Some(cmd) = raw.next() else {
        eprint!("{USAGE}");
        return ExitCode::from(2);
    };
    let result = Args::parse(raw).and_then(|a| match cmd.as_str() {
        "package" => cmd_package(&a),
        "key" => cmd_key(&a),
        "catalog" => cmd_catalog(&a),
        "terms" => cmd_terms(&a),
        "marker" => cmd_marker(&a),
        "price" => cmd_price(&a),
        "serve" => cmd_serve(&a),
        "watch" => cmd_watch(&a),
        "help" | "-h" | "--help" => {
            print!("{USAGE}");
            Ok(())
        }
        other => Err(format!("unknown command {other}\n\n{USAGE}")),
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("nutflix-eltor: {e}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_package(a: &Args) -> Res<()> {
    a.check_known(&["store", "work", "video-id", "namespace", "max-height", "title", "description", "entry"])?;
    let [input] = a.pos.as_slice() else { return Err("package takes one input file".into()) };
    let store = PathBuf::from(a.req("store")?);
    let work = a.opt("work")?.map_or_else(|| store.join(".work"), PathBuf::from);
    let ns = Namespace::parse(a.opt("namespace")?.unwrap_or("nfx:eltor:1")).map_err(|e| e.to_string())?;
    let video_id = a.req("video-id")?;
    let video = VideoAddr::new(ns, video_id).map_err(|e| e.to_string())?;
    let opts = PackageOptions { video, max_height: a.num("max-height")?, preset: None, thumbnail: false };
    let packaged =
        package(Path::new(input), &store, &work, &SystemRunner::default(), &opts).map_err(|e| e.to_string())?;
    let title = a.opt("title")?.unwrap_or(video_id);
    let entry = Video::from_hash_list(&packaged.hash_list, &packaged.root, title, a.opt("description")?.unwrap_or(""))
        .map_err(|e| e.to_string())?;
    let json = serde_json::to_vec_pretty(&entry).map_err(|e| e.to_string())?;
    match a.opt("entry")? {
        Some(path) => std::fs::write(path, &json).map_err(|e| format!("{path}: {e}"))?,
        None => println!("{}", String::from_utf8_lossy(&json)),
    }
    eprintln!("root {} ({} files, {} renditions)", packaged.root_hex(), packaged.segs(), entry.renditions.len());
    Ok(())
}

fn read_key(path: &str) -> Res<[u8; 32]> {
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::symlink_metadata(path).map_err(|e| format!("{path}: {e}"))?;
    if !meta.is_file() || meta.permissions().mode() & 0o077 != 0 {
        return Err(format!("{path}: must be a regular file with no group or other permissions (0600)"));
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let hex_key = text.strip_suffix('\n').unwrap_or(&text);
    let mut key = [0u8; 32];
    if hex_key.len() != 64 || hex::decode_to_slice(hex_key, &mut key).is_err() {
        return Err(format!("{path}: not a 64-hex-character key"));
    }
    Ok(key)
}

fn pubkey(key: &[u8; 32]) -> Res<String> {
    nfx_proto::event::public_key_hex(key).map_err(|e| e.to_string())
}

fn cmd_key(a: &Args) -> Res<()> {
    match a.pos.as_slice() {
        [s] if s == "new" => {
            a.check_known(&["out"])?;
            let out = a.req("out")?;
            let mut key = [0u8; 32];
            let pk = loop {
                getrandom::fill(&mut key).map_err(|e| e.to_string())?;
                if let Ok(pk) = pubkey(&key) {
                    break pk;
                }
            };
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(out)
                .map_err(|e| format!("{out}: {e}"))?;
            f.write_all(format!("{}\n", hex::encode(key)).as_bytes()).map_err(|e| format!("{out}: {e}"))?;
            println!("{pk}");
            Ok(())
        }
        [s] if s == "pub" => {
            a.check_known(&["key"])?;
            println!("{}", pubkey(&read_key(a.req("key")?)?)?);
            Ok(())
        }
        _ => Err("key new --out FILE | key pub --key FILE".into()),
    }
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn cmd_catalog(a: &Args) -> Res<()> {
    match a.pos.split_first() {
        Some((s, entries)) if s == "sign" => {
            a.check_known(&["key", "seeder", "port", "valid-for", "out"])?;
            let key = read_key(a.req("key")?)?;
            let port: u16 = a.num("port")?.ok_or("--port is required")?;
            let valid_for: u64 = a.num("valid-for")?.unwrap_or(7 * 24 * 3600);
            let videos = entries
                .iter()
                .map(|p| {
                    let bytes = std::fs::read(p).map_err(|e| format!("{p}: {e}"))?;
                    serde_json::from_slice::<Video>(&bytes).map_err(|e| format!("{p}: {e}"))
                })
                .collect::<Res<Vec<_>>>()?;
            let c = Catalog::sign(&key, a.req("seeder")?, port, now(), valid_for, videos).map_err(|e| e.to_string())?;
            let out = a.req("out")?;
            std::fs::write(out, c.to_json().map_err(|e| e.to_string())?).map_err(|e| format!("{out}: {e}"))?;
            println!("signed {} videos for {} (valid until {})", c.videos.len(), c.seeder, c.expires_at);
            Ok(())
        }
        Some((s, [file])) if s == "verify" => {
            a.check_known(&["seeder", "contact"])?;
            let bytes = std::fs::read(file).map_err(|e| format!("{file}: {e}"))?;
            let c = Catalog::verify(&bytes, now()).map_err(|e| e.to_string())?;
            match (a.opt("seeder")?, a.opt("contact")?) {
                (Some(fp), Some(contact)) => {
                    let m = Marker::find(contact)
                        .ok_or("no nutflix-eltor marker in --contact")?
                        .map_err(|e| e.to_string())?;
                    c.check_binding(fp, &m).map_err(|e| e.to_string())?;
                    println!("ok: {} videos, bound to {fp}", c.videos.len());
                }
                (None, None) => println!("ok: {} videos, signature valid (not bound to a relay)", c.videos.len()),
                _ => return Err("--seeder and --contact go together".into()),
            }
            Ok(())
        }
        _ => Err("catalog sign ... | catalog verify FILE".into()),
    }
}

fn cmd_terms(a: &Args) -> Res<()> {
    match a.pos.as_slice() {
        [s] if s == "encode" => {
            a.check_known(&["rate-msat", "quota-bytes", "bitrate", "minutes"])?;
            let rate: u64 = a.num("rate-msat")?.ok_or("--rate-msat is required")?;
            let quota = match (a.num::<u64>("quota-bytes")?, a.num::<u64>("bitrate")?) {
                (Some(_), Some(_)) => return Err("--quota-bytes or --bitrate, not both".into()),
                (Some(q), None) => q,
                (None, Some(bps)) => quota_for_bitrate(bps, 1250).map_err(|e| e.to_string())?,
                (None, None) => 0,
            };
            let minutes: u64 = a.num("minutes")?.unwrap_or(30);
            println!(
                "{}",
                Terms::for_long_play(rate, minutes, quota).and_then(|t| t.encode()).map_err(|e| e.to_string())?
            );
            Ok(())
        }
        [s, hex_terms] if s == "decode" => {
            a.check_known(&[])?;
            let t = Terms::decode(hex_terms).map_err(|e| e.to_string())?;
            println!(
                "rate_msat {}\npaid_quota_bytes {}\ngrace_ms {}\nmax_age_ms {}\nmax_periods {}\n\
                 handshake_fee_msat {}\nsetup_timeout_ms {}\nusable_minutes {}",
                t.rate_msat,
                t.paid_quota_bytes,
                t.grace_ms,
                t.max_age_ms,
                t.max_periods,
                t.handshake_fee_msat,
                t.setup_timeout_ms,
                t.usable_minutes()
            );
            Ok(())
        }
        _ => Err("terms encode ... | terms decode HEX".into()),
    }
}

fn cmd_marker(a: &Args) -> Res<()> {
    a.check_known(&["key", "port"])?;
    let pk = pubkey(&read_key(a.req("key")?)?)?;
    let port: u16 = a.num("port")?.ok_or("--port is required")?;
    println!("{}", Marker::new(&pk, port).map_err(|e| e.to_string())?.format());
    Ok(())
}

fn cmd_price(a: &Args) -> Res<()> {
    a.check_known(&["terms", "routing-fee-msat", "bitrate", "margin-permille"])?;
    let path =
        a.many("terms").iter().map(|h| Terms::decode(h)).collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
    if path.is_empty() {
        return Err("give each hop's --terms, last hop last".into());
    }
    let fee: u64 = a.num("routing-fee-msat")?.unwrap_or(0);
    let margin: u64 = a.num("margin-permille")?.unwrap_or(100);
    let per_round = price::per_round_msat(&path, fee).map_err(|e| e.to_string())?;
    println!("per_minute_msat {per_round}");
    match price::payload_budget_per_round(&path, margin) {
        Some(b) => println!("payload_budget_bytes_per_minute {b}"),
        None => println!("payload_budget_bytes_per_minute unlimited"),
    }
    if let Some(bps) = a.num::<u64>("bitrate")? {
        println!("sustains_bitrate {}", price::sustains(&path, margin, bps));
    }
    Ok(())
}

fn cmd_serve(a: &Args) -> Res<()> {
    a.check_known(&["store", "catalog", "listen"])?;
    let listen = a.opt("listen")?.unwrap_or("127.0.0.1:8443").parse().map_err(|_| "--listen: not an address")?;
    let catalog_path = a.req("catalog")?;
    let catalog = std::fs::read(catalog_path).map_err(|e| format!("{catalog_path}: {e}"))?;
    Catalog::verify(&catalog, now()).map_err(|e| format!("{catalog_path}: {e}"))?;
    let origin = nfe_seed::Origin::bind(listen, Path::new(a.req("store")?), catalog).map_err(|e| e.to_string())?;
    eprintln!("serving on {}", origin.local_addr().map_err(|e| e.to_string())?);
    origin.run().map_err(|e| e.to_string())
}

fn cmd_watch(a: &Args) -> Res<()> {
    use nfe_watch::fetch::{Fetcher, Remote};
    use nfe_watch::pacer::Pacer;
    use nfe_watch::player::Player;
    use std::sync::Arc;
    use std::time::Duration;

    a.check_known(&[
        "origin",
        "socks",
        "socks-user",
        "socks-pass",
        "direct",
        "seeder",
        "contact",
        "video",
        "terms",
        "listen",
    ])?;
    let origin = a.req("origin")?.parse().map_err(|_| "--origin: not IP:PORT (an IP literal, never a name)")?;
    let socks = match (a.opt("socks")?, a.opt("direct")?) {
        (Some(s), None) => Some(s.parse().map_err(|_| "--socks: not an address")?),
        (None, Some("yes")) => {
            eprintln!("warning: --direct skips tor: this connection is neither paid nor anonymous (lab only)");
            None
        }
        _ => return Err("give --socks ADDR (tor's SocksPort) or --direct yes (lab only)".into()),
    };
    let socks_auth = match (a.opt("socks-user")?, a.opt("socks-pass")?) {
        (Some(u), Some(p)) => Some((u.to_owned(), p.to_owned())),
        (None, None) => None,
        _ => return Err("--socks-user and --socks-pass go together".into()),
    };
    let path =
        a.many("terms").iter().map(|h| Terms::decode(h)).collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
    let budget = price::payload_budget_per_round(&path, 100);
    match budget {
        Some(b) => eprintln!("pacing: at most {b} payload bytes per 60 s round"),
        None if path.is_empty() => eprintln!("pacing: off (no --terms given)"),
        None => eprintln!("pacing: off (no hop sets a quota)"),
    }
    let remote = Remote { origin, socks, socks_auth, timeout: Duration::from_secs(60) };
    let fetcher = Arc::new(Fetcher::new(remote, Pacer::new(budget)));
    let catalog = fetcher.catalog(now()).map_err(|e| format!("catalog: {e}"))?;
    match (a.opt("seeder")?, a.opt("contact")?) {
        (Some(fp), Some(contact)) => {
            let m = Marker::find(contact).ok_or("no nutflix-eltor marker in --contact")?.map_err(|e| e.to_string())?;
            catalog.check_binding(fp, &m).map_err(|e| e.to_string())?;
            if m.port != origin.port() {
                return Err("--origin's port is not the marker's content port".into());
            }
        }
        (None, None) => eprintln!("warning: catalog not bound to a relay (give --seeder and --contact)"),
        _ => return Err("--seeder and --contact go together".into()),
    }
    let video = match a.opt("video")? {
        Some(v) => catalog.videos.iter().find(|x| x.video == v).ok_or("no such video in the catalog")?,
        None => catalog.videos.first().ok_or("the catalog is empty")?,
    };
    let (list, bytes) = fetcher.hash_list(video).map_err(|e| format!("hash list: {e}"))?;
    let listen = a.opt("listen")?.unwrap_or("127.0.0.1:0").parse().map_err(|_| "--listen: not an address")?;
    let server = Player::new(Arc::clone(&fetcher), &video.root, list, bytes).bind(listen).map_err(|e| e.to_string())?;
    let local = server.local_addr().map_err(|e| e.to_string())?;
    eprintln!("{}: {} ({} s)", video.video, video.title, video.duration_ms / 1000);
    println!("http://{local}/{}/master.m3u8", video.root);
    server.run().map_err(|e| e.to_string())
}
