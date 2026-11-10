# S2: quota as price

- **Status:** planned.
- **Question:** what happens when a viewer fetches 0.9× and 1.1× the seeder's `paid_quota_bytes` in one round,
  and how many bytes does each hop count?
- **Kills the design if:** quota closes cannot be avoided by pacing (for example, counting is not predictable from
  payload bytes).

## Setup

As S1, with the seeder quota Q = 50 MB per round (`eltor_terms.py encode --rate-msat 1000 --quota-bytes 50000000`);
guard and middle quota 0 (unlimited) in the first run, then 2Q in a second run.

## Evidence to record

Per-hop byte counters for the round (`ELTOR_PAYMENT_GET`), the close reason at 1.1Q (expected RESOURCELIMIT), the
payload-to-counted ratio per hop (expected about 1× at the seeder, about 2× at guard and middle).

## Verdict

Not run.
