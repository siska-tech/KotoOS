# KOTO-0179: repository cleanup for public release

- Status: todo
- Type: harness
- Priority: P1 (blocks publication)
- Related: KOTO-0180 (vendor koto-audio — the other publication blocker),
  KOTO-0063 (doc/implementation status map — the audit muscle to reuse).

## Goal

The repository currently carries far too much working/scratch data to be
publishable. Inventory and either delete, relocate, or explicitly bless every
tracked artifact so the tree is presentable as a public open-source project.

## Candidate categories (to be inventoried, not a final list)

- Working captures/logs, ad-hoc benchmark outputs, device session dumps.
- Generated artifacts that are rebuildable (`sdcard_mock/bytecode/*.kbc` is
  build output of `apps/*` — decide: keep for test fixtures vs. build in CI).
- Historical planning docs that describe abandoned directions (mark as
  archived rather than delete where they explain decisions).
- `docs/issues/` stays — it is the project's decision log and a feature of
  the repo, not clutter.
- Anything containing personal paths, credentials, or device serials.

## Acceptance Criteria

- [ ] Written inventory of tracked non-source data with a keep/move/delete
      decision per item.
- [ ] Deletions and moves executed; `.gitignore` extended so scratch data
      cannot re-accumulate.
- [ ] A fresh clone builds and passes `check_all` with nothing referencing
      removed files.
- [ ] README-level entry points (build, run sim, flash device) verified
      against the cleaned tree.
