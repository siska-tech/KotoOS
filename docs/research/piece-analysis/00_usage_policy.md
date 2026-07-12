# P/ECE Research Usage Policy for KotoOS

## Purpose

This document defines how P/ECE SDK, documents, samples, and hardware information
may be used as research material for KotoOS.

## Summary

P/ECE explicitly allows users to develop, distribute, and sell software for P/ECE
without platform license or royalty payment.

However, the available materials do not clearly grant permission to copy,
modify, redistribute, or incorporate the P/ECE SDK, headers, libraries, bundled
source code, documents, fonts, images, sounds, or other assets into KotoOS.

Therefore, KotoOS treats P/ECE materials as reference material only.

## Allowed

- Studying P/ECE hardware architecture
- Studying API categories and design intent
- Studying application lifecycle patterns
- Studying sample application structure at a conceptual level
- Creating original KotoOS APIs inspired by observed design goals
- Writing new documentation in our own words
- Writing new sample applications from scratch

## Not Allowed

- Copying P/ECE source code into KotoOS
- Translating P/ECE source code line-by-line
- Copying P/ECE headers or function declarations verbatim
- Copying documentation text
- Copying diagrams
- Reusing bundled fonts
- Reusing images, sounds, music, or other assets
- Using P/ECE names/logos in a way that implies endorsement or compatibility

## Clean-room rule

When implementing KotoOS features inspired by P/ECE, use a two-step process:

1. Research note:
   Describe observed behavior, API role, and design intent without copying code.

2. Implementation:
   Implement KotoOS behavior from the research note, using original Rust code,
   original API naming where appropriate, and KotoOS-specific architecture.

## Compatibility statement

KotoOS does not aim to be a P/ECE-compatible runtime.
KotoOS may provide a P/ECE-inspired development experience for PicoCalc,
but it is an independent system.