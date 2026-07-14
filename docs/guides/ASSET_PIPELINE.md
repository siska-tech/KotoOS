# Asset Development Pipeline

KOTO-0054 defines the first host-side pattern for turning source-authored media
into package-ready KPA assets. The pipeline is deliberately small and
dependency-free so future KotoVN, KotoDOS, KotoMML, and PicoMings tooling can add
specialized converters without changing the package contract.

## Asset Map

| Source asset | Tool path | Generated asset | KPA package path | Manifest type | Runtime fit |
| :----------- | :-------- | :-------------- | :--------------- | :------------ | :---------- |
| 40x40 ASCII PBM `P1` icon | `harness/asset_pipeline.py convert-icon` | `KICON1` text bitmap | `icons/*.kicon` | `image` | KotoShell launcher icon. |
| M+ BDF bitmap font source | `harness/mplus_to_kfont.py` | `KFNT` fixed-cell bitmap font | `fonts/*.kfont` | `font` | Text renderer, IME, KotoVN text. |
| Existing `.kfont` blob | `harness/asset_pipeline.py font-preview` | UTF-8 text preview report | `previews/*.txt` or harness-only | `data` when packaged | Host validation of metrics and Japanese sample rendering. |
| `.map` text tilemap | `harness/build_apps.py` validation | Packaged source bytes | `maps/*.map` | `data` | App-local LF/CRLF map loaded with `asset_load`; never embedded in bytecode. |
| Native KotoMML source | `harness/build_apps.py` | unchanged KMML on SD; runtime `KAQ1` image in PSRAM | `audio/*.kmml` | `audio` | SIM reads the mounted file; Pico compiles SD text into the owned Native KotoAudio path. |
| Native KotoMML phrase | `koto-mml bake` | PCM16 mono `KACL` clip | `audio/*.kacl` | `audio` | Pre-rendered KotoAudio clip playback. |
| WAV clip | `koto-audio-convert --codec pcm16\|experimental-sldpcm4` | PCM16 or SLD4 `KACL` clip | `audio/*.kacl` | `audio` | Short clips use bounded owned storage; larger clips stream incrementally from the KPA. |
| KotoVN image source | future image codec converter | RLE/indexed image payload | `assets/*.rle` | `image` or image-specific subtype | Sequential scene image streaming. |
| `.kspr` ASCII sprite sheet | `harness/build_apps.py` (`images` block) | `KIM1` RGB565 tile strip | `sprites/*.kim` | `image` | App `asset_load` + `draw_pixels` tile blits. |
| PNG pixel art (any paint tool) | `koto-img png2kspr` | `.kspr` ASCII source | (source, not packaged) | — | Import path into the `.kspr` loop above. |
| `.kspr` / `.kim` sprite sheet | `koto-img kspr2png` / `kim2png` | PNG preview | harness-only | — | Review, README shots, re-editing in a paint tool. |
| PNG launcher icon (40×40) | `koto-img png2kicon` | `KICON1` mask source | `apps/<dir>/icon.kicon` | `image` | Import path for the app icon mask (KOTO-0196). |
| `.kicon` launcher icon | `koto-img kicon2png` | PNG preview | harness-only | — | Review / re-editing the icon mask in a paint tool. |
| PicoMings sprite sheet source | future sprite packer | tile/sprite banks plus index | `assets/sprites/*` | `image`/`data` | Scanline sprite composition. |
| Koto bytecode | `koto-compiler` or `kbc-asm` | `KBC1` bytecode | `bytecode/*.kbc` | `bytecode` | Runtime entry or support modules. |

The generated package tree mirrors the final KPA namespace:

```text
bytecode/main.kbc
icons/pipeline.kicon
fonts/mplus10.kfont
previews/pipeline_icon.pbm
```

The source manifest keeps assets in desired read order. The packer must preserve
that order in its layout report, and sequential assets must have monotonic
payload offsets.

## Prototype Commands

Convert a launcher icon and preview:

```powershell
python harness\asset_pipeline.py convert-icon `
  --src harness\fixtures\asset_pipeline\icon_40.pbm `
  --out harness\fixtures\asset_pipeline\package_assets\icons\pipeline.kicon `
  --preview harness\fixtures\asset_pipeline\package_assets\previews\pipeline_icon.pbm
