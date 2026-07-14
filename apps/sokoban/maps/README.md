# Sokoban stage maps

Each `*.map` file is one stage, authored as a 10x8 ASCII tilemap (10 columns,
8 rows). Stages load in filename order, so the numeric prefix sets the level
number (`01-*.map` is stage 1).

Glyphs:

| Glyph | Meaning            | Cell |
| :---- | :----------------- | :--- |
| `#`   | wall               | 2    |
| `.`   | floor              | 1    |
| `o`   | goal (置き場所)    | 3    |
| `O`   | crate (荷物)       | 4    |
| `*`   | crate on a goal    | 5    |
| `@`   | porter start       | 1    |
| ` `   | void (no tile)     | 0    |

Each map must be exactly 10 columns x 8 rows and contain exactly one `@`.

These files are the single source of truth. The build validates and stores them
as read-only KPA data assets; the app selects a package path, loads it with
`asset_load`, and decodes LF/CRLF-separated rows at runtime. Add the new path to
`load_stage`, update `stage_count`, then run:

```
python harness/build_apps.py
```
