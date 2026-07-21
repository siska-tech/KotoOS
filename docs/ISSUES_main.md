# Issue Management

KotoOS uses lightweight repository-local issues until an external tracker is introduced. This index covers **conventional KotoOS work**; each such issue lives in `docs/issues/main/` and has a stable `KOTO-0000` style ID.

The **KotoGFX rendering migration** is tracked separately in [ISSUES_kotogfx.md](ISSUES_kotogfx.md) (issues under `docs/issues/kotogfx/`, `GFX-0000` ID series).

## Workflow

| Status        | Meaning                                                                |
| :------------ | :--------------------------------------------------------------------- |
| `todo`        | Accepted work that is not started                                      |
| `in-progress` | Currently being implemented or actively investigated                   |
| `done`        | Implemented or researched and verified against its acceptance criteria |

Issues are scoped to their stated acceptance criteria. A `done` research or
planning issue does not imply that the downstream implementation or hardware
validation is complete.

## Issue Template

```markdown
# KOTO-0000: Short Title

- Status: todo
- Type: feature | harness | docs | research | bug
- Priority: P0 | P1 | P2
- Requirements: FR-XXX-0, NFR-XXX-0

## Goal

What outcome this issue creates.

## Acceptance Criteria

- [ ] Observable condition.

## Notes

Design notes, risks, and links.
```

## Active Roadmap

### 2026-07 Triage Backlog

Filed 2026-07-11 from a device-testing triage session. KotoSoko (the Sokoban
rewrite) was deleted from the repo in the same session; `apps/sokoban` remains
and is tracked by KOTO-0185.

| Issue                                                                  | Status | Title                                                  |
| :--------------------------------------------------------------------- | :----- | :------------------------------------------------------ |
| [KOTO-0177](issues/main/KOTO-0177-firmware-exit-key-parity.md)              | done   | Firmware exit keys diverge from sim (X/Esc exit; F10 only) |
| [KOTO-0178](issues/main/KOTO-0178-sdk-sample-audit.md)                      | done   | SDK sample audit — dirty_rects hang = frame-1 text-strip panic-halt, fixed + device-confirmed |
| [KOTO-0179](issues/main/KOTO-0179-repo-cleanup-for-publication.md)          | todo   | Repository cleanup for public release                    |
| [KOTO-0180](issues/main/KOTO-0180-vendor-koto-audio.md)                     | todo   | Vendor koto-audio into the KotoOS workspace              |
| [KOTO-0181](issues/main/KOTO-0181-boot-splash-screen.md)                    | todo   | Boot splash screen                                       |
| [KOTO-0182](issues/main/KOTO-0182-memory-status-tool.md)                    | todo   | SRAM/PSRAM memory status visibility tool                 |
| [KOTO-0183](issues/main/KOTO-0183-koto-language-include.md)                 | todo   | Koto language source-file splitting (include)            |
| [KOTO-0184](issues/main/KOTO-0184-audio-gfx-dev-tooling.md)                 | todo   | KotoAudio/KotoGFX developer support tooling              |
| [KOTO-0185](issues/main/KOTO-0185-sokoban-broken.md)                        | todo   | Sokoban does not work correctly                          |
| [KOTO-0186](issues/main/KOTO-0186-core1-audio-worker-stack-overflow.md)     | todo   | Core1 audio worker stack overflow under LTO (deferred to post-KOTO-0180) |

### KotoIDE Toolset

Filed 2026-07-13 from the [KotoIDE roadmap](planning/KOTOIDE_ROADMAP.md) —
code/graphics/sound authoring, conversion, and audition tooling. Phase 1
(KOTO-0187..0189) items are independent; Phase 2 builds on Phase 1; Phase 3
(language intelligence) is deliberately last. The cue-table dry-run and
retained-layer inspector candidates stay in KOTO-0184.

| Issue                                                                  | Status | Title                                                  |
| :--------------------------------------------------------------------- | :----- | :------------------------------------------------------ |
| [KOTO-0187](issues/main/KOTO-0187-koto-img-sprite-converter.md)             | done   | koto-img — PNG ↔ .kspr sprite converter                  |
| [KOTO-0188](issues/main/KOTO-0188-kmml-audition-cli.md)                     | done   | .kmml audition — render to WAV and play on the host      |
| [KOTO-0189](issues/main/KOTO-0189-kwt-pcm16-bake.md)                        | done   | Bake Native KotoAudio phrases to PCM16 KACL clips |
| [KOTO-0190](issues/main/KOTO-0190-vscode-extension-foundation.md)           | done   | VS Code extension foundation (grammars, tasks, matcher)  |
| [KOTO-0191](issues/main/KOTO-0191-sim-watch-live-reload.md)                 | done   | koto-sim --watch — live reload on source/asset change    |
| [KOTO-0192](issues/main/KOTO-0192-asset-custom-editors.md)                  | done   | VS Code custom editors for .kspr and .kmml               |
| [KOTO-0193](issues/main/KOTO-0193-compiler-library-diagnostics.md)          | done   | koto-compiler library split with structured diagnostics  |
| [KOTO-0194](issues/main/KOTO-0194-koto-lsp.md)                              | in-progress | koto-lsp — live diagnostics, definitions, budget inlays |
| [KOTO-0195](issues/main/KOTO-0195-per-app-descriptor.md)                    | done   | Per-app app.json descriptor (split apps/apps.json)       |
| [KOTO-0196](issues/main/KOTO-0196-app-json-schema-icon-editor.md)           | done   | app.json authoring + .kicon editor in VS Code            |
| [KOTO-0197](issues/main/KOTO-0197-vscode-new-app-project-wizard.md)         | in-progress | VS Code new-app project wizard                       |
| [KOTO-0198](issues/main/KOTO-0198-vscode-tilemap-editor.md)                 | in-progress | VS Code tilemap editor                               |
| [KOTO-0199](issues/main/KOTO-0199-generic-retained-tilemap.md)              | done   | Generic 20x20 retained tilemap geometry                  |
| [KOTO-0200](issues/main/KOTO-0200-retained-tilemap-samples.md)              | done   | Retained tilemap display and scrolling samples           |
| [KOTO-0201](issues/main/KOTO-0201-retained-first-present-without-base.md)    | done   | Retained first present without a full-screen base         |
| [KOTO-0202](issues/main/KOTO-0202-package-map-assets.md)                     | done   | Package-authored map assets                               |
| [KOTO-0203](issues/main/KOTO-0203-vscode-tilemap-kspr-preview.md)             | done   | VS Code tilemap `.kspr` preview                           |

