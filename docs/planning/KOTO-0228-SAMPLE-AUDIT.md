# KOTO-0228 sample application audit

Date: 2026-07-17

The sample suite was audited for App-lifetime logical state that benefits from
KOTO-0228 `struct` / `static` / `impl` organization. Records were introduced
only where several values form one state machine or where a state transition
has a useful receiver-method boundary. Frame-local scratch values remain local.

| Sample | Decision | Result or reason |
| :----- | :------- | :--------------- |
| Counter Loop | Migrated | `CounterState` owns the counter; `increment` performs its transition. |
| Dirty Rects | Migrated | `MotionState` owns the x position; `advance` also owns wraparound. |
| Audio Codecs | Migrated | `RegressionState` groups the soak's running flag and frame counter. |
| Full Color Image Gallery | Migrated | `GalleryState` groups scene, phase, timer, and transition step. |
| Retained Tilemap | Migrated | `TilemapState` owns parsed row width and bounded upload progress. |
| Retained Tilemap Scroll | Migrated | `ScrollState` owns camera position and restartable upload progress. |
| KotoUI Gallery | Migrated | `GalleryState` replaces the raw two-byte business-state buffer and groups selection, localized status, and resource kind. Protocol/event/locale buffers remain packed. |
| Hello Text | Kept | Stateless apart from the current frame's intent. |
| Input Echo | Kept | Its persistent value is bounded byte text, not 32-bit record fields. |
| IME Playground | Kept | Its persistent value is the host-facing bounded composition byte buffer. |
| Actor Array | Kept | `ActorArray` deliberately uses the compact packed SDK representation documented by KOTO-0228. |
| File Note | Kept | Document text, KotoUI packets, locale tables, and events are bounded packed buffers; the remaining scalar metadata is mixed-width and tightly coupled to those layouts. The source also had concurrent uncommitted work during this audit. |

This keeps the examples honest about the V1 boundary: typed records are for
cohesive `int` / `bool` state, while byte strings, ABI packets, packed actor
pools, images, maps, and other bulk data continue to use bounded buffers or
their SDK-owned representations.
