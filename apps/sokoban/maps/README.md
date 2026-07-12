# Sokoban stage maps

Each `*.txt` file is one stage, authored as a 10x8 ASCII tilemap (10 columns,
8 rows). Stages load in filename order, so the numeric prefix sets the level
number (`01-*.txt` is stage 1).

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

These files are the single source of truth. The build step embeds them into
`src/main.koto` (the generated `stage_data()` block) and decodes them at
runtime, so **adding a stage is just dropping a new `NN-name.txt` here and
running**:

```
python harness/build_apps.py
```