### RP2350 / Pico 2 Compatibility

Filed 2026-07-14 from PicoCalc community feedback. The available RP2350A Pico
2 W is the first implementation and hardware-validation target. RP2350B and
the Pico Plus 2(W) module-PSRAM backend follow after that gate. See the
[RP2350 support roadmap](planning/RP2350_SUPPORT_ROADMAP.md).

| Issue                                                                  | Status | Title                                                  |
| :--------------------------------------------------------------------- | :----- | :----------------------------------------------------- |
| [KOTO-0204](issues/main/KOTO-0204-rp2350-build-board-profiles.md)          | done   | RP2350A Pico 2 W build foundation                     |
| [KOTO-0205](issues/main/KOTO-0205-rp2350-picocalc-peripheral-parity.md)    | in-progress | RP2350A Pico 2 W PicoCalc peripheral parity       |
| [KOTO-0206](issues/main/KOTO-0206-pico-plus-2-onboard-psram.md)            | todo   | RP2350B Pico Plus 2(W) and onboard PSRAM backend      |

### Storage Performance

Filed 2026-07-14 after the full-color streaming sample exposed 1 MHz SD
throughput and audio starvation during image loads.

| Issue                                                                  | Status      | Title                                      |
| :--------------------------------------------------------------------- | :---------- | :----------------------------------------- |
| [KOTO-0207](issues/main/KOTO-0207-sd-spi-init-and-throughput.md)         | done        | SD SPI initialization and transfer throughput |

### RP2040 Memory And Audio Reliability

The RP2040 product profile keeps a measured stack canary margin while sharing
bounded CPU0/CPU1 audio state. Changes in this section require product-path
audio stress and ELF SRAM measurements rather than build-only validation.

| Issue                                                                  | Status | Title                                      |
| :--------------------------------------------------------------------- | :----- | :----------------------------------------- |
| [KOTO-0226](issues/main/KOTO-0226-cpu0-audio-scratch-sharing-regression-harness.md) | done | CPU0 audio scratch sharing regression harness |
| [KOTO-0227](issues/main/KOTO-0227-pico-w-switchable-audio-wifi-sram-residency.md) | done | Pico W switchable Audio/Wi-Fi SRAM residency |

### KotoUI GUI Components

Filed 2026-07-15 from the
[KotoUI GUI component roadmap](planning/KOTOUI_ROADMAP.md). The work keeps UI
state allocation-free and keyboard-first, builds components on an abstract
painter/damage contract, validates them in the simulator, and finishes with a
bounded KotoShell production pilot.

| Issue                                                                  | Status | Title                                           |
| :--------------------------------------------------------------------- | :----- | :---------------------------------------------- |
| [KOTO-0208](issues/main/KOTO-0208-koto-ui-foundation.md)                   | done   | KotoUI allocation-free foundation           |
| [KOTO-0209](issues/main/KOTO-0209-koto-ui-focus-events.md)                   | done   | KotoUI keyboard events and focus routing    |
| [KOTO-0210](issues/main/KOTO-0210-koto-ui-basic-controls.md)                 | done   | KotoUI label, button, and checkbox controls |
| [KOTO-0211](issues/main/KOTO-0211-koto-ui-list.md)                           | done   | KotoUI bounded list and selection control   |
| [KOTO-0212](issues/main/KOTO-0212-koto-ui-text-field.md)                     | done   | KotoUI single-line text field and IME composition |
| [KOTO-0213](issues/main/KOTO-0213-koto-ui-dialog-composition.md)             | done   | KotoUI panel and modal dialog composition       |
| [KOTO-0214](issues/main/KOTO-0214-koto-ui-rendering-integration.md)          | done   | KotoUI rendering and platform integration       |
| [KOTO-0215](issues/main/KOTO-0215-koto-ui-gallery-validation.md)             | done   | KotoUI component gallery and regression harness |
| [KOTO-0216](issues/main/KOTO-0216-koto-ui-shell-pilot.md)                    | done   | KotoShell pilot adoption of KotoUI              |

### KotoUI App ABI

Filed 2026-07-15 from the
[KotoUI App ABI roadmap](planning/KOTOUI_APP_ABI_ROADMAP.md). This phase exposes
the bounded native component model to sandboxed KotoRuntime applications through
a versioned retained description, semantic events, SDK builders, an app-authored
Gallery, and an existing-app pilot.

