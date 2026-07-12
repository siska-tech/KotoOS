# Issue Management — KotoGFX Migration

This index tracks the **KotoGFX rendering realignment** track only. Conventional /
non-migration work lives in [ISSUES_main.md](ISSUES_main.md) (issues under
`docs/issues/main/`). GFX migration issues live in `docs/issues/kotogfx/` and use a
separate `GFX-0000` ID series so the two tracks stay visually distinct.

The whole track is governed by
[KOTOGFX_RENDER_MIGRATION_PLAN.md](architecture/KOTOGFX_RENDER_MIGRATION_PLAN.md), which aligns
the current renderer with [kotogfx-architecture.md](architecture/kotogfx-architecture.md) and
[design/KOTOOS_RESOURCE_OWNERSHIP.md](design/KOTOOS_RESOURCE_OWNERSHIP.md). Read
the plan first — each issue below is one of its stages.

## Workflow

| Status        | Meaning                                                                |
| :------------ | :-------------------------------------------------------------------- |
| `todo`        | Accepted work that is not started                                      |
| `in-progress` | Currently being implemented or actively investigated                   |
| `done`        | Implemented and verified against its acceptance criteria               |

## Hard constraints (every GFX issue)

No change to: VM semantics, opcode values, bytecode ABI, hostcall IDs,
PSRAM/LCD/CodeWindow/audio behaviour, or KotoBlocks/KotoSnake/Sokoban behaviour;
no app bytecode rebuild. Each stage is individually behaviour-preserving (verbatim
move + re-export) except GFX-0006, the one behavioural-policy step, which is
separately gated by golden-frame parity.

## Issue Template

```markdown
# GFX-0000: Short Title

- Status: todo
- Type: feature | refactor | harness | docs | research | bug
- Priority: P0 | P1 | P2
- Requirements: FR-XXX-0, NFR-XXX-0

Source of truth: link into KOTOGFX_RENDER_MIGRATION_PLAN.md / arch docs.

## Goal
## Acceptance Criteria
## Notes
```

## Roadmap

Stages are strictly sequential 0001 → 0005 (each consumes the prior); 0006 depends
on 0002–0005; 0007 is docs-only and can land anytime.

| Issue | Status | Stage | Title | Risk |
| :---- | :----- | :---- | :---- | :--- |
| [GFX-0001](issues/kotogfx/GFX-0001-surface-geometry-into-koto-gfx.md) | done | 1 (first safe move) | App-surface geometry into koto-gfx | Low |
| [GFX-0002](issues/kotogfx/GFX-0002-retained-layer-data-model-into-koto-gfx.md) | done | 2 | Retained layer data model into koto-gfx | Low |
| [GFX-0003](issues/kotogfx/GFX-0003-dirty-derivation-into-koto-gfx.md) | done | 3 | Dirty derivation into koto-gfx | Low–Med |
| [GFX-0004](issues/kotogfx/GFX-0004-rasterizer-compositor-into-koto-gfx.md) | done | 4 | Rasterizer + compositor into koto-gfx | **High** (pixel parity) |
| [GFX-0005](issues/kotogfx/GFX-0005-display-service-extraction.md) | todo | 5 | Display service extraction (present-as-request) | Med |
| [GFX-0006](issues/kotogfx/GFX-0006-game2d-api-and-budgeted-immediate.md) | todo | 6 | KotoGame2D API + budgeted-immediate enforcement | Med (behavioural) |
| [GFX-0007](issues/kotogfx/GFX-0007-deprecate-legacy-immediate-base-path.md) | todo | 7 | Deprecate legacy per-frame-base path (docs) | Low |

## Follow-up investigations (GFX-0008+)

Issues opened after the staged migration table above; statuses come from each
issue file.

| Issue | Status | Title |
| :---- | :----- | :---- |
| [GFX-0008](issues/kotogfx/GFX-0008-commandcountshift-policy-refinement.md) | in-progress | CommandCountShift policy refinement (bounded-damage relaxation) |
| [GFX-0009](issues/kotogfx/GFX-0009-staticrebuild-cost-investigation.md) | todo | StaticRebuild cost investigation and reduction plan |
| [GFX-0010](issues/kotogfx/GFX-0010-rectsexceeded-pressure-investigation.md) | in-progress | RectsExceeded pressure investigation (coalesce-ordering defect) |
| [GFX-0011](issues/kotogfx/GFX-0011-commandcountshift-fallback-diagnosis.md) | in-progress | CommandCountShift fallback diagnosis and refinement |
| [GFX-0013](issues/kotogfx/GFX-0013-diffed-static-rebuild-damage.md) | in-progress | Diffed damage for mid-session static rebuilds |

## Test gate (all stages)

`cargo test -p koto-gfx`; `cargo test -p koto-pico` (the four `app_render`
invariants); `cargo test -p koto-sim` (golden-frame + budget observation);
`cargo build -p koto-pico --target thumbv6m-none-eabi --bins` (firmware is not in
the default clippy gate — lint it explicitly). Per-stage additions are in each
issue's Acceptance Criteria.
