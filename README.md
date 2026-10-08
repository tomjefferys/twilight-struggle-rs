# Twilight Struggle (CLI)

A Rust terminal implementation of the *Twilight Struggle* board game, with
the rules complete: the map, the whole turn structure, all the operations,
the Space Race, scoring, and every card event (all 109 of them; the China
Card is an ops card and has none). You can play a full game from the
opening placement to final scoring, in an interactive keyboard-driven map
or a line-based REPL, against one of three AI opponents or a friend at the
same keyboard.

This is an unofficial personal/hobby project. See `CLAUDE.md` for the
architecture and `CARDS.md` for which card events are implemented.

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
cargo run                          # title screen and menu, then the interactive map
cargo run -- --play ussr           # new game vs the AI, you as the USSR (us | ussr | random)
cargo run -- --new --ai ussr       # new game, AI plays the USSR
cargo run -- --repl                # the line-based REPL instead
cargo run -- worldmap              # one-shot: the whole-world map
cargo run -- region europe         # one-shot: zoom into a region
cargo test                         # all tests (unit + snapshot)
```

The menu offers one player (choose USA, USSR or random), two players
(hotseat, with a handover screen between turns so the other hand stays
hidden), the command console, and resume. In the REPL, type `help` for the
full command list. `--seed <n>` (or the REPL's `seed <n>`) fixes the dice
seed for reproducible rolls; `--fast` skips the AI's step-by-step modals.

## Status

Implemented:

- The 84-country map with adjacency, regions, stability and battlegrounds,
  shown as a dashboard, per-region, whole-world and per-country views.
- Influence placement, realignment and coups, with control, presence and
  DEFCON region limits, Military Operations, and DEFCON degradation.
- The full turn structure: new-game setup (shuffled and dealt Early War
  deck, opening placement), headline phase, ten turns of action rounds, the
  end-of-turn Military Operations penalty and held-scoring-card loss, the
  Mid/Late War additions, and final scoring. Games can end by VP, DEFCON,
  Europe Control, Wargames, Cuban Missile Crisis, or after turn 10.
- The Space Race and its perks.
- Every card event (the China Card has none): scoring, fixed-effect, choice, war,
  turn-long and lasting cards, plus dual use (an opponent's card played for
  ops also fires its event).
- Three AI opponents: `SearchAi` (the default, Monte Carlo search),
  `HeuristicAi`, and `RandomAi` (`--ai-kind search|heuristic|random`).
- A game history log, viewable in-session or exported to a text file, and a
  library of named test states (`--state <file>/<name>`).

Not implemented: saving and resuming an in-progress game.

## License

MIT — see [LICENSE](LICENSE).
