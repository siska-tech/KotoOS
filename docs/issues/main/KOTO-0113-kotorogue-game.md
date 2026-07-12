# KOTO-0113: KotoRogue turn-based roguelike

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-PM-1, FR-PM-2

## Goal

Ship a compact turn-based roguelike, "KotoRogue" (`dev.koto.games.kotorogue`):
a procedurally generated dungeon crawl with bump combat, item pickups, fog of
war, depth progression, and BGM/SFX — built entirely on the KotoSDK prelude and
running through the same KotoSim runtime path the device will use.

## Acceptance Criteria

- [x] KotoRogue compiles, verifies, and runs in KotoSim within the user-slot
  (40/45), heap (≈0.8 KB/16 KB), and frame-fuel (≈40 K/60 K) budgets.
- [x] Each level is procedurally generated: rooms, connecting corridors, stairs,
  scattered gold/potions, and depth-scaled monsters seeded by an in-heap LCG.
- [x] Turn-based loop: arrow-key movement, bump-to-attack combat, monster chase
  AI that activates in lit areas, `Q` to quaff a healing potion.
- [x] Classic Rogue room-lighting fog of war: the player's current room and the
  eight adjacent corridor cells are lit; explored cells stay dim; unseen cells
  are black.
- [x] Depth progression by descending `>` stairs (max HP grows and HP refills
  each descent), a win at `GOAL_DEPTH`, death when HP reaches 0, and restart.
- [x] Title, playing, win, and death screens with a HUD (depth, HP bar, gold,
  potions, kills, message line).
- [x] BGM plus per-action SFX (step, hit, kill, gold, potion, stairs, hurt,
  over, win) as package KotoMML assets.
- [x] Registered in `apps/apps.json` with manifest, icon, audio assets, and play
  scenarios; `harness/build_apps.py --check` is clean and the golden-frame
  fixture is updated for the new package (shell package count 13 → 15).

## Notes

- All game state lives in heap buffers, not VM locals, so `main` stays well under
  the 45 user-slot budget while pure-ish helpers do the per-turn work: `map[300]`
  (one byte per cell: bits 0-2 type, bit 3 explored, bit 4 lit), `rooms[24]`
  (room rectangles for room-light visibility), `stats[16]` (player position, HP,
  gold, depth, RNG seed, ...), and an `ActorArray` of up to 10 monsters
  (`state`=HP, `frame`=type, `timer`=alive). See [[koto-app-local-slot-budget]].
- Koto buffers are function-scoped and helpers receive them as heap offsets
  (`int`), so helper bodies use `heap_get_u8`/`heap_set_u8(base + i, …)` rather
  than `buf[i]` indexing, which only applies to a `buf`-declared variable.
- The RNG seed is threaded through `stats` so `rng(stats, m)` both advances and
  stores it; each generated level differs while the first run stays deterministic
  for screenshots and scripted scenarios.
- Monsters are drawn and act only while their cell is lit, which keeps the dungeon
  tense and matches the room-lighting visibility model.
- Scaffolding note: `koto-app-scaffold` re-serializes `apps/apps.json` and drops
  the `assets`/`maps` blocks of existing entries; they were restored by hand after
  registering this app.