| Issue                                                                  | Status | Title                                      |
| :--------------------------------------------------------------------- | :----- | :----------------------------------------- |
| [KOTO-0217](issues/main/KOTO-0217-koto-ui-app-abi-design.md)              | done   | App-facing KotoUI ABI and locale contract  |
| [KOTO-0218](issues/main/KOTO-0218-koto-ui-vm-session-host-calls.md)        | done   | KotoUI VM session and host calls        |
| [KOTO-0219](issues/main/KOTO-0219-koto-ui-sdk-builders.md)                 | done   | KotoSDK UI builders and event API       |
| [KOTO-0220](issues/main/KOTO-0220-koto-ui-app-gallery.md)                  | in-progress | App-authored KotoUI component Gallery      |
| [KOTO-0221](issues/main/KOTO-0221-file-note-koto-ui-pilot.md)              | done   | File Note app pilot migration to KotoUI    |
| [KOTO-0222](issues/main/KOTO-0222-kotoshell-ja-en-localization.md)          | done   | KotoShell Japanese/English localization    |
| [KOTO-0229](issues/main/KOTO-0229-koto-ui-stateful-builders.md)             | done   | KotoUI stateful App builders               |
| [KOTO-0230](issues/main/KOTO-0230-sdk-text-resources-list-row-builders.md)   | done   | SDK text resources and List row builders   |
| [KOTO-0231](issues/main/KOTO-0231-koto-ui-sdk-transaction-resource-locale-ergonomics.md) | done | KotoUI SDK transaction/resource/locale ergonomics |
| [KOTO-0232](issues/main/KOTO-0232-koto-ui-compile-time-packet-capacity-helpers.md) | done | KotoUI compile-time packet capacity helpers |
| [KOTO-0233](issues/main/KOTO-0233-koto-buffer-capacity-at-ui-builder-call-sites.md) | done | Koto buffer capacity at KotoUI builder call sites |
| [KOTO-0234](issues/main/KOTO-0234-gallery-named-failure-codes-and-storage.md) | done | Gallery failure codes and storage offsets as named declarations |
| [KOTO-0235](issues/main/KOTO-0235-koto-fixed-buffer-fields-structured-storage.md) | done | Koto fixed buffer fields in static records (structured storage) |
| [KOTO-0236](issues/main/KOTO-0236-asset-len-and-sdk-storage-capacity-helpers.md) | done | Compile-time asset sizes and SDK storage capacity helpers |
| [KOTO-0237](issues/main/KOTO-0237-compile-time-text-asset-shape-helpers.md) | done | Compile-time text asset shape helpers |
| [KOTO-0238](issues/main/KOTO-0238-text-asset-line-maxima-and-const-additive-sizing.md) | done | Text asset line maxima and compile-time additive sizing |

### KotoConfig

The [KotoConfig roadmap](planning/KOTOCONFIG_ROADMAP.md) separates the native
settings UI and shared persistence service from Shell consumers. Wi-Fi remains
optional future work behind explicit firmware capabilities.

| Issue                                                                  | Status | Title                                      |
| :--------------------------------------------------------------------- | :----- | :----------------------------------------- |
| [KOTO-0223](issues/main/KOTO-0223-kotoconfig-foundation-language.md)       | done   | KotoConfig foundation and language settings |
| [KOTO-0224](issues/main/KOTO-0224-kotoconfig-wifi-extension-design.md)     | done   | KotoConfig optional Wi-Fi extension design |
| [KOTO-0227](issues/main/KOTO-0227-pico-w-switchable-audio-wifi-sram-residency.md) | done | Pico W switchable Audio/Wi-Fi SRAM residency |
| [KOTO-0239](issues/main/KOTO-0239-bounded-network-service-embassy-net.md) | done | Bounded NetworkService and Embassy network integration |
| [KOTO-0240](issues/main/KOTO-0240-wifi-secret-credential-provider.md) | in-progress | Wi-Fi secret credential provider and persistence |
| [KOTO-0241](issues/main/KOTO-0241-kotoconfig-wifi-page.md) | done | KotoConfig Wi-Fi page and bilingual interaction |
| [KOTO-0242](issues/main/KOTO-0242-kotosim-fake-network-service.md) | done | KotoSim deterministic fake NetworkService |
| [KOTO-0243](issues/main/KOTO-0243-picocalc-wifi-config-validation.md) | in-progress | PicoCalc Wi-Fi configuration validation |
| [KOTO-0244](issues/main/KOTO-0244-bounded-sntp-time-service.md) | done | Bounded SNTP time service and Shell clock |
| [KOTO-0251](issues/main/KOTO-0251-pico-w-product-network-wiring.md) | in-progress | Pico W product NetworkService wiring on the residency arena |
| [KOTO-0252](issues/main/KOTO-0252-rp2040-app-session-stack-peak-reduction.md) | todo | RP2040 app-session main-stack peak attribution and reduction |

### Application Networking

Applications use an OS-brokered, capability-gated service rather than owning
raw sockets or network-stack handles.

| Issue                                                                  | Status | Title                                      |
| :--------------------------------------------------------------------- | :----- | :----------------------------------------- |
| [KOTO-0245](issues/main/KOTO-0245-bounded-app-fetch-service.md) | done | Bounded app Fetch service and SDK |
| [KOTO-0246](issues/main/KOTO-0246-bounded-json-data-decoder.md) | in progress | Bounded JSON data decoder for Koto apps |
| [KOTO-0247](issues/main/KOTO-0247-weather-internet-data-reference-app.md) | todo | Weather Internet-data reference app |
| [KOTO-0248](issues/main/KOTO-0248-app-network-credential-vault.md) | todo | Application network credential vault and grants |
| [KOTO-0249](issues/main/KOTO-0249-bounded-app-mqtt-service.md) | in progress | Bounded application MQTT subscribe service and SDK |
| [KOTO-0250](issues/main/KOTO-0250-iot-dashboard-mqtt-reference-app.md) | todo | IoT Dashboard MQTT reference app |

