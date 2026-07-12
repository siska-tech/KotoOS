# KOTO-0059: Roadmap State Cleanup

- Status: done
- Type: docs
- Priority: P0
- Requirements: NFR-DEV-4

## Goal

Separate the completed KotoSim baseline from the next active roadmap so project
state is readable without scanning every historical issue.

## Acceptance Criteria

- [x] `docs/ISSUES.md` distinguishes active work from the completed baseline.
- [x] The issue status definition explains that `done` is scoped to each issue's
  acceptance criteria.
- [x] New follow-up issues exist for simulator cleanup, implementation-status
  mapping, and embedded bring-up.

## Notes

This is a bookkeeping cleanup. It does not change runtime behavior.
