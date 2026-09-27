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
- **`main.rs`** — a REPL (`cargo run`) plus one-shot mode
  (`cargo run -- <command>`). Type `help` inside the REPL for the command
  list.
- **`interactive.rs`** — the terminal-driving code for `worldmap`/`wm`
  when run interactively (a real TTY, not one-shot mode): raw mode, the
  alternate screen, and the arrow/Enter/Esc event loop that moves a
  `Region` selection (`Region::step`, in `country.rs`) around the world
  map and zooms into `render_region`. The only place in the crate that
  touches the terminal directly — everything it draws still comes from
  `render::render_world_map`/`render_region`, which stay pure `Canvas`
  producers. Uses `crossterm`, the one non-serde dependency.

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