### Koto Language And SDK Evolution

Language additions stay compile-time-only unless a separate issue explicitly
changes the VM ABI. SDK migrations preserve the existing flat constants while
official Apps adopt clearer namespaced domains.

| Issue                                                                  | Status | Title                                      |
| :--------------------------------------------------------------------- | :----- | :----------------------------------------- |
| [KOTO-0225](issues/main/KOTO-0225-koto-language-enum-sdk-domains.md)       | done   | Koto language enums and SDK constant domains |
| [KOTO-0228](issues/main/KOTO-0228-koto-static-records-inline-methods.md)   | done        | Koto static records and inline methods       |
| [KOTO-0237](issues/main/KOTO-0237-compile-time-text-asset-shape-helpers.md) | done | Compile-time text asset shape helpers |
| [KOTO-0238](issues/main/KOTO-0238-text-asset-line-maxima-and-const-additive-sizing.md) | done | Text asset line maxima and compile-time additive sizing |

### Cleanup And Planning

| Issue                                                          | Status | Title                                   |
| :------------------------------------------------------------- | :----- | :-------------------------------------- |
| [KOTO-0059](issues/main/KOTO-0059-roadmap-state-cleanup.md)         | done   | Roadmap state cleanup                   |
| [KOTO-0060](issues/main/KOTO-0060-sim-runtime-profile-cleanup.md)   | done   | KotoSim runtime profile cleanup         |
| [KOTO-0061](issues/main/KOTO-0061-kotosim-module-split.md)          | done   | KotoSim module split                    |
| [KOTO-0062](issues/main/KOTO-0062-manifest-json-parser-cleanup.md)  | done   | Manifest JSON parser cleanup            |
| [KOTO-0063](issues/main/KOTO-0063-doc-implementation-status-map.md) | done   | Documentation implementation status map |
| [KOTO-0070](issues/main/KOTO-0070-memo-basic-multiline-input.md)    | done   | Memo basic multiline input              |
| [KOTO-0071](issues/main/KOTO-0071-ime-usability-hardening.md)       | done   | IME usability hardening                 |
| [KOTO-0072](issues/main/KOTO-0072-memo-editor-usable-ui.md)         | done   | Memo editor usable UI                   |
| [KOTO-0073](issues/main/KOTO-0073-ime-toggle-status-bar.md)         | done   | IME toggle and status bar baseline      |
| [KOTO-0092](issues/main/KOTO-0092-compiler-local-slot-reuse.md)     | done   | Compiler per-scope local slot reuse     |
| [KOTO-0096](issues/main/KOTO-0096-manifest-driven-heap-profile.md)  | done   | Manifest-driven per-app heap profile    |
| [KOTO-0101](issues/main/KOTO-0101-runtime-budget-diagnostics.md)    | done   | Runtime budget diagnostics              |
| [KOTO-0102](issues/main/KOTO-0102-koto-blocks-local-reduction.md)   | done   | KotoBlocks local slot reduction         |
| [KOTO-0104](issues/main/KOTO-0104-inline-boundary-slot-reuse.md)    | done   | Inline-boundary local slot reuse        |
| [KOTO-0105](issues/main/KOTO-0105-fix-existing-ime-test-failure.md) | done   | Fix existing IME test failure           |

### Memo UX

| Issue                                                             | Status      | Title                                |
| :---------------------------------------------------------------- | :---------- | :----------------------------------- |
| [KOTO-0074](issues/main/KOTO-0074-memo-visual-shell.md)                | done        | Memo visual shell                    |
| [KOTO-0075](issues/main/KOTO-0075-memo-font-metrics-caret-accuracy.md) | done        | Memo font metrics and caret accuracy |
| [KOTO-0076](issues/main/KOTO-0076-memo-scrollbar.md)                   | done        | Memo scrollbar                       |
| [KOTO-0077](issues/main/KOTO-0077-ime-candidate-popup-ux.md)           | done        | IME candidate popup UX               |
| [KOTO-0078](issues/main/KOTO-0078-ime-candidate-list-navigation.md)    | in-progress | IME candidate list navigation        |
| [KOTO-0079](issues/main/KOTO-0079-memo-command-bar-actions.md)         | done        | Memo command bar actions             |
| [KOTO-0080](issues/main/KOTO-0080-memo-open-save-dialog-baseline.md)   | done        | Memo open/save dialog baseline       |
| [KOTO-0093](issues/main/KOTO-0093-memo-save-as-filename-prompt.md)     | done        | Memo save / save-as filename prompt  |
| [KOTO-0088](issues/main/KOTO-0088-memo-light-theme-colored-text.md)    | done        | Memo light theme and colored text    |
| [KOTO-0089](issues/main/KOTO-0089-larger-skk-dictionary.md)            | todo        | Larger SKK dictionary for evaluation |
| [KOTO-0090](issues/main/KOTO-0090-memo-line-wrap-and-scroll.md)        | done        | Memo line wrap and horizontal scroll |
| [KOTO-0099](issues/main/KOTO-0099-memo-ime-candidate-overlap-avoidance.md) | done  | Memo IME candidate overlap avoidance |
| [KOTO-0100](issues/main/KOTO-0100-romaji-kana-missing-youon.md)        | done        | Romaji-to-kana missing youon rows    |
| [KOTO-0106](issues/main/KOTO-0106-inline-memo-ime-candidate-display.md) | done       | Inline memo IME candidate display    |
| [KOTO-0107](issues/main/KOTO-0107-inline-ime-composition-insertion-layout.md) | done | Inline IME composition insertion layout |
| [KOTO-0108](issues/main/KOTO-0108-memo-input-after-opening-long-document.md) | done | Memo input after opening long document |
| [KOTO-0109](issues/main/KOTO-0109-romaji-kana-punctuation-long-vowel.md) | done | Romaji/kana punctuation and long-vowel |
| [KOTO-0110](issues/main/KOTO-0110-memo-backspace-delete-key-repeat.md) | done | Memo Backspace/Delete key repeat |
| [KOTO-0111](issues/main/KOTO-0111-memo-new-document.md) | done | Memo new document |
| [KOTO-0112](issues/main/KOTO-0112-memo-save-confirmation-flow.md) | done | Memo save confirmation flow |

