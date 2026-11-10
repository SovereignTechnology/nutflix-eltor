# ADR 0002: Catalog v0 and the seeder marker

- **Status:** Accepted
- **Date:** 2026-11-10
- **Code:** `crates/nfe-catalog`, `crates/nfe-eltor/src/marker.rs`

## Context

ADR 0001 has viewers find seeders in the El Tor directory and fetch each seeder's catalog over the paid circuit
that ends at it. This record fixes the two formats involved: the marker a seeder puts in its relay's ContactInfo,
and the catalog it serves.

## Decision

### Marker (ContactInfo)

```
nutflix-eltor:v0 ck=<64 lowercase hex> port=<1-65535>
```

- `ck`: the catalog key, an x-only BIP-340 public key. BIP-340 rather than ed25519 because it is the key type of
  Nostr and NFX: the same key can later sign NFX manifests or be a creator's npub (issue #1, open question 3), and
  the signing code is NFX's own (`nfx-proto`), not new.
- `port`: the content port that the relay's exit-to-self policy accepts.
- The marker may sit among other ContactInfo text. Parsers read the `key=value` tokens right after the
  `nutflix-eltor:v0` token, ignore unknown keys, and reject duplicates, a missing key, or a malformed value.
- The relay signs its descriptor, so the key and port are bound to that relay's identity.

### Catalog

Served at `GET /catalog.json` on the content port. One JSON object:

| Field | Meaning |
|---|---|
| `v` | `0` |
| `seeder` | the relay fingerprint, 40 lowercase hex |
| `port` | the content port; must equal the marker's |
| `issued_at`, `expires_at` | unix seconds; the window is at most 30 days |
| `videos[]` | `video` (NFX-01 address), `title` (≤ 200 chars, no control characters), `description` (≤ 5000), `root` (sha256 of the NFX-05 hash list, served at `/<root>`), `segs`, `duration_ms`, `renditions[]` (`id`, `bandwidth`, optional `resolution`) |
| `pubkey` | the catalog key, equal to the marker's `ck` |
| `sig` | BIP-340 over `sha256("nutflix-eltor/catalog/v0" ‖ 0x00 ‖ canon(catalog without sig))` |

`canon` is NFX-11 §9 canonical JSON. Viewers refuse input that canon refuses (duplicate keys, fractions, integers
beyond 2^53 − 1), unknown fields, catalogs over 1 MiB, more than 1000 videos, or `issued_at` more than 15 minutes in
the future.

**Prices are not in the catalog.** What a minute costs is each relay's own signed Foundation terms in the
directory, which the viewer reads before paying. A catalog cannot promise a price the relay does not charge.

A viewer accepts a catalog only after both checks:

1. it verifies (shape, signature, window), and
2. it binds to the relay being paid: `seeder` = that relay's fingerprint, `pubkey` = its marker's `ck`, `port` = its
   marker's `port`.

### Video addresses and files

Videos use NFX-01 addresses in the namespace `nfx:eltor:1` by default, and NFX-05 files unchanged: CMAF
renditions, every file named by its sha256, and a hash list whose sha256 is `root`. The seeder serves files at
`/<sha256>` and checks each file against its name before sending it.

The hash list names the video address, so the same segments packaged for another namespace (an nfxd network such
as `nfx:testnet:1`) share every segment but not the hash list or `root`. A bridge (issue #1) re-wraps the hash list;
it never re-encodes video.

## Consequences

- Catalogs are verifiable offline, and any copy is as good as the original.
- The catalog key is separate from the relay identity key, so it can live outside tor's data directory and later be a
  Nostr identity.
- A v1 marker or catalog is a new token and a new domain string; v0 readers ignore a v1 marker.
