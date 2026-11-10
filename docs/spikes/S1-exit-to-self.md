# S1: exit-to-self attach

- **Status:** planned. Runs once the El Tor regtest lab reaches a positive-price Foundation circuit.
- **Question:** does a SOCKS stream to the seeder's own IP:port attach to the pinned paid circuit
  `[guard, middle, seeder]`, where the seeder's exit policy accepts only that address?
- **Kills the design if:** it cannot attach, even when we attach streams ourselves (`ATTACHSTREAM`).

## Setup

- Chutney network on loopback with a distinct 127.0.x address per node (no exit-enclave shortcut).
- Seeder relay: `ElTorFoundation 1`, positive `ElTorFoundationTerms` (`lab/eltor_terms.py`), its own CLN offer,
  `ExitPolicyRejectPrivate 0`, `ExitPolicy accept <seeder ip>:<content port>`, `ExitPolicy reject *:*`.
  Content server bound to the seeder's address only, serving a file whose sha256 is known.
- Guard and middle: Foundation relays, non-exits.
- Client: `ElTorFoundation 1`, `__ElTorRequirePaidStreams 1`, plan pinning the three fingerprints in order.

## Runs

1. `UseMicrodescriptors 0` (full descriptors, the configuration upstream's acceptance run uses).
2. `UseMicrodescriptors 1`: expected to fail or wait, because microdescriptor exit summaries ignore single-address
   accepts. Recorded either way.

## Evidence to record

`curl --socks5 <client socks> http://<seeder ip>:<port>/<sha256>`: bytes, sha256 match, which circuit carried the
stream (`GETINFO stream-status`, `circuit-status`), the seeder CLN's settled invoices for the run.

## Verdict

Not run.
