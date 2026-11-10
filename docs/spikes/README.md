# Spike records

One file per spike, `S<N>-<slug>.md`, written as the spike runs. A spike answers one question from
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
| S1 exit-to-self attach | not started | |
| S2 quota at 0.9× and 1.1× | not started | |
| S3 free-circuit freeloader | not started | |
| S4 90-minute play, two scopes | not started | |
| S5 onion catalog | not started | |
