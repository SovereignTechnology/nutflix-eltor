# ADR 0001: Video over paid El Tor circuits

- **Status:** Accepted (design). M0 spikes may revise the parts marked *to prove*.
- **Date:** 2026-11-10

## Context

nutflix and NFX deliver video peer to peer and pay for it with a separate payment protocol (Cashu ecash per
delivered chunk, NFX-07). nutflix-eltor tries the opposite approach: let the transport be the payment. El Tor is a
Tor fork in which a client pays every relay in its circuit over Lightning, per 60-second round. If the seeder is a
relay in the viewer's circuit, then serving video is paid for by the same mechanism that carries it.

We read El Tor at these commits. Paths below are relative to those repositories.

- `el-tor/eltor` (C, Tor 0.5.0-alpha base) at `2e53526`
- `el-tor/eltord` (Rust daemon and spec book) at `9f589be`

### What El Tor provides

El Tor has two payment modes. nutflix-eltor targets **Foundation** (eltord #12, opt-in).

| | Legacy (default) | Foundation |
|---|---|---|
| Relay choice | Random within a fee cap; `ExitNodes` is only a preference (`eltord/src/client/select_relay_algo.rs:247-264`) | An exact ordered list of 1–16 fingerprints in a private JSON plan (`eltord/src/client/runtime_config.rs:10-31`) |
| Terms | Rate per interval in the descriptor | Per-relay signed terms: rate per round, round length, grace, byte quota per round, max age, periods (`eltor/src/feature/relay/router.c:3157-3175`; `eltord/spec/01a_circuit_renewal_wire.md`) |
| Byte quota | Published, not enforced | Enforced; the circuit is closed with RESOURCELIMIT when exceeded (`eltor/src/feature/payment/payment_circuit.c:448-465`) |
| Long streams | A new circuit every 10 rounds; streams break | In-place renewal; streams survive until max age |
| Verification | The relay polls its node for the payer note | The relay checks its own Core Lightning node, single-use redemption (`eltord/src/wallet_adapters/cln.rs`) |
| Network | Any El Tor network | Refuses to start without `TestingTorNetwork 1` (`eltor/src/app/config/config.c:4123-4124`) |

Other facts that shape the design:

- **Every hop charges in Foundation.** A circuit mixing paid and unpaid hops is refused
  (`eltor/src/feature/payment/payment_origin.c:630`). Guard and middle count wire bytes in both directions, about
  twice the payload; the last hop counts about once.
- **Stopping payment stops data** after the round lapses plus a 15 s grace (`eltor/src/core/or/relay.c:1737-1745`).
- **Paid onion services are specified but not implemented** (`eltord/spec/02_paid_hidden_service.md:3`). A
  Foundation client cannot use a free onion service today: the end-to-end onion hop carries no payment keys, so its
  streams never attach (`payment_origin.c:938-955`).
- **Free circuits exist.** Relays give circuits without a payment profile a small free allowance (2 cells/s,
  128-cell burst, 10 minutes, 4 MiB/s per relay).
- **ContactInfo is public and signed.** It is published verbatim in the relay's descriptor
  (`router.c:3177-3182`) and parsed by eltord into `Relay.contact` (`eltord/src/rpc/get_relay_descriptors.rs:54-57`).
- **Embedding:** the app can run El Tor's `tor` with its own torrc and drive it with eltord's controllers
  (`eltor::client::run_foundation_client_controller`, `eltor::relay::run_foundation_relay_controller`). Payment state
  is available as the control-port event `ELTOR_PAYMENT_STATE` and through `ELTOR_PAYMENT_GET` and
  `ELTOR_PAYMENT_LIST`. One controller owns exactly one payment scope and does not rebuild it.
- **Wallets:** BOLT12 payers are Core Lightning, or Phoenixd through LNI. A receiving relay needs Core Lightning in
  Foundation mode.
- **There is no production El Tor network.** The public El Tor network runs in test mode from a single host, and El
  Tor cannot use the ordinary Tor network (its cells are 2048 bytes).

## Decision

### 1. Roles

**Seeder.** An El Tor Foundation relay plus a content server plus Core Lightning.

- The relay's exit policy accepts only its own content port ("exit-to-self"). The content server listens on loopback
  only, so it is reachable through that relay's exit and from nowhere else.
- Its Foundation terms are its earnings: `rate_msat` per round, and `paid_quota_bytes` set from the top rendition's
  bitrate × 60 s × headroom, so a paid minute buys about a minute of video. A free video has rate 0.
- v0: whoever seeds earns. A share for the creator when someone else seeds is out of scope for v0.

**Viewer.** An El Tor Foundation client plus a local player.

- It lists seeders and reads every candidate hop's signed terms from the directory before paying, and shows the
  total per-minute price (seeder + middle + guard + routing-fee estimate).
- It writes a plan pinning `[guard, middle, seeder]`, with a spending cap.
- A local proxy fetches segments over SOCKS by IP literal, checks each file's sha256, and serves localhost HLS to
  the player.
- It paces bytes under every hop's per-round quota, because exceeding a quota closes the circuit.
- Stop means stop paying; the circuit closes and playback ends.

### 2. Discovery (all El Tor)

- A seeder's ContactInfo carries a marker: `nutflix-eltor:v0 ck=<catalog key> port=<content port>` (format in
  [ADR 0002](0002-catalog-v0.md); the key is BIP-340, the same kind as Nostr keys). The descriptor is signed by
  the relay, so the key is bound to the relay.
