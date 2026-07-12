# KOTO-0054: Asset Development Pipeline

- Status: done
- Type: feature
- Priority: P2
- Requirements: FR-PKG-2, FR-FONT-1, FR-FONT-2, FR-VN-2, FR-MML-1, NFR-DEV-3, NFR-DEV-4

## Goal

Define and prototype the host-side asset conversion pipeline needed by future
apps, including image conversion, font preview/conversion, audio or MML assets,
and package layout validation.

## Acceptance Criteria

- [x] A design note lists supported source asset types, generated Koto asset
      formats, and where each fits in the KPA package layout.
- [x] At least one image or icon conversion path can generate a package-ready
      asset and preview output.
- [x] Font preview or validation tooling can confirm glyph metrics and sample
      Japanese text rendering.
- [x] The package harness can verify sequential asset placement for generated
      assets.

## Notes

This issue does not need to complete all future media formats. It should create
the pattern that KotoVN, KotoDOS, KotoMML, and PicoMings can extend.

Completed by [ASSET_PIPELINE.md](../../guides/ASSET_PIPELINE.md),
`harness/asset_pipeline.py`, and the `harness/fixtures/asset_pipeline` checks.
