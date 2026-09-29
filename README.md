# Twilight Struggle (CLI)

A Rust terminal implementation of the *Twilight Struggle* board game: a
data model, map rendering (dashboard, region, world-map, and per-country
views), a REPL and interactive keyboard-driven mode, and a growing subset
of the ops-spending rules (influence placement, realignment, coups) with
enforced alternating turns and a game history log.

This is an unofficial personal/hobby project and a work in progress —
full game rules (cards, DEFCON, Military Operations, scoring, and more)
haven't been built yet. See `CLAUDE.md` for the current architecture and
what's implemented.

## Disclaimer

*Twilight Struggle* is a board game designed by Ananda Gupta and Jason
Matthews, published by **GMT Games**. This project is an unofficial,
non-commercial fan implementation created for personal and educational
purposes. It is not affiliated with, endorsed by, or sponsored by GMT
Games, and no claim is made to any of GMT Games' trademarks, artwork, or
copyrighted materials — this repository contains only original code and
data (country names, adjacency, and stability values are historical/
geographic facts, not GMT's creative content) written to replicate the
published game's rules. If you enjoy this, please support the designers
and publisher by buying the original game.

## Requirements

- [Rust](https://www.rust-lang.org/tools/install) (stable toolchain)

## Getting started

```
cargo run                          # interactive REPL
cargo run -- worldmap              # one-shot: the whole-world map
cargo run -- region europe         # one-shot: zoom into a region
cargo test                         # all tests (unit + snapshot)
```

Inside the REPL, type `help` for the full command list, or `worldmap`
(`wm`) to open the interactive, arrow-key-driven map. `--seed <n>` (or
the REPL's `seed <n>`) fixes the dice seed for reproducible rolls.

## Status

Currently working:

- The full 86-country map and its adjacency, region, and stability data,
  with a dashboard, per-region, whole-world, and per-country view.
- Influence placement, realignment, and coups, each spending a turn's
  operations points, with legality checks (presence/adjacency, control,
  DEFCON where applicable) and correct turn alternation.
- A game history log, viewable in-session or exported to a text file.

Not yet implemented: cards, DEFCON degradation from military operations,
the space race, scoring, and AI opponents.

## License

MIT — see [LICENSE](LICENSE).
