# Spike records

One file per spike, `S<N>-<slug>.md`, written as the spike runs.

**Lab:** `testnet/labctl` on `SovereignTechnology/eltord`, branch `testnet/noissue-regtest-lab` at `fe6b8d6c4251`
(pinned).
- **Network:** bitcoind regtest; Core Lightning for the payer, each paid relay and a shared zero-price node; a
  Chutney `basic-min` network (authorities `000a`–`003a`, exit `004r`, middles `006m`–`008m`, Foundation client
  `005c` on SOCKS 9060, Foundation-off client `009c`).
- **Origin:** a checksummed origin at `127.0.0.1:18880` (`GET /bytes/<n>?seed=<s>&rate=<B/s>`, at most 64 MiB).
- **Scenarios:** `labctl up` ends with a checksummed fetch through the paid-only client. `labctl scenario renewal`
  and `labctl scenario payer-stop` each print one PASS/FAIL JSON line.
- **Rules:** every listener binds 127.0.x.y. `$ELTOR_LAB_STATE` is an absolute directory we own (0700), marked
  `.eltor-lab`. Spikes run on an instance of our own, never on another project's lab guests.

**What the lab has already shown** (lab owner's runs, 2026-11-10):
- **payer-stop PASS:** the circuit closed 79 s after the payer stopped, with 0 free-fallback hits and 0 settlements
  while stopped; a fresh path was back in 8 s. This sets M2's "Stop" criterion (roadmap).
- **El Tor bug el-tor/eltord#17:** in about 1 in 4 fresh constructions, the relay controllers resynchronize, the
  client's first status query gets `UnknownCircuit`, and the client stops that scope for good. Both long-lived paid
  circuits run so far died about 2 minutes after the first period renewal (rounds 11–12). Expect S4 to reproduce
  this until it is fixed.
- **Narrow exits:** narrowing the exits' policy to the origin left the client with no usable exit, so the lab keeps
  Chutney's permissive exits. S1 asks the same question with the exit pinned in a Foundation plan.
- **Other El Tor bugs filed by the lab:** el-tor/eltord#14, #15, #16; el-tor/eltor#11 (with Foundation on, tor
  relaunches circuits it cannot build every second); el-tor/chutney#1; and PR el-tor/eltord#18. A spike answers one question from
[ADR 0001](../decisions/0001-architecture.md#to-prove-in-m0) in the regtest lab and ends with a verdict.

Each record has:

- **Question** and the kill condition from the ADR.
- **Setup:** El Tor commits, lab topology, torrc and terms (no secrets: control passwords, plans and wallet material
  are never committed).
- **Commands**, exactly as run.
- **Evidence:** settled invoices on the receiving nodes, segment sha256 matches, circuit close reasons, per-hop byte
  counters, timings.
- **Verdict:** PASS, FAIL or PARTIAL, and what changes in the design.

| Spike | Record | Verdict |
|---|---|---|
| S1 exit-to-self attach | [S1](S1-exit-to-self.md) | planned |
| S2 quota at 0.9× and 1.1× | [S2](S2-quota.md) | planned |
| S3 free-circuit freeloader | [S3](S3-freeloader.md) | planned |
| S4 90-minute play, two scopes | [S4](S4-long-play.md) | planned |
| S5 onion catalog | [S5](S5-onion-catalog.md) | planned |
