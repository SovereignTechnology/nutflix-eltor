# nutflix-eltor

> **Source of truth:** [`nostr://npub1s0vtechh66tx7vrwdud8zfyheu9zca7swwfrzd4qu2a4f93mxs6qvn9adx/nutflix-eltor`](https://gitworkshop.dev/npub1s0vtechh66tx7vrwdud8zfyheu9zca7swwfrzd4qu2a4f93mxs6qvn9adx/nutflix-eltor).
> Issues and pull requests live there. This GitHub repository is a mirror.

Video over [El Tor](https://github.com/el-tor), paid per minute by the circuit itself.

El Tor is a Tor fork whose clients pay every relay in their circuit over Lightning (BOLT12), one 60-second round at
a time, with a byte quota per round. nutflix-eltor makes the video seeder one of those relays. A viewer builds a
paid circuit that ends at the seeder, fetches content-addressed video segments through it, and pays each hop every
minute it keeps watching. Serving data is the thing that gets paid, so there is no separate money protocol: stop
paying and the circuit, and the video, stop.

nutflix-eltor is a deliberately small, new implementation in the nutflix/NFX family. It drops most of the
[NFX](https://github.com/SovereignTechnology/nfx) standards stack (iroh transport, Nostr availability beacons, the
Cashu payment protocol) and uses El Tor for transport, payment and discovery. It keeps NFX-05's file layout
(CMAF segments named by sha256, plus a hash list), so compatibility with `nfxd` can be added later as a bridge
([issue #1](https://github.com/SovereignTechnology/nutflix-eltor/issues/1)).

## Status (2026-11-10)

Early. The architecture is in [ADR 0001](docs/decisions/0001-architecture.md), the catalog and seeder marker in
[ADR 0002](docs/decisions/0002-catalog-v0.md), and the milestones in [docs/roadmap.md](docs/roadmap.md).

Built so far, all without an El Tor network: packaging (NFX's `nfx-media`), catalog keys, signed catalogs and their
binding to a relay, El Tor Foundation terms, the seeder's ContactInfo marker, client plans in eltord's format,
per-minute prices and pacing budgets, and the seeder's content origin. `lab/smoke-seed.sh` packages a clip, signs a
catalog, serves it and fetches every file back by hash. Driving El Tor itself (`seed`, `watch`) waits for M0's
spikes in a local El Tor regtest lab.

El Tor itself is pre-1.0. Its newer paid-circuit mode ("Foundation") runs only on test-mode networks today, and
paid onion services are specified but not implemented. nutflix-eltor targets that mode and tracks the patches it
needs in the roadmap.

## How it works (v0)

```
viewer: nutflix-eltor watch ──SOCKS──▶ El Tor client (Foundation plan: guard, middle, SEEDER)
  ▲ localhost HLS, paced under quotas    pays all three hops every 60 s round over Lightning
  player                                 ▼
                    seeder: El Tor relay, exit-to-self ──▶ content server on loopback only
                    (signed catalog + /<sha256> segments); its Lightning node receives its rate
```

- **Seeder:** an El Tor relay whose exit policy reaches only its own content server. Its per-round rate is what
  it earns, and its byte quota per round caps what one paid minute buys.
- **Viewer:** reads every hop's signed terms from the El Tor directory, shows the per-minute price, pins the seeder
  as the last hop, verifies every segment's sha256, and plays.
- **Discovery:** seeders mark their relay descriptor. Viewers list seeders from the El Tor directory and fetch each
  seeder's signed catalog over the paid circuit.

## Repository layout

| Path | What it is |
|---|---|
| `crates/nfe-catalog` | Catalog v0: format, BIP-340 signing, verification, binding to a relay |
| `crates/nfe-eltor` | El Tor glue: Foundation terms, the seeder marker, client plans, prices and pacing |
| `crates/nfe-seed` | Seeder: the content origin (relay controller and exit-to-self come in M1) |
| `crates/nfe-watch` | Viewer (M2, not yet): payment controller, paced segment fetcher, local HLS |
| `crates/nutflix-eltor` | The binary: `package`, `key`, `catalog`, `terms`, `marker`, `price`, `serve` (`seed`, `watch` later) |
| `lab/` | Lab tools: terms encoder (Python), seeder smoke test, later the regtest scenarios |
| `docs/` | Decisions, roadmap, spike records |

## Licence

AGPL-3.0-or-later. See [LICENSE](LICENSE).
