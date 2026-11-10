# Roadmap

Every milestone is checked in a local El Tor regtest lab (bitcoind regtest, a Core Lightning node per relay,
Chutney directory authorities and relays) on our own hosts. Hosted CI builds and runs unit tests only; it never runs
a network. The architecture is [ADR 0001](decisions/0001-architecture.md).

| Milestone | What | Done when |
|---|---|---|
| **M0** spikes | S1 exit-to-self attach, S2 quota at 0.9× and 1.1×, S3 free-circuit freeloader, S4 90-minute play and two scopes on one tor, S5 onion catalog | Every spike has a record in [`spikes/`](spikes/) with a PASS/FAIL verdict, and ADR 0001 is revised to match |
| **M1** seeder | Packaging with `nfx-media` (pinned), signed catalog, loopback content server, Core Lightning, Foundation terms encoder (P4) | Every segment fetched over the paid circuit matches its sha256, and the seeder's node shows one settled invoice per round |
| **M2** viewer | Plan writer, payment controller, paced segment fetcher, localhost HLS, spending cap | 20 minutes of 720p play with no RESOURCELIMIT close; Stop halts data within 90 s and no further invoices are paid (the lab's payer-stop scenario measured 79 s) |
| **M3** discovery | ContactInfo marker, seeder listing from the directory, verified catalog | Two seeders are listed, and either plays with no manual configuration |
| **M4** paywall | Patch P1: paid-only exit streams, optional free catalog port | The S3 freeloader gets 0 segment bytes |
| **M5** long play | Patch P3 (successive scopes), or long terms | A 2-hour title plays continuously, and the payment events show its renewals |
| **M6** real money | **Gate: the maintainers pick the network.** Either our own El Tor network (3 directory authorities, 3 relays and seeders, test mode, mainnet Lightning at about 1 sat per round) or signet first | A 10-minute watch settles, and the viewer's and seeder's ledgers reconcile with no uncertain payments |
| **M7** desktop | A desktop app on eltor-app's pattern (Tauri, bundled tor, wallet setup) | A packaged app plays a lab title end to end |
| later | nfxd compatibility ([#1](https://github.com/SovereignTechnology/nutflix-eltor/issues/1)) | nfxd fetches and verifies a nutflix-eltor video, and nutflix-eltor plays an nfxd-seeded one |

## Patches to El Tor

Carried on `SovereignTechnology/eltord` (plus an `eltor` fork if a C change is needed) and offered upstream.

| Patch | Where | What | Needed by |
|---|---|---|---|
| P1 | eltor (C) | Paid-only exit streams; optional free port | M4 |
| P2 | eltor (C) | Accept the end-to-end onion hop for paid streams; keep intro circuits past the free lifetime | only if the catalog moves to an onion service |
| P3 | eltord (Rust) | Several or successive payment scopes per process; per-session cap; stop after this round | M2 (surviving el-tor/eltord#17), M5 |
| P4 | eltord (Rust) | Foundation terms encoder; bounded descriptor read; round and byte counters in the status | M1 |
| P5 | eltor (C) | A dedicated signed descriptor line for seeders | after M3 |
| P6 | eltor (C) | Optional throttle instead of close on quota | optional |

## Risks

- **Scopes die.** Under el-tor/eltord#17 about 1 in 4 fresh paid circuits loses its scope for good, and long-lived
  circuits died after the first period renewal in the lab. The viewer must open a new scope and move segment
  fetches to it, which needs P3.
- **No production network.** Foundation runs only in test mode, so M6 means running our own network.
- **Fees.** Lightning routing fees dwarf 1-sat rounds; a late payment (beyond the 60 s lead plus 15 s grace) closes
  the circuit. The player buffers ahead and resumes on a new circuit.
- **Hard quota closes.** Pacing is mandatory; guard and middle count about twice the payload.
- **Viewer wallets.** BOLT12 payers are Core Lightning or Phoenixd only. Paying per round with BOLT11 would widen
  that, upstream.
- **Maturity and licences.** Everything is pre-1.0 and unreviewed. eltord and LNI ship no LICENSE file yet; ask
  upstream before any binary release. A statically linked tor carries LGPL-3 obligations (equix).