### Shell UX

| Issue                                                                  | Status | Title                                        |
| :--------------------------------------------------------------------- | :----- | :------------------------------------------- |
| [KOTO-0091](issues/main/KOTO-0091-package-description-category-metadata.md) | done   | Package description and category metadata    |
| [KOTO-0081](issues/main/KOTO-0081-shell-visual-home.md)                     | done   | Shell visual home                            |
| [KOTO-0082](issues/main/KOTO-0082-shell-icon-grid-pagination.md)            | done   | Shell icon grid and pagination               |
| [KOTO-0083](issues/main/KOTO-0083-shell-selected-app-details-pane.md)       | done   | Shell selected app details pane (toggleable) |
| [KOTO-0084](issues/main/KOTO-0084-shell-system-status-indicators.md)        | done   | Shell system status indicators               |
| [KOTO-0085](issues/main/KOTO-0085-shell-command-bar-actions.md)             | done   | Shell command bar actions                    |
| [KOTO-0086](issues/main/KOTO-0086-shell-favorites-categories-sort.md)       | done   | Shell favorites, categories and sort         |
| [KOTO-0087](issues/main/KOTO-0087-shell-icon-asset-set.md)                  | done   | Shell icon asset set                         |

### Games And Media

| Issue                                                          | Status | Title                                       |
| :------------------------------------------------------------- | :----- | :------------------------------------------ |
| [KOTO-0094](issues/main/KOTO-0094-koto-blocks-game.md)              | done   | KotoBlocks tetromino game + sprite/tile API |
| [KOTO-0095](issues/main/KOTO-0095-app-audio-host-call.md)           | done   | App audio host call (BGM and SFX)           |
| [KOTO-0097](issues/main/KOTO-0097-game2d-abi-design.md)             | done   | Game2D ABI design (tile/sprite boundary)    |
| [KOTO-0098](issues/main/KOTO-0098-kotomml-multitrack.md)            | done   | KotoMML multi-track playback                |
| [KOTO-0103](issues/main/KOTO-0103-koto-blocks-game-feel-effects.md) | done   | KotoBlocks game-feel effects pass           |
| [KOTO-0113](issues/main/KOTO-0113-kotorogue-game.md)               | done   | KotoRogue turn-based roguelike              |
| [KOTO-0116](issues/main/KOTO-0116-package-image-asset-load.md)     | done   | Package image assets (asset_load + .kim)    |
| [KOTO-0145](issues/main/KOTO-0145-add-pcm-playback-path.md)        | todo   | Add PCM playback path for Pico audio backend |
| [KOTO-0146](issues/main/KOTO-0146-pico-audio-cpu1-worker.md)       | in-progress | Pico audio CPU1 worker for stable PCM service |
| [KOTO-0147](issues/main/KOTO-0147-pico-cpu1-render-prep-worker.md) | todo   | Pico CPU1 render-prep worker after audio service |

### Embedded Bring-Up

| Issue                                                       | Status | Title                         |
| :---------------------------------------------------------- | :----- | :---------------------------- |
| [KOTO-0064](issues/main/KOTO-0064-pico-hal-crate-bootstrap.md)   | done   | Pico HAL crate bootstrap      |
| [KOTO-0065](issues/main/KOTO-0065-pico-probe-blink-cdc.md)       | done   | Pico probe: blink and USB CDC |
| [KOTO-0066](issues/main/KOTO-0066-pico-probe-lcd-fill.md)        | done        | Pico probe: LCD fill          |
| [KOTO-0067](issues/main/KOTO-0067-pico-probe-keyboard-i2c.md)    | done        | Pico probe: keyboard I2C      |
| [KOTO-0068](issues/main/KOTO-0068-pico-probe-sd-read.md)         | done        | Pico probe: SD mount and read |
| [KOTO-0069](issues/main/KOTO-0069-pico-probe-psram-roundtrip.md) | done        | Pico probe: PSRAM round-trip  |
| [KOTO-0114](issues/main/KOTO-0114-pico-probe-pwm-audio-output.md) | done        | Pico probe: PWM audio output  |
| [KOTO-0115](issues/main/KOTO-0115-pico-probe-battery-power-status.md) | done        | Pico probe: battery and power |

### Device Firmware

The device roadmap targets KotoSim-equivalent KotoShell behavior using the same
portable shell/runtime code. Order is significant: stabilize the retained-GRAM
dirty renderer, restore the SD catalog with bounded storage, add package
presentation and shell actions/status, then launch apps and pass the parity
gate.