- Viewers list seeders from the El Tor directory (full descriptors, which Foundation clients fetch anyway).
- The catalog (titles, renditions, hash-list roots; never prices, which are the relays' own signed terms) is signed
  with the catalog key and fetched over the paid circuit to that seeder. The first paid minute is the cost of
  browsing.
- Later: a free catalog port (patch P1) lets browsing ride the free allowance, and a dedicated descriptor line
  (patch P5) replaces the ContactInfo marker.

### 3. Data

NFX-05's layout, unchanged: CMAF/fMP4 renditions, 2-second segments, every file named by its sha256, and a hash list
per rendition. Every file is verified before use. Packaging reuses NFX's `nfx-media` crate, pinned to a release tag.
Keeping this layout costs nothing, and it means an `nfxd` and a nutflix-eltor seeder can hold byte-identical files.

### 4. Implementation shape

- Rust. The `eltor` crate is a git dependency pinned to an exact commit. Patches we need are carried on our fork and
  offered upstream.
- The app spawns El Tor's `tor` with a torrc it writes, and runs the eltord controllers in its own process. It does
  not use `run_with_args`, which relies on process-wide state.
- Lab first. Every milestone is proven in a local regtest network (bitcoind regtest, Core Lightning per relay,
  Chutney) on our own hosts. Hosted CI runs builds and unit tests only, never a network.

### 5. What we leave out

No iroh, no Nostr availability beacons, no Cashu, no NFX-07 to NFX-16. NFX compatibility is a later bridge
(issue #1).

## Consequences

- Payment and delivery cannot drift apart: a viewer is paying exactly while bytes flow, and a seeder cannot be paid
  for data it did not carry.
- Viewers are anonymous to seeders (three hops). Seeders are public relays, which they already are in NFX (their
  beacons are public).
- Every hop costs money, not only the seeder. Lightning routing fees dwarf 1-sat rounds, so prices and the fee
  estimate must be shown honestly before play.
- A real-money deployment needs our own El Tor network until Foundation runs outside test mode.
- Quota closes are hard, so the viewer must pace. A late payment (beyond the 60 s lead plus 15 s grace) closes the
  circuit, so the player buffers ahead and can resume on a new circuit, since segments are independent requests.

## To prove in M0

| Spike | Question | Kills the design if |
|---|---|---|
| S1 | Does a stream to the seeder's IP attach to the pinned paid circuit through the exit-to-self policy, with and without microdescriptors? | It cannot attach even with our own ATTACHSTREAM |
| S2 | What exactly happens at 0.9× and 1.1× the seeder's quota in one round; what are the per-hop byte counts? | Quotas can't be paced to avoid closes |
| S3 | How many segment bytes does a free-circuit freeloader get from the seeder? | Patch P1 is infeasible |
| S4 | Does 90 minutes of 720p play under long terms; can one tor run two payment scopes at once? | Throughput can't sustain 720p |
| S5 | Does a Foundation viewer hang on an ordinary onion service, as the code reading predicts? | (Confirms the catalog-over-paid-circuit choice) |

## Patches we expect to need

Carried on `SovereignTechnology/eltord` (and an `eltor` fork if needed), offered upstream:

- **P1 (C):** paid-only exit streams, with an optional free port for the catalog (`relay.c:1731-1746`).
- **P2 (C):** accept the end-to-end onion hop in the paid-stream readiness check (`payment_origin.c:932-955`), and
  keep onion intro circuits past the 10-minute free lifetime. Only if the catalog moves to an onion service.
- **P3 (Rust):** several or successive payment scopes per process, a per-session cap separate from the account
  budget, and stop-after-this-round.
- **P4 (Rust):** a Foundation terms encoder, a bounded descriptor read, and round and byte counters in the status.
- **P5 (C):** a dedicated signed descriptor line for content seeders.
- **P6 (C):** optional throttling instead of closing on quota.
