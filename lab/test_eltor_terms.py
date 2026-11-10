"""Vectors for lab/eltor_terms.py.

The two fixtures are El Tor's own: eltord `spec/vectors/payment_renewal_v1.json` at 9f589be,
`.fixtures[0].terms_hex` and `.fixtures[1].terms_hex` (rate 1000 / 2000 msat, 24 h, 144 periods).
The zero-price default is `payment_profile.c:10-16` (what a relay uses when the option is omitted).
"""
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from eltor_terms import Terms, quota_for_bitrate, terms_for_long_play  # noqa: E402

SPEC_HOP1 = ("01010a00000000000003e80000ea600000ea6000003a9800000000000000000000000005265c"
             "0000000090000000000000000000007530")
SPEC_HOP2 = ("01010a00000000000007d00000ea600000ea6000003a9800000000000000000000000005265c"
             "0000000090000000000000000000007530")
ZERO_DEFAULT = ("01010a00000000000000000000ea600000ea6000003a98000000000000000000000000001b7740"
                "00000003000000000000000000007530")


class TermsVectors(unittest.TestCase):
    def test_spec_fixtures_encode(self):
        day = 24 * 3600 * 1000
        self.assertEqual(Terms(rate_msat=1000, max_age_ms=day, max_periods=144).encode(), SPEC_HOP1)
        self.assertEqual(Terms(rate_msat=2000, max_age_ms=day, max_periods=144).encode(), SPEC_HOP2)

    def test_zero_price_default(self):
        self.assertEqual(Terms(rate_msat=0).encode(), ZERO_DEFAULT)

    def test_round_trip(self):
        for h in (SPEC_HOP1, SPEC_HOP2, ZERO_DEFAULT):
            self.assertEqual(Terms.decode(h).encode(), h)

    def test_decode_is_strict(self):
        with self.assertRaises(ValueError):
            Terms.decode(SPEC_HOP1.upper())
        with self.assertRaises(ValueError):
            Terms.decode(SPEC_HOP1[:-2])
        with self.assertRaises(ValueError):
            Terms.decode("02" + SPEC_HOP1[2:])  # version 2
        with self.assertRaises(ValueError):
            Terms.decode(SPEC_HOP1[:4] + "09" + SPEC_HOP1[6:])  # batch_count 9

    def test_validation(self):
        with self.assertRaises(ValueError):
            Terms(rate_msat=1, grace_ms=60_001).encode()
        with self.assertRaises(ValueError):
            Terms(rate_msat=1, max_periods=1).encode()
        with self.assertRaises(ValueError):
            Terms(rate_msat=-1).encode()

    def test_quota_and_long_play(self):
        # 720p top rendition at 3 Mbit/s peak: 22.5 MB per minute, 28.125 MB with 1.25 headroom.
        self.assertEqual(quota_for_bitrate(3_000_000), 28_125_000)
        t = terms_for_long_play(1000, 120, quota_for_bitrate(3_000_000))
        self.assertEqual((t.max_periods, t.max_age_ms), (12, 7_200_000))
        self.assertEqual(t.usable_minutes(), 120)
        self.assertEqual(Terms.decode(t.encode()), t)


if __name__ == "__main__":
    unittest.main()
