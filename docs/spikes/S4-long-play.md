# S4: long play and two scopes

- **Status:** planned. Needs its own lab instance: 90+ minutes of wall clock.
- **Question:** does 90 minutes of 720p (about 3 Mbit/s) play over one pinned paid circuit under long terms
  (2 h, 12 periods on every hop), and can one tor process carry two payment scopes at once?
- **Kills the design if:** throughput cannot sustain 720p on loopback.
- **Known blocker:** el-tor/eltord#17. Both long-lived paid circuits the lab ran died about 2 minutes after the first
  period renewal, and the client then stops that scope for good. Until it is fixed, S4 records how play fails, and the
  viewer must survive losing a scope by opening a new one and moving segment fetches to it. That makes patch P3
  (successive scopes per process) a requirement, not an option, and S4's second question (two scopes on one tor)
  the important one.

## Setup

As S1, all three hops with `eltor_terms.py encode --rate-msat 1000 --minutes 120 --bitrate 3000000`. Client and
relays with `clock_uncertainty_ms` 1000 (a VM pause or clock step otherwise aborts the client). A paced fetcher that
pulls 2 s segments at real time.

## Evidence to record

Renewals seen in `ELTOR_PAYMENT_STATE` events, any stall longer than one segment, total bytes, invoices per hop
(expected 90 per hop), and the result of starting a second plan on the same tor.

## Verdict

Not run.