| Order | Issue                                                               | Status      | Outcome                                      |
| :---- | :------------------------------------------------------------------ | :---------- | :------------------------------------------- |
| 1     | [KOTO-0117](issues/main/KOTO-0117-pico-firmware-main-loop.md)            | done        | Portable shell state and physical input loop |
| 2     | [KOTO-0118](issues/main/KOTO-0118-pico-sd-package-list.md)                | done        | Initial physical SD package discovery        |
| 3     | [KOTO-0119](issues/main/KOTO-0119-pico-shell-raster-backend.md)          | in-progress | Shared shell painter on the ILI9488           |
| 4     | [KOTO-0120](issues/main/KOTO-0120-pico-shell-dirty-rect-performance.md)   | todo        | Retained-GRAM responsive dirty rendering     |
| 5     | [KOTO-0121](issues/main/KOTO-0121-pico-shell-sd-catalog-reintegration.md) | todo        | Stack-safe product SD catalog                |
| 6     | [KOTO-0122](issues/main/KOTO-0122-pico-shell-package-metadata-icons.md)   | todo        | Simulator-equivalent metadata and icons      |
| 7     | [KOTO-0123](issues/main/KOTO-0123-pico-shell-actions-preferences.md)      | done        | Commands, favorites, sort, and persistence   |
| 8     | [KOTO-0124](issues/main/KOTO-0124-pico-shell-system-status.md)            | done        | Real device status indicators                |
| 9     | [KOTO-0125](issues/main/KOTO-0125-pico-shell-runtime-launch-return.md)    | in-progress | Launch apps and return safely to shell       |
| 10    | [KOTO-0126](issues/main/KOTO-0126-pico-kotoshell-parity-validation.md)    | done        | Physical/KotoSim parity release gate         |
| 11    | [KOTO-0127](issues/main/KOTO-0127-pico-large-bytecode-budget.md)          | todo        | Large app bytecode and heap budget           |
| 12    | [KOTO-0128](issues/main/KOTO-0128-pico-app-frame-flicker.md)              | done        | Flicker-free app runtime rendering           |
| 13    | [KOTO-0129](issues/main/KOTO-0129-pico-device-draw-pixels.md)            | done        | Device `draw_pixels_rgb565` blit path        |
| 14    | [KOTO-0130](issues/main/KOTO-0130-pico-device-asset-load.md)             | done        | Device `asset_load` package image support    |
| 15    | [KOTO-0131](issues/main/KOTO-0131-pico-app-render-perf.md)               | in-progress | App render performance and per-frame metrics |
| 16    | [KOTO-0134](issues/main/KOTO-0134-embassy-main-future-size.md)           | in-progress | Investigate ~128 KiB embassy main future     |
| 17    | [KOTO-0135](issues/main/KOTO-0135-stateful-game2d-host-renderer.md)      | done        | Stateful Game2D tile host renderer           |
| 18    | [KOTO-0136](issues/main/KOTO-0136-game2d-static-layer.md)                | done        | Game2D retained static/background layer      |
| 19    | [KOTO-0137](issues/main/KOTO-0137-koto-blocks-shape-table.md)           | done        | KotoBlocks shape table and bytecode locality |
| 20    | [KOTO-0138](issues/main/KOTO-0138-koto-blocks-loopless-blit-piece.md)   | done        | KotoBlocks loopless blit_piece cell table    |
| 21    | [KOTO-0139](issues/main/KOTO-0139-bytecode-const-data-heap-image.md)    | todo        | Bytecode const data / initial heap image     |
| 22    | [KOTO-0140](issues/main/KOTO-0140-retained-sprite-stamp-layer.md)       | todo        | Retained sprite/stamp layer (cell stamps)    |
| 23    | [KOTO-0143](issues/main/KOTO-0143-full-repaint-instrumentation-coalescing.md) | todo  | Full-repaint reason codes + tile coalescing  |
| 24    | [KOTO-0141](issues/main/KOTO-0141-retained-text-layer.md)               | todo        | Retained text layer                          |
| 25    | [KOTO-0142](issues/main/KOTO-0142-compiler-inline-diagnostics.md)       | todo        | Compiler inline diagnostics (short-term)     |
| 26    | [KOTO-0144](issues/main/KOTO-0144-game2d-api-cleanup-retained-docs.md)  | todo        | Game2D API cleanup + retained author docs    |
| 27    | [KOTO-0132](issues/main/KOTO-0132-profile-and-optimize-pio-psram-read-bandwidth.md) | in-progress | Profile/optimize PIO PSRAM read bandwidth |

The post-KOTO-0138 retained-rendering roadmap (orders 21–26) is designed in
[GAME2D_RETAINED_RENDER_ARCHITECTURE.md](architecture/GAME2D_RETAINED_RENDER_ARCHITECTURE.md), the
source of truth for these issues. Rows are listed in execution order; **KOTO-0139,
KOTO-0140, and KOTO-0143 are required before KotoBlocks is comfortably playable.**

## Completed Baseline

These issues describe the current KotoSim and repository-harness baseline. They
are kept for traceability, but they are not the active roadmap.

### Foundation

| Issue                                                    | Status | Title                                           |
| :------------------------------------------------------- | :----- | :---------------------------------------------- |
| [KOTO-0001](issues/main/KOTO-0001-rust-workspace.md)          | done   | Rust workspace bootstrap                        |
| [KOTO-0002](issues/main/KOTO-0002-package-scan.md)            | done   | KotoSim package manifest scan                   |
| [KOTO-0003](issues/main/KOTO-0003-issue-management.md)        | done   | Repository-local issue management               |
| [KOTO-0004](issues/main/KOTO-0004-kotofs-sandbox.md)          | done   | KotoFS sandbox path resolver                    |
| [KOTO-0005](issues/main/KOTO-0005-render-surface-model.md)    | done   | Core render surface and dirty rectangle harness |
| [KOTO-0006](issues/main/KOTO-0006-host-input-script.md)       | done   | Scripted host input harness                     |
| [KOTO-0007](issues/main/KOTO-0007-package-format-spec.md)     | done   | `.kpa` package format specification             |
| [KOTO-0008](issues/main/KOTO-0008-rp2040-bringup-plan.md)     | done   | RP2040 bring-up plan and HAL backend decision   |
| [KOTO-0009](issues/main/KOTO-0009-host-fs-hal.md)             | done   | Host filesystem HAL adapter                     |
| [KOTO-0010](issues/main/KOTO-0010-shell-rendering.md)         | done   | KotoShell render model integration              |
| [KOTO-0011](issues/main/KOTO-0011-kpa-manifest-validation.md) | done   | KPA manifest validation in core                 |
| [KOTO-0012](issues/main/KOTO-0012-ci-local-checks.md)         | done   | Local CI command and check script               |

