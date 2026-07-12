# GFX-0007: Deprecate legacy per-frame-base immediate path (docs)

- Status: todo
- Type: docs
- Priority: P2
- Requirements: FR-RT-5

Source of truth: [KOTOGFX_RENDER_MIGRATION_PLAN.md](../../architecture/KOTOGFX_RENDER_MIGRATION_PLAN.md)
§3 (legacy compatibility path) + §4 Stage 7.

Depends on: [GFX-0006](GFX-0006-game2d-api-and-budgeted-immediate.md).

## Goal

Document the legacy immediate path — apps that emit a full-screen base `Rect`
every frame plus per-frame `draw_text` / `draw_rect`, handled by
`full_screen_base_in` / `present_app_commands` — as **deprecated**, and define the
condition for its eventual removal. No code or bytecode change in this issue.

## Acceptance Criteria

- [ ] The retained model is documented for app authors; immediate draw is
      documented as fallback / debug / overlay only (closes the
      [KOTO-0144](../main/KOTO-0144-game2d-api-cleanup-retained-docs.md) intent for
      the GFX track).
- [ ] The legacy per-frame-base path is marked deprecated with a stated removal
      precondition: **all shipped apps emit retained layers**.
- [ ] No app bytecode is rebuilt; the legacy path keeps working byte-for-byte
      until its precondition is met (removal is out of scope here).

## Notes

This is the cleanup capstone. The three immediate roles that survive permanently
(panic/debug/fallback, budgeted overlay, legacy compat) are restated so a future
reader does not re-introduce immediate draw as the default frame path.