```

Validate a font and render Japanese sample text:

```powershell
python harness\asset_pipeline.py font-preview `
  --font assets\fonts\mplus10.kfont `
  --sample Koto日本語 `
  --out harness\fixtures\asset_pipeline\font_preview.txt
```

Verify generated asset placement against a package manifest:

```powershell
python harness\asset_pipeline.py verify-layout `
  --manifest harness\fixtures\asset_pipeline\asset_pipeline.kpa.json `
  --layout harness\fixtures\asset_pipeline\asset_pipeline.layout.csv
```

## Sprite Sheets (`.kspr` → `.kim`)

App tile/sprite art is authored as a reviewable ASCII `.kspr` source and compiled
to a binary `KIM1` strip by `harness/build_apps.py`. An app registers it with an
`images` block (mirroring the `maps`/`audio` blocks) in its `apps/<dir>/app.json`
descriptor — app-relative source, package-local output:

```json
"images": [
  { "source": "sprites/tiles.kspr",
    "output": "sprites/kotorogue_tiles.kim" }
]
```

The `.kspr` source has `# comments`, `color <char> <RRGGBB>` palette entries, and
`tile <id> <name>` headers each followed by exactly 16 rows of 16 palette chars.
Tiles stack top-to-bottom into a 16-wide strip. The compiled **`KIM1`** format is:

```text
"KIM1"            4 bytes magic
width             u16 little-endian   (16)
height            u16 little-endian   (16 * tile_count)
pixels            width*height * 2    row-major, little-endian RGB565
```

The committed `.kim` is declared as an `image` asset in the package manifest, so
the app can pull it into its heap with [`asset_load`](../spec/KOTO_SDK.md) and blit each
16×16 tile from byte offset `8 + tile * 512` with `draw_pixels`. `draw_pixels` is
opaque, so entity/item tiles carry their floor background and are only drawn over
lit floor. `build_apps.py --check` fails if a committed `.kim` is stale.

### PNG import/export (`koto-img`, KOTO-0187)

Sprite art does not have to be typed as ASCII: `tools/koto-img` converts
between PNG and the formats above, so sheets can be drawn in Aseprite or any
paint tool and committed as reviewable `.kspr` text.

```powershell
cargo run -p koto-img -- png2kspr art/tiles.png apps/myapp/sprites/tiles.kspr
cargo run -p koto-img -- kspr2png apps/myapp/sprites/tiles.kspr preview.png
cargo run -p koto-img -- kim2png  sdcard_mock/sprites/myapp_tiles.kim preview.png
```

- **`png2kspr`** slices a PNG (width and height must be non-zero multiples of
  16, all pixels fully opaque) into 16×16 tiles, left-to-right then
  top-to-bottom, and emits deterministic `.kspr` text. Colors are kept exact —
  an image with more distinct colors than the palette alphabet (65 characters)
  is rejected, never quantized.
- **`kspr2png` / `kim2png`** render what the device blits: palette colors are
  truncated to RGB565 exactly as `build_apps.py` packs them, then expanded to
  8-bit by bit replication. Because that expansion is idempotent, a
  `kspr2png` → edit → `png2kspr` cycle recompiles to a **byte-identical
  `.kim`** for untouched pixels (palette characters and tile names are
  regenerated, so the `.kspr` text itself may differ). The crate's tests pin
  this round-trip and byte-parity with `build_apps.py` on the KotoRogue sheet.


## Harness Coverage

`harness/check_project.py` checks the prototype fixtures without rewriting them:

- the PBM source regenerates the committed `KICON1` package asset exactly;
- the PBM preview regenerates exactly;
- `mplus10.kfont` has valid `KFNT` metrics and can render `Koto日本語`;
- `asset_pipeline.layout.csv` matches manifest asset order;
- every generated fixture asset exists and has the layout-reported size.

This keeps KOTO-0054 focused on the pattern rather than finalizing every media
codec. Future converters should add source fixtures, generated package assets,
preview output, and layout checks in the same shape.
