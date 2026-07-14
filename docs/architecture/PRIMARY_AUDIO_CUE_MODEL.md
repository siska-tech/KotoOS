# Runtime audio asset model

Application KMML is a real SD-card asset. KotoBlocks and every other app use the
same Native KotoAudio path; there is no app-name route table.

## SD to playback

1. An app authors `apps/<app>/audio/*.kmml` and declares the copied
   `audio/*.kmml` path in its manifest.
2. `harness/build_apps.py` copies that text to the SD-card tree unchanged. It
   does not generate Rust cue tables.
3. `play_bgm_asset` or `play_sfx_asset` queues the declared path.
4. After the VM frame, Pico reads the text from SD and the bounded no-heap
   Native KMML compiler creates a pointer-free `KAQ1` runtime image.
5. Pico stores that image after the app bytecode in PSRAM and caches its
   path/address metadata for the app session. A repeated cue is read from PSRAM;
   the SD file is not parsed again.
6. The CPU1 worker copies the image into an owned fixed-capacity KotoAudio player.
   No reference points into PSRAM or a temporary buffer (RP2040 PSRAM is not
   memory-mapped).

KotoSim follows the same source semantics: it reads the mounted SD asset and
compiles it into the same owned KotoAudio cue representation. The simulator does
not consult firmware-generated tables.

| Hostcall | Runtime role |
| --- | --- |
| `play_bgm_asset(path)` | Replace/start the owned BGM player |
| `play_sfx_asset(path)` | Start a bounded owned SFX player |
| `stop_bgm()` | Stop only BGM; active SFX continue |

## Bounds and failures

- Native KMML supports at most `MAX_SEQUENCE_VOICES` tracks.
- Device BGM tracks hold at most 272 events each; SFX tracks hold at most 32.
- The source file is capped at 4096 bytes and one app session caches 16 cues.
- Instruments must be valid Native KotoAudio builtin IDs. Drum aliases such as
  `!bd`, `!sd`, `!hh`, and `!oh` compile to those same builtin drum IDs.
- A malformed/missing asset, exhausted PSRAM, or busy CPU1 staging slot is a
  safe audio drop with a device diagnostic; it never selects another engine.
- Firmware without usable PSRAM cannot play package KMML assets. Numeric
  host-owned `play_sfx(id)` / `play_bgm(id)` cues remain available.

The KotoAudio test suite scans all 48 shipped KMML files and asserts that they
fit the device bounds. See [KOTOMML_FORMAT.md](../spec/KOTOMML_FORMAT.md) for the
authoring syntax.
