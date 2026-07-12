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
| KotoMML source | future `kotomml` compiler | compact score/event stream | `audio/*.kmml` or `audio/*.kaud` | `audio` | Audio mixer playback. |
| KotoVN image source | future image codec converter | RLE/indexed image payload | `assets/*.rle` | `image` or image-specific subtype | Sequential scene image streaming. |
| `.kspr` ASCII sprite sheet | `harness/build_apps.py` (`images` block) | `KIM1` RGB565 tile strip | `sprites/*.kim` | `image` | App `asset_load` + `draw_pixels` tile blits. |
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
`images` block (mirroring the `maps`/`assets` blocks) in `apps/apps.json`:

```json
"images": [
  { "source": "apps/kotorogue/sprites/tiles.kspr",
    "output": "sdcard_mock/sprites/kotorogue_tiles.kim" }
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