### Text And Japanese Input

| Issue                                                 | Status | Title                         |
| :---------------------------------------------------- | :----- | :---------------------------- |
| [KOTO-0013](issues/main/KOTO-0013-font-glyph-model.md)     | done   | Bitmap font glyph model       |
| [KOTO-0014](issues/main/KOTO-0014-text-layout.md)          | done   | Text grid and IME line layout |
| [KOTO-0015](issues/main/KOTO-0015-ime-romaji-kana.md)      | done   | Romaji-to-kana input core     |
| [KOTO-0016](issues/main/KOTO-0016-ime-sticky-shift.md)     | done   | Sticky Shift state machine    |
| [KOTO-0017](issues/main/KOTO-0017-skk-dictionary-index.md) | done   | SKK dictionary index strategy |
| [KOTO-0038](issues/main/KOTO-0038-memo-ime-integration.md) | done   | Memo IME integration          |

### Runtime And Packages

| Issue                                                          | Status | Title                              |
| :------------------------------------------------------------- | :----- | :--------------------------------- |
| [KOTO-0018](issues/main/KOTO-0018-runtime-selection-spike.md)       | done   | Runtime VM selection spike         |
| [KOTO-0019](issues/main/KOTO-0019-runtime-host-api.md)              | done   | Runtime host API boundary          |
| [KOTO-0020](issues/main/KOTO-0020-kpa-packer-prototype.md)          | done   | KPA packer prototype               |
| [KOTO-0021](issues/main/KOTO-0021-asset-sequential-read-harness.md) | done   | Sequential asset read harness      |
| [KOTO-0033](issues/main/KOTO-0033-runtime-bytecode-verifier.md)     | done   | KBC1 bytecode verifier             |
| [KOTO-0034](issues/main/KOTO-0034-runtime-vm-core.md)               | done   | Cooperative bytecode VM core       |
| [KOTO-0036](issues/main/KOTO-0036-runtime-text-file-host-calls.md)  | done   | Runtime text and file host calls   |
| [KOTO-0042](issues/main/KOTO-0042-runtime-input-ime-host-calls.md)  | done   | Runtime input and IME host calls   |
| [KOTO-0045](issues/main/KOTO-0045-high-level-app-language-spike.md) | done   | High-level app language spike      |
| [KOTO-0046](issues/main/KOTO-0046-koto-language-compiler-mvp.md)    | done   | Koto language compiler MVP         |
| [KOTO-0047](issues/main/KOTO-0047-bytecode-sdk-prelude.md)          | done   | Bytecode SDK prelude               |
| [KOTO-0048](issues/main/KOTO-0048-app-build-package-loop.md)        | done   | App build and package loop         |
| [KOTO-0051](issues/main/KOTO-0051-bytecode-debug-data.md)           | done   | Bytecode debug data and source map |

### Device And Media

| Issue                                                       | Status | Title                                  |
| :---------------------------------------------------------- | :----- | :------------------------------------- |
| [KOTO-0022](issues/main/KOTO-0022-psram-block-api.md)            | done   | PSRAM block API prototype              |
| [KOTO-0023](issues/main/KOTO-0023-audio-mixer-core.md)           | done   | Software PCM mixer core                |
| [KOTO-0024](issues/main/KOTO-0024-power-status-model.md)         | done   | Power status model and shell indicator |
| [KOTO-0025](issues/main/KOTO-0025-keyboard-matrix-validation.md) | done   | Keyboard matrix validation plan        |
| [KOTO-0026](issues/main/KOTO-0026-lcd-init-profiles.md)          | done   | LCD controller init profiles           |

### App Engines

| Issue                                                       | Status | Title                                  |
| :---------------------------------------------------------- | :----- | :------------------------------------- |
| [KOTO-0027](issues/main/KOTO-0027-kotodos-mode.md)               | done   | KotoDOS 320x200 mode model             |
| [KOTO-0028](issues/main/KOTO-0028-kotovn-script-spike.md)        | done   | KotoVN script and image pipeline spike |
| [KOTO-0029](issues/main/KOTO-0029-kotomml-format.md)             | done   | KotoMML format and playback model      |
| [KOTO-0030](issues/main/KOTO-0030-picomings-sprite-model.md)     | done   | PicoMings scanline sprite model        |
| [KOTO-0037](issues/main/KOTO-0037-memo-editor-core.md)           | done   | Memo editor core                       |
| [KOTO-0039](issues/main/KOTO-0039-memo-kpa-fixture.md)           | done   | Memo KPA fixture                       |
| [KOTO-0041](issues/main/KOTO-0041-bytecode-memo-app.md)          | done   | Bytecode memo app                      |
| [KOTO-0052](issues/main/KOTO-0052-sdk-samples.md)                | done   | SDK samples                            |
| [KOTO-0054](issues/main/KOTO-0054-asset-development-pipeline.md) | done   | Asset development pipeline             |

### Simulator And Tooling

