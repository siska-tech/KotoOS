# KotoUI ABI canonical fixtures

`valid_panel_button_mount.hex` is a 142-byte `KUI1` mount packet containing a
320x320 root Panel (`id=1`, title `Demo`) and one enabled Button (`id=2`, label
`OK`) focused initially. A decoder must accept it and re-encode identical bytes.

`invalid_truncated_mount.hex` must fail with `BAD_ARGUMENT` without changing a
live session.

`valid_en_us_capabilities.hex` is the canonical 64-byte `KUC1` response with
the v1 minimum capacities, locale `en-US`, LTR direction, and generation 1.

Canonical malformed mutations of the valid packet are:

| Case | Mutation | Expected status |
| :-- | :-- | :-- |
| Unsupported format | bytes 4..6 = `02 00` | `UNSUPPORTED` |
| Too many nodes | bytes 12..14 = `21 00` | `NO_MEMORY` |
| Duplicate widget ID | bytes 88..90 = `01 00` | `BAD_ARGUMENT` |
| Parent is not prior | bytes 90..92 = `02 00` | `BAD_ARGUMENT` |
| Invalid UTF-8 | byte 141 = `ff` | `BAD_ARGUMENT` |
| Nonzero reserved | byte 36 = `01` | `BAD_ARGUMENT` |

Offsets are zero-based and end-exclusive in the table. KOTO-0218 runtime tests
and KOTO-0219 compiler tests share these files.
