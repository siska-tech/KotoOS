# KotoOS Documentation

This directory is organized by theme. Start with
[planning/REQUIREMENTS.md](planning/REQUIREMENTS.md) and
[architecture/ARCHITECTURE.md](architecture/ARCHITECTURE.md) for the big
picture, or [guides/APP_DEV_LOOP.md](guides/APP_DEV_LOOP.md) to build an app.

| Directory | Contents |
| :-------- | :------- |
| [spec/](spec/) | Stable formats and contracts: the Koto app language, bytecode ABI, KPA package format, KotoMML audio format, SDK prelude, SKK dictionary format |
| [architecture/](architecture/) | System and crate-boundary architecture: overall design, KotoGFX/KotoVM boundaries, retained rendering, audio model, HAL API |
| [hardware/](hardware/) | PicoCalc bring-up and device work: RP2040 plan, LCD init profiles, keyboard matrix, hardware logs and debug guides |
| [guides/](guides/) | How-to docs for day-to-day development: the app dev loop and the asset pipeline |
| [planning/](planning/) | Requirements, research, validation plan, traceability, implementation status, and roadmaps |
| [devlog/](devlog/) | Development log: milestones, timeline, and deep-dive performance/budget analyses |
| [design/](design/) | Design notes (resource ownership model) |
| [research/](research/) | Long-form research material |
| [issues/](issues/) | Repository-local issue tracker, split into `main/` (KOTO-…), `kotogfx/` (GFX-…), and `diagnostics/` (DIAG-…) tracks |

Issue indexes: [ISSUES_main.md](ISSUES_main.md) tracks conventional KotoOS
work; [ISSUES_kotogfx.md](ISSUES_kotogfx.md) tracks the KotoGFX rendering
realignment.

`python harness\check_project.py` validates every markdown link and issue
file in this tree.
