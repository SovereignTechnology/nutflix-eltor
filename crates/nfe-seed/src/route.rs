//! The URL surface shared by the seeder's origin and the viewer's local proxy: NFX-05 §6, plus `/catalog.json`.
//!
//! - `GET /<sha256>[.<ext>]`: file bytes (the extension is ignored for lookup);
//! - `GET /<root>`: the hash list (it is a file like any other);
//! - `GET /<root>/master.m3u8`: the master playlist of that hash list;
//! - `GET /<root>/<sha256>.<ext>`: a file listed in that hash list (what a player pointed at the master requests,
//!   since playlist URIs are relative);
//! - `GET /catalog.json`: the seeder's signed catalog (nutflix-eltor, ADR 0002).
//!
//! Anything else, including any query string, is not a route.

use nfx_proto::hashlist::Role;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    Catalog,
    /// `/<sha256>[.<ext>]`
    File {
        sha256: String,
        ext: Option<String>,
    },
    /// `/<root>/master.m3u8`
    Master {
        root: String,
    },
    /// `/<root>/<sha256>.<ext>`
    Listed {
        root: String,
        sha256: String,
        ext: String,
    },
}

fn is_sha(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn is_ext(s: &str) -> bool {
    (1..=8).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

/// `name` = `<sha256>` or `<sha256>.<ext>`.
fn content_name(name: &str) -> Option<(String, Option<String>)> {
    match name.split_once('.') {
        None => is_sha(name).then(|| (name.to_owned(), None)),
        Some((sha, ext)) => (is_sha(sha) && is_ext(ext)).then(|| (sha.to_owned(), Some(ext.to_owned()))),
    }
}

impl Route {
    pub fn parse(path: &str) -> Option<Self> {
        if path == "/catalog.json" {
            return Some(Self::Catalog);
        }
        let rest = path.strip_prefix('/')?;
        match rest.split_once('/') {
            None => content_name(rest).map(|(sha256, ext)| Self::File { sha256, ext }),
            Some((root, "master.m3u8")) if is_sha(root) => Some(Self::Master { root: root.to_owned() }),
            Some((root, name)) if is_sha(root) => match content_name(name)? {
                (sha256, Some(ext)) => Some(Self::Listed { root: root.to_owned(), sha256, ext }),
                (_, None) => None,
            },
            Some(_) => None,
        }
    }
}

/// NFX-05 §6 content types: by the file's role when the hash list is known, else by extension.
pub fn content_type(role: Option<Role>, ext: Option<&str>) -> &'static str {
    match role {
        Some(Role::PlaylistMaster | Role::Playlist) => "application/vnd.apple.mpegurl",
        Some(Role::Segment) => "video/iso.segment",
        Some(Role::Init) => "video/mp4",
        Some(Role::Subtitle) => "text/vtt",
        Some(Role::Thumb) | None => match ext {
            Some("m3u8") => "application/vnd.apple.mpegurl",
            Some("m4s") => "video/iso.segment",
            Some("mp4") => "video/mp4",
            Some("vtt") => "text/vtt",
            Some("jpg" | "jpeg") => "image/jpeg",
            Some("png") => "image/png",
            Some("webp") => "image/webp",
            Some("json") => "application/json",
            _ => "application/octet-stream",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes() {
        let a = "a".repeat(64);
        let b = "b".repeat(64);
        assert_eq!(Route::parse("/catalog.json"), Some(Route::Catalog));
        assert_eq!(Route::parse(&format!("/{a}")), Some(Route::File { sha256: a.clone(), ext: None }));
        assert_eq!(
            Route::parse(&format!("/{a}.m4s")),
            Some(Route::File { sha256: a.clone(), ext: Some("m4s".into()) })
        );
        assert_eq!(Route::parse(&format!("/{a}/master.m3u8")), Some(Route::Master { root: a.clone() }));
        assert_eq!(
            Route::parse(&format!("/{a}/{b}.m3u8")),
            Some(Route::Listed { root: a.clone(), sha256: b.clone(), ext: "m3u8".into() })
        );
        for bad in [
            "/".to_owned(),
            "/catalog.json?x".to_owned(),
            format!("/{a}?x=1"),
            format!("/{}", a.to_uppercase()),
            format!("/{a}.M4S"),
            format!("/{a}."),
            format!("/{a}.toolongext"),
            format!("/{a}/{b}"),
            format!("/{a}/{b}.m4s/x"),
            format!("/../{a}"),
            format!("/{a}/../{b}.m4s"),
            format!("//{a}"),
            format!("/{a}/master.m3u8/"),
        ] {
            assert_eq!(Route::parse(&bad), None, "{bad}");
        }
    }

    #[test]
    fn content_types() {
        assert_eq!(content_type(Some(Role::Segment), Some("bin")), "video/iso.segment");
        assert_eq!(content_type(None, Some("m3u8")), "application/vnd.apple.mpegurl");
        assert_eq!(content_type(None, None), "application/octet-stream");
    }
}
