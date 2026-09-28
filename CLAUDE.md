# Twilight Struggle

A Rust CLI implementation of the board game *Twilight Struggle*. Currently
focused on the data model and terminal display; full game rules (cards,
DEFCON, coups, etc.) haven't been built yet.

## Architecture

- **`WorldMap`** (`src/map.rs`) — immutable game data: every country's
  name, stability, battleground flag, region, and adjacency, loaded from
  `data/standard_map.json`. Neighbours are stored as name strings in the
  JSON and resolved to `CountryId` indices at load time.
- **`Board`** (`src/board.rs`) — the mutable game state: US/USSR
  influence per country. Kept separate from `WorldMap` so it stays cheap
  to clone (undo, AI lookahead) without duplicating immutable data.
- **`MapLayout`** (`src/layout.rs`) — everything needed to *display* the
  map, loaded from `data/standard_layout.json`: each country's cell in
  its region-view grid, its short code, and its position on the single
  world map (`world_cell`), plus the two superpower boxes. Validated at
  load time (unique codes, no cell collisions, no out-of-bounds
  `world_cell`, etc.) with a fail-loud `LayoutError`.
- **`Scenario`** (`src/scenario.rs`) / **`GameStatus`** (`src/status.rs`)
  — a named starting state (currently just `data/demo_state.json`): a
  `Board` snapshot plus turn/DEFCON/VP/space-race/China-card status.
- **`ops`** (`src/ops.rs`) — the first real game operation: spending
  operation points to place influence. `InfluencePlacement` keeps *two*
  cloned `Board`s (cheap, per `Board`'s own design note): a fixed `base`
  snapshot taken when the action starts, and a running `board` that
  accumulates each placement. Presence/adjacency legality is checked
  against `base` only — a point placed earlier in the same action can't
  unlock a new country that had no influence in or next to it when the
  action began — while cost (1 op, or 2 in a country the opponent
  controls) tracks the running `board`, since control genuinely can flip
  mid-action. Nothing reaches a real `Board` until `commit`; `undo_last`
  refunds the exact cost a point was charged, not a
  flat 1. This is the model for every future ops-spending operation
  (coups, realignment, card play).
- **`render`** (`src/render/`) — every view is a pure function
  `(WorldMap, MapLayout, Board, ...) -> Canvas`; nothing in this module
  touches the terminal directly, which keeps every view snapshot-testable.
  A `Canvas` only turns into a `String` via `.render(ColorMode)`.
  - `world.rs` — the six-region dashboard (`map`/`world` command).
  - `region.rs` — one region zoomed in, with real adjacency connectors
    (`region <name>` / `1`-`6`).
  - `worldmap.rs` — the whole world as one to-scale map with real
    landmass shading and *Twilight Struggle* region colour tinting, no
    connectors (`worldmap`/`wm`). See **`data/world_background.md`**
    before touching this — it documents the whole pipeline (source data,
    projection warp, region-colour tinting) and the reasoning behind a
    long list of hand-tuned fixes, so a future adjustment doesn't have to
    start from scratch.
  - `country.rs` — a single country's detail view.

  `render_region` and `render_world_map` both take an optional
  `&InfluencePlacement` (`region.rs`/`worldmap.rs`): `None` reproduces
  the plain view byte-for-byte (every `render_world`/`render_country`
  call, which don't take one, and every static call with no session
  open); `Some` swaps in the placement's speculative board so numbers and
  control glyphs update live, adds a `+N` badge (region) or turns a
  chip's flag into `+` (world map), dims an illegal target (region only),
  and appends a balance-line footer.
- **`main.rs`** — a REPL (`cargo run`) plus one-shot mode
  (`cargo run -- <command>`). Type `help` inside the REPL for the command
  list, including `ops`/`place`/`undo`/`confirm`/`cancel` for staging and
  committing an influence placement; `set`/`add`/`remove`/`load` are
  refused while a session is open, since they'd shift the board a
  placement was judged legal against.
- **`interactive.rs`** — the terminal-driving code for `worldmap`/`wm`
  when run interactively (a real TTY, not one-shot mode): raw mode, the
  alternate screen, and the arrow/Enter/Esc event loop. On the world map
  it moves a `Region` selection (`Region::step`, in `country.rs`); Enter
  zooms into `render_region`, where arrow keys move a country selection
  on that region's display grid instead (`MapLayout::step_country`, in
  `layout.rs` — a nearest-in-that-direction search over `Cell` positions,
  not a hand-written table, since the grids are sparse with interior
  holes). Each region remembers its last-selected country across visits.
  When an `InfluencePlacement` session is open, `+`/`=` places one point
  in the selected country (region screen only), `u` undoes the last
  point, `c` confirms it into the board, and `X` discards it; `Esc`/`q`
  leave a still-open session untouched rather than clearing it, so it can
  be resumed from the REPL or by reopening the map. `run`'s return value
  (`Outcome`) tells the REPL which of those happened.
  The only place in the crate that touches the terminal directly —
  everything it draws still comes from `render::render_world_map`/
  `render_region`, which stay pure `Canvas` producers. Uses `crossterm`,
  the one non-serde dependency.

## Data files (`data/`)

All game/display data is JSON or plain text, embedded into the binary via
`include_str!` rather than read at runtime — the data is part of the
build, not a runtime dependency.

- `standard_map.json` — the 86 countries and their adjacency (game rules
  data).
- `standard_layout.json` — display-only layout data (region-view cells,
  codes, world map positions, superpower boxes).
- `world_background.txt` — pre-rasterized ASCII/Unicode landmass art for
  the world map, generated once from real coastline data (not checked-in
  tooling — see `world_background.md`).
- `world_background.md` — **read this before changing the world map's
  background art, country positions, or region colour tinting.**
- `demo_state.json` — the bundled demo scenario's starting `Board` +
  `GameStatus`.
- `backup/` — earlier full snapshots of the world map, kept in case a
  future change needs to compare against or revert to an earlier version.

## Conventions

- Fail loud at load time: JSON loaders validate everything they can (see
  `MapError`/`LayoutError`/`ScenarioError`) rather than silently
  tolerating bad data.
- No `thiserror`/`anyhow` — plain enums implementing `Display` + `Error`.
- Views are pure functions returning a `Canvas`; never print/touch the
  terminal from inside `render/`.
- Snapshot tests live in `tests/snapshots/`; regenerate by running the
  relevant `cargo run -- --color never <command>` and diffing/replacing
  the snapshot file, not by hand-editing it.

## Useful commands

```
cargo run                          # interactive REPL
cargo run -- worldmap              # one-shot: the whole-world map
cargo run -- region europe         # one-shot: zoom into a region
cargo test                         # all tests (unit + snapshot)
cargo clippy --all-targets         # lint; expected clean
```