| Issue                                                             | Status | Title                                    |
| :---------------------------------------------------------------- | :----- | :--------------------------------------- |
| [KOTO-0031](issues/main/KOTO-0031-sim-framebuffer-image.md)            | done   | KotoSim software framebuffer + image out |
| [KOTO-0032](issues/main/KOTO-0032-sim-live-window.md)                  | done   | KotoSim live interactive window          |
| [KOTO-0035](issues/main/KOTO-0035-sim-runtime-launch.md)               | done   | KotoSim runtime launch path              |
| [KOTO-0040](issues/main/KOTO-0040-memo-sim-validation.md)              | done   | Memo simulator validation                |
| [KOTO-0043](issues/main/KOTO-0043-sim-interactive-bytecode-session.md) | done   | KotoSim interactive bytecode session     |
| [KOTO-0044](issues/main/KOTO-0044-bytecode-assembler.md)               | done   | Bytecode assembler and IR target         |
| [KOTO-0049](issues/main/KOTO-0049-sim-app-dev-experience.md)           | done   | KotoSim app development experience       |
| [KOTO-0050](issues/main/KOTO-0050-runtime-inspector.md)                | done   | Runtime inspector                        |
| [KOTO-0053](issues/main/KOTO-0053-app-scaffold-tool.md)                | done   | App scaffold tool                        |
| [KOTO-0055](issues/main/KOTO-0055-save-data-management.md)             | done   | Save data management                     |
| [KOTO-0056](issues/main/KOTO-0056-app-failure-recovery.md)             | done   | App failure recovery screen              |
| [KOTO-0057](issues/main/KOTO-0057-shell-app-details.md)                | done   | Shell app details view                   |
| [KOTO-0058](issues/main/KOTO-0058-golden-frame-validation.md)          | done   | Golden frame validation                  |

### Embedded Performance And Audio (KOTO-0133+)

These issues landed after the index above stopped being maintained; rows are
generated from each issue file's own status line.

| Issue | Status | Title |
| :---- | :----- | :---- |
| [KOTO-0133](issues/main/KOTO-0133-connect-audio-hostcall.md) | done | Connect audio hostcalls to PicoCalc audio backend |
| [KOTO-0148](issues/main/KOTO-0148-pico-sram-map-audit.md) | in-progress | Pico SRAM map audit and Embassy pool diet |
| [KOTO-0150](issues/main/KOTO-0150-psram-qpi-pio-read-bandwidth-optimization.md) | todo | PSRAM QPI/PIO read bandwidth optimization |
| [KOTO-0151](issues/main/KOTO-0151-psram-qpi-clkdiv4-roundtrip-delay-sweep.md) | done | PSRAM QPI clkdiv=4 roundtrip delay sweep |
| [KOTO-0152](issues/main/KOTO-0152-reimplement-psram-qpi-backend.md) | done | Reimplement PicoCalc PSRAM QPI backend as a mode-owned backend |
| [KOTO-0154](issues/main/KOTO-0154-compiler-peephole.md) | done | Conservative compiler peephole pass |
| [KOTO-0159](issues/main/KOTO-0159-kotoblocks-dirty-rect-coalescing.md) | in-progress | KotoBlocks event-frame dirty-rect coalescing |
| [KOTO-0162](issues/main/KOTO-0162-legacy-audio-deprecation.md) | in-progress | Deprecate legacy KotoOS audio; KotoAudio sequence runtime is primary |
| [KOTO-0163](issues/main/KOTO-0163-primary-audio-validation.md) | in-progress | Validate the primary KotoAudio path on KotoBlocks (SIM + Pico) and minimal tuning |
| [KOTO-0164](issues/main/KOTO-0164-primary-audio-cue-model.md) | done | Primary audio asset / cue model — route table cleanup (SIM + Pico) |
| [KOTO-0165](issues/main/KOTO-0165-port-koto-audio-runtime-to-pico.md) | done | Port the koto-audio runtime to Pico and delete the legacy audio engine |
| [KOTO-0166](issues/main/KOTO-0166-sldpcm4-drum-tables.md) | done | SLDPCM4 built-in drum tables (flash diet) |
| [KOTO-0167](issues/main/KOTO-0167-kotorogue-device-freeze-at-game-over.md) | todo | KotoRogue freezes at game over on hardware |
| [KOTO-0168](issues/main/KOTO-0168-kotorun-render-performance.md) | in-progress | KotoRun steady-play render performance |
| [KOTO-0169](issues/main/KOTO-0169-vm-frame-cost-attribution.md) | done | Steady-frame vm_us attribution and reduction |
| [KOTO-0170](issues/main/KOTO-0170-ram-interpreter-default-on.md) | done | Make ram_interpreter the default firmware build |
| [KOTO-0171](issues/main/KOTO-0171-psram-fast-code-window-default-on.md) | done | Make psram_fast_code_window the default firmware build |
| [KOTO-0172](issues/main/KOTO-0172-main-task-stack-frame-reduction.md) | done | Shrink the embassy main-task poll stack frame |
| [KOTO-0173](issues/main/KOTO-0173-two-tile-code-window-cache.md) | done | Re-land the two-tile CodeWindow cache (KOTO-0134 retry) |
| [KOTO-0174](issues/main/KOTO-0174-present-path-cost-reduction.md) | done | Present-path (raster/transfer) cost attribution and reduction |
| [KOTO-0175](issues/main/KOTO-0175-kotorun-commandcountshift-full-repaints.md) | done | KotoRun's recurring CommandCountShift full repaints (fps-8 hitches) |
| [KOTO-0176](issues/main/KOTO-0176-release-profile-lto.md) | in-progress | Deterministic release code layout (workspace LTO + codegen-units=1) |
