# Full-color image streaming gallery

This sample cycles through four 320x320 RGB source PNGs from `art/`:

- `landscape.png`: Mount Fuji at sunrise;
- `city.png`: a rainy neon city;
- `autumn.png`: an autumn temple garden;
- `space.png`: a spacecraft and ringed planet.

During the normal app build, `harness/build_apps.py` converts each image to a
normal 320x320 KIM1 RGB565 image. Pixels stay in their original row-major order.
There is no tiling, clustering, resampling, palette reduction, or spatial
resolution reduction; RGB888 to RGB565 is the only quantization.

The gallery holds for two seconds, then alternates between an ordered dissolve
and a left-to-right wipe-out. It streams 8 scanlines (5,120 bytes) from the
package and commits each 320x8 band to LCD GRAM. Keeping each SD read short
allows the background audio ring to be serviced between image bands. The band
buffer plus a 512-byte black transition block stays well below the device's
16 KiB app-heap limit.

`audio/music.wav` is converted to an infinitely looping 16 kHz mono SLD4 KACL
and streamed from the package as background music. Recreate the committed asset
with:

```powershell
cargo run --manifest-path src/koto-audio/Cargo.toml `
  -p koto-audio-tools --bin koto-audio-convert `
  --features experimental-sldpcm4 -- `
  --codec experimental-sldpcm4 --sldpcm4-fallback force --loop `
  apps/samples/full_color_tile_image/audio/music.wav `
  apps/samples/full_color_tile_image/audio/music_sld4.kacl
```

## Reusable conversion tool

Convert any non-interlaced 8-bit RGB/RGBA PNG of exactly 320x320 pixels:

```powershell
python tools/png_full_color_image.py art.png --output output/image.kim
```

The tool has no third-party runtime dependencies and writes the exact same
320x320 image asset as the app build pipeline.

Rebuild only this gallery with:

```powershell
python harness/build_apps.py --app dev.koto.samples.full-color-tile-image
```
