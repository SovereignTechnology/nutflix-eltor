#!/usr/bin/env python3
"""El Tor Foundation terms (`ElTorFoundationTerms`): encode, decode, and size a seeder's quota.

The format is "canonical finite v1 policy terms": 55 bytes, unsigned big-endian, written as 110 lowercase hex
characters. Source of truth (El Tor): eltor `src/feature/payment/payment_profile.c` (encoder 55-75, decoder 77-102,
validity 48-53 and 86-89), eltord `spec/01a_circuit_renewal_wire.md` and `spec/vectors/payment_renewal_v1.json`.
eltord ships a decoder only (`src/rpc/payment_policy.rs`); patch P4 upstreams an encoder.

Stdlib only, so lab scripts can use it inside an offline guest.
"""
from __future__ import annotations

import math
import struct
from dataclasses import dataclass

_FMT = ">BBBQIIIQQIQI"
TERMS_LEN = 55
ROUND_MS = 60_000  # fixed in v1
LEAD_MS = 60_000  # fixed in v1
ROUNDS_PER_PERIOD = 10  # batch_count, fixed in v1
PERIOD_MS = ROUND_MS * ROUNDS_PER_PERIOD


@dataclass(frozen=True)
class Terms:
    rate_msat: int  # price per 60 s round; 0 = zero-priced
    paid_quota_bytes: int = 0  # per round; 0 = unlimited paid quota
    grace_ms: int = 15_000  # <= 60000
    max_age_ms: int = 1_800_000  # > 0
    max_periods: int = 3  # >= 2, counts period 0
    handshake_fee_msat: int = 0
    setup_timeout_ms: int = 30_000  # > 0

    def validate(self) -> None:
        for name in ("rate_msat", "paid_quota_bytes", "max_age_ms", "handshake_fee_msat"):
            v = getattr(self, name)
            if not 0 <= v < 2**64:
                raise ValueError(f"{name} out of u64 range")
        if not 0 <= self.grace_ms <= 60_000:
            raise ValueError("grace_ms must be 0..60000")
        if self.max_age_ms <= 0:
            raise ValueError("max_age_ms must be > 0")
        if not 2 <= self.max_periods < 2**32:
            raise ValueError("max_periods must be >= 2")
        if not 0 < self.setup_timeout_ms < 2**32:
            raise ValueError("setup_timeout_ms must be > 0")

    def encode(self) -> str:
        """110 lowercase hex characters, ready for `ElTorFoundationTerms` in a relay's torrc."""
        self.validate()
        b = struct.pack(_FMT, 1, 1, ROUNDS_PER_PERIOD, self.rate_msat, ROUND_MS, LEAD_MS, self.grace_ms,
                        self.paid_quota_bytes, self.max_age_ms, self.max_periods, self.handshake_fee_msat,
                        self.setup_timeout_ms)
        assert len(b) == TERMS_LEN
        return b.hex()

    @classmethod
    def decode(cls, hex_terms: str) -> "Terms":
        """Strict, like El Tor's parser: exact length, lowercase hex, v1 constants."""
        if len(hex_terms) != 2 * TERMS_LEN or hex_terms != hex_terms.lower():
            raise ValueError("terms must be 110 lowercase hex characters")
        (version, mode, batch, rate, round_ms, lead_ms, grace, quota, max_age, periods, fee,
         setup) = struct.unpack(_FMT, bytes.fromhex(hex_terms))
        if (version, mode, batch, round_ms, lead_ms) != (1, 1, ROUNDS_PER_PERIOD, ROUND_MS, LEAD_MS):
            raise ValueError("not canonical v1 terms")
        t = cls(rate, quota, grace, max_age, periods, fee, setup)
        t.validate()
        return t

    def usable_minutes(self) -> int:
        """Rounds a circuit can be funded for: renewal of period p needs p < max_periods and
        p * 600000 + 600000 <= max_age_ms (payment_profile.c:397-400, as read in the lab notes)."""
        periods = min(self.max_periods, self.max_age_ms // PERIOD_MS)
        return periods * ROUNDS_PER_PERIOD


def quota_for_bitrate(bits_per_second: int, headroom: float = 1.25) -> int:
    """Per-round byte quota that lets a viewer fetch one minute of a stream at `bits_per_second`
    (the top rendition's peak bitrate, container overhead included) with `headroom` for buffering.
    This is the seeder's quota: the last hop counts payload bytes about once."""
    if bits_per_second <= 0 or headroom < 1.0:
        raise ValueError("bitrate must be > 0 and headroom >= 1.0")
    return math.ceil(bits_per_second / 8 * (ROUND_MS / 1000) * headroom)


def terms_for_long_play(rate_msat: int, minutes: int, paid_quota_bytes: int = 0) -> Terms:
    """Terms whose max age and periods cover `minutes` of play (rounded up to whole 10-round periods)."""
    periods = max(2, math.ceil(minutes / ROUNDS_PER_PERIOD))
    return Terms(rate_msat=rate_msat, paid_quota_bytes=paid_quota_bytes, max_age_ms=periods * PERIOD_MS,
                 max_periods=periods)


if __name__ == "__main__":
    import argparse
    import json

    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    e = sub.add_parser("encode", help="print ElTorFoundationTerms hex")
    e.add_argument("--rate-msat", type=int, required=True)
    e.add_argument("--quota-bytes", type=int, default=0)
    e.add_argument("--bitrate", type=int, help="size the quota from a peak bitrate (bit/s) instead")
    e.add_argument("--minutes", type=int, default=30, help="max play per circuit (whole 10-minute periods)")
    d = sub.add_parser("decode", help="print the fields of a terms hex")
    d.add_argument("hex")
    a = ap.parse_args()
    if a.cmd == "encode":
        quota = quota_for_bitrate(a.bitrate) if a.bitrate else a.quota_bytes
        print(terms_for_long_play(a.rate_msat, a.minutes, quota).encode())
    else:
        t = Terms.decode(a.hex)
        print(json.dumps({**t.__dict__, "usable_minutes": t.usable_minutes()}, indent=2))
