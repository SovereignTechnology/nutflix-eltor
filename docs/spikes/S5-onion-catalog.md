# S5: onion catalog

- **Status:** planned.
- **Question:** can a Foundation viewer (`__ElTorRequirePaidStreams 1`) fetch a catalog from an ordinary, free onion
  service on the El Tor network?
- **Expected:** no. The end-to-end onion hop carries no payment keys, so the paid-stream readiness check never
  passes (`eltor src/feature/payment/payment_origin.c:938-955`). This spike confirms ADR 0001's choice to fetch the
  catalog over the paid circuit, and sizes patch P2 if we ever want onion catalogs.

## Setup

As S1, plus a free v3 onion service on a fourth relay host serving a small JSON file.

## Evidence to record

Whether the descriptor fetch, introduction and rendezvous complete, where the stream waits, and the client log.

## Verdict

Not run.
