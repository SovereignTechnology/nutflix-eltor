# S3: free-circuit freeloader

- **Status:** planned.
- **Question:** how many segment bytes can a client get from an exit-to-self seeder without paying, over a circuit
  with no payment profile (the relays' free allowance)?
- **Kills the design if:** patch P1 (paid-only exit streams) turns out infeasible; until then, this sizes the leak.

## Setup

As S1, plus a second Foundation client with no plan (free circuits only), and a plain El Tor client without
`ElTorFoundation`.

## Evidence to record

Bytes received per client before the circuit closes or stalls, elapsed time, close reasons. Expected: a trickle
(the free allowance is 2 cells/s with a 128-cell burst, for at most 10 minutes per circuit).

## Verdict

Not run.
