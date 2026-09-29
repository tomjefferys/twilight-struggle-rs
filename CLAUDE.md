# Twilight Struggle

A Rust CLI implementation of the board game *Twilight Struggle*. Currently
focused on the data model and terminal display, a handful of the
ops-spending actions (influence placement, realignment, coups), and
enforced alternating turns; full game rules (cards, DEFCON, Military
Operations, scoring, etc.) haven't been built yet.

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
  world map (`world_cell`), plus the two superpower boxes. Also carries
  each region's `guests` (`MapLayout::guests`): countries native to a
  *different* region, or a superpower, placed on this region's grid too —
  at their own cell, distinct from their native one, since cells are only
  unique per region and a foreign country's own cell routinely collides
  with a native one here. A `Guest`'s `GuestEntity` is `Country(CountryId)`
  or `Superpower(Superpower)`; one entity can have more than one guest
  cell in the same region when no single cell is grid-adjacent to every
  native country it borders there (USSR appears three times in Europe's
  grid: once diagonally between Finland and Poland, covering both, and
  once by Romania). A guest's cell is hand-picked, not just any free
  adjacent one: it's placed on whichever side of its bordering native
  country matches that adjacency's real compass direction (compared via
  both countries' `world_cell`), so an arrow key toward a guest points
  the way you'd actually expect — the reason this matters enough to
  choose deliberately, not arbitrarily, is that the *wrong* side makes
  the two feel like they're on opposite sides of the map, and stepping
  back and forth between them (an ordinary, expected way to use a
  bidirectional connector) reads as broken rather than as "go back the
  way you came." The one exception is Europe's Algeria and USA guests:
  France's and Canada's other five and two real neighbours already fill
  every cell around them but one, so each is forced onto that single
  leftover cell regardless of which way it actually points from there.
  `MapLayout::step_country` treats a region's country
  guests as extra candidates alongside its natives — never a guest
  superpower, which is never selectable — so its return value may belong
  to a different region than the one passed in; the caller is what acts
  on that (see `interactive.rs` below). Validated at load time (unique
  codes, no cell collisions between natives or guests, no out-of-bounds
  `world_cell`, every guest naming exactly one of a country or a
  superpower and grid-adjacent to a native country there that actually
  borders it, etc.) with a fail-loud `LayoutError`. `undrawn_links()`
  reports whatever a region's grid still couldn't place a connector for —
  an in-region pair too far apart, or a cross-region/superpower link with
  no guest chip for it — as `UndrawnLink { region, from, to: LinkTarget }`;
  empty for the standard layout, and pinned there by
  `tests/layout.rs::standard_layout_has_no_undrawn_links`.
- **`Scenario`** (`src/scenario.rs`) / **`GameStatus`** (`src/status.rs`)
  — a named starting state (currently just `data/demo_state.json`): a
  `Board` snapshot plus turn/active-side/DEFCON/VP/space-race/China-card
  status. `GameStatus` itself is plain data with no rules attached — the
  `Game` type below is the only thing that ever mutates `active`, `turn`,
  or `action_round`.
- **`ops`** (`src/ops/`) — the game's ops-spending operations. Three kinds
  so far, sharing the `Operation` enum (`src/ops/mod.rs`) as the seam the
  rest of the crate reads through:
  - `influence.rs` — `InfluencePlacement`, spending operation points to
    place influence. Keeps *two* cloned `Board`s (cheap, per `Board`'s
    own design note): a fixed `base` snapshot taken when the action
    starts, and a running `board` that accumulates each placement.
    Presence/adjacency legality is checked against `base` only — a point
    placed earlier in the same action can't unlock a new country that
    had no influence in or next to it when the action began — while cost
    (1 op, or 2 in a country the opponent controls) tracks the running
    `board`, since control genuinely can flip mid-action. Nothing reaches
    a real `Board` until `commit`; `undo_last` refunds the exact cost a
    point was charged, not a flat 1.
  - `realign.rs` — `Realignment` (rule 6.2): spending 1 op per die-roll
    contest that can reduce the opponent's influence in a country — or,
    on a bad roll, the acting side's own. Deliberately **not** modelled
    like placement: each `roll` resolves immediately onto the caller's
    real `Board` and can't be undone, the way a physical die roll can't,
    so there's no speculative board and no `commit`. It keeps a `base`
    snapshot too, but only to report *what changed* (`delta`, read per
    side since one country can take both a win and a loss across two
    rolls in the same action) — legality and the modifier/odds maths
    always read the live board instead. No presence is required to
    target a country (unlike placement) and there's no DEFCON
    restriction (rule 6.1.3 is out of scope). `modifiers`/`odds` are free
    functions of `(map, board, id, side)`, not methods, so a preview
    (region footer, country detail, REPL) works with no session open.
    Dice come from `src/dice.rs`, a small seedable splitmix64-based `Dice`
    (no `rand` dependency) — seeded from entropy in the REPL, from a
    fixed default (overridable with `--seed`) in one-shot mode, so the
    snapshot-regeneration workflow below stays reproducible.
  - `coup.rs` — `Coup` (rule 6.3): realignment-shaped (immediate,
    irreversible, no `commit`), but a whole action is a *single* attempt
    that spends every op on the card at once, rather than one roll per
    op. The target number is the country's stability, doubled; roll 1d6,
    add the card's ops, and a modified roll that's strictly *greater
    than* the target number succeeds by the margin — removing that much
    opposing influence and, if there isn't enough opposing influence to
    absorb the whole margin, adding the rest as friendly influence (rule
    6.3.3). No presence is required (rule 6.3.1) and, like realignment's
    6.1.3 carve-out, DEFCON degradation and Military Operations (rule
    6.3.4) are out of scope for now. `coup_resolve`/`coup_odds` are free
    functions mirroring realignment's `resolve`/`odds`, just over one die
    (sixths) instead of two (36ths). `Coup::attempt` shares `Realignment`'s
    `&mut Dice` and its `roll <country>` REPL command / `r` interactive
    key — `roll` resolves whichever of the two kinds of session is open.
    A second `attempt` on an already-resolved `Coup` is refused, the same
    way rolling with no ops left is refused for a realignment.

  A card-play operation would add a fourth `Operation` variant; whether
  it stages like placement or resolves immediately like realignment/coup
  is a per-operation call, not a rule of the enum.

  `InfluencePlacement`, `Realignment`, `Coup`, and `Operation` all derive
  `Clone` — cheap, per `Board`'s own design note — for the same reason:
  `Game` (below) needs to be clonable for AI lookahead.
- **`Game`** (`src/game.rs`) — turns. Owns the status, the board, and
  whichever `Operation` is open, so "an operation belongs to the active
  side, and closing it passes the turn" lives in one place rather than
  being duplicated between the REPL and interactive mode. `begin(kind)` is
  the entire enforcement mechanism: it always opens with `active()` and a
  full turn's ops (`OPS_PER_ACTION_ROUND`, 4), so there's no argument
  through which a caller could name the wrong side, and it's refused while
  an operation is already open. One turn spends exactly one operation —
  `confirm`/`cancel` are the only two ways to close one, and both hand the
  turn to the other side via the private `advance` (USSR → USA; USA → USSR
  plus `action_round += 1`, rolling `turn` over once `action_round`
  exceeds `action_rounds_per_turn`). `pass` is the same handover with no
  operation opened. Ending a turn with ops unspent is allowed and simply
  forfeits them, the same as `InfluencePlacement` never requiring every op
  to be spent. `Game` takes no `WorldMap` or `Dice` of its own — both are
  passed per call, matching how the ops modules already split `Board` out
  — and is cheap to `Clone`, which is the whole point: nothing in this
  module touches a terminal, so it's the complete surface a future AI
  opponent drives, and lookahead means cloning a `Game` to try a line of
  play without touching the real one. `Game::lookahead` clones status,
  board, and open operation like `Clone` does, but starts the copy's log
  empty, since the log is the one field whose size isn't bounded and a
  search cloning many nodes shouldn't drag a growing history through every
  branch it never plays out.
- **`log`** (`src/log.rs`) — `GameLog`, the game's append-only history, a
  `Vec<LogEntry>` built up entirely inside `Game` — the one place every
  mutation already funnels through — so the REPL and `interactive.rs` are
  both covered without either having to remember to log anything. Every
  operation closes with an `Event::Closed` entry (pushed from
  `confirm`/`cancel`, stamped with the turn/AR/side *before* `advance`
  runs) naming the operation kind and its final ops balance — a
  realignment's or coup's dice already have their own entries by then
  (`Event::Realign`/`Event::Coup`, pushed the instant `Game::roll`
  resolves, since a roll is irreversible the moment it happens), and an
  influence placement's points get the same treatment: `Event::Placed`,
  pushed by `log_close` immediately before `Closed`, the placement
  analogue of a resolved roll (omitted entirely if nothing was placed, the
  same way zero rolls simply mean zero `Event::Realign` entries) — so
  `log`/`export` show `confirm`/`cancel` as its own line for every
  operation kind, never merged onto the line reporting what happened.
  `Event::Pass` covers `Game::pass`. `Game::board_mut` is the one mutator
  `Game` can't observe by itself (`set`/`add`/`remove` in `main.rs` bypass
  the operation system entirely), so `Game::record_edit` and
  `Game::record_note` exist for a caller to report an edit or an
  annotation (e.g. `load demo` resetting the board) explicitly. This
  module holds no formatting and touches no map — the same split `ops/`
  keeps between rules and display.
- **`render`** (`src/render/`) — every view is a pure function
  `(WorldMap, MapLayout, Board, ...) -> Canvas`; nothing in this module
  touches the terminal directly, which keeps every view snapshot-testable.
  A `Canvas` only turns into a `String` via `.render(ColorMode)`.
  - `world.rs` — the six-region dashboard (`map`/`world` command).
  - `region.rs` — one region zoomed in, with real adjacency connectors
    (`region <name>` / `1`-`6`). Every adjacency that leaves the region —
    to another region's country, or to a superpower — is drawn too, as a
    guest chip (`MapLayout::guests`): a foreign country tinted with
    `region_color` for *its own* region (never thick-bordered, since a
    guest can't be the selection, and never dimmed for operation
    illegality, since it isn't a target from this screen), or a
    superpower tinted like its own box and never selectable. This is what
    makes the whole map walkable by arrow keys alone without ever
    returning to the world map — see `interactive.rs` below. Only what a
    guest chip couldn't cover (none, on the standard layout) is footnoted
    below the grid, the same as an in-region adjacency the grid geometry
    couldn't draw a connector for.
  - `worldmap.rs` — the whole world as one to-scale map with real
    landmass shading and *Twilight Struggle* region colour tinting, no
    connectors (`worldmap`/`wm`). See **`data/world_background.md`**
    before touching this — it documents the whole pipeline (source data,
    projection warp, region-colour tinting) and the reasoning behind a
    long list of hand-tuned fixes, so a future adjustment doesn't have to
    start from scratch.
  - `country.rs` — a single country's detail view, drawn as a titled box
    with up to three panels: Country (influence, control, sub-regions),
    Neighbours (one line each, plus a `+1 realign` marker on whichever
    ones are supplying a realignment's `adjacent_controlled` modifier),
    and — only while an operation is open — Operation, showing exactly
    what the region footer's own breakdown shows (a placement's cost and
    pending count; a realignment's per-side modifiers and odds; a coup's
    target number and odds), titled with `operation_header` and its first
    row always `operation_touched_line`. `render_country` takes a
    `MapLayout` (for that touched-line and `short_name`) and a `ViewMode`
    (`Static`/`Interactive`) that gates a key-hint row below the box —
    `Static` for the REPL's one-shot prints, `Interactive` for the
    screen `interactive.rs` opens. Like `region.rs`/`worldmap.rs`, it
    opens by substituting `op.board()` when the operation has a
    speculative one, so a placement's pending influence shows here too.
    Every panel but Neighbours' own influence readouts is built as a
    `Vec<(String, Style)>` before drawing, mirroring `region.rs`'s
    `build_footer_lines`; two small `Canvas` primitives exist for this
    layout specifically — `draw_divider` for the `├─ Title ─┤` rows
    between panels, and the free function `put_border_title` for the
    outer box's own `┌─ * Poland ─── Europe · stability 3 ─┐` top border.
  - `log.rs` — turns a `GameLog` into text: `log_entry_line` is the
    canonical rendering of one entry, fixed-column and tagged so a roll's
    numbers can't be mistaken for each other (`d6:` only ever the actual
    die; `mod:`/`ops:`/`target:`/`sum:` label everything else by where it
    came from — a bare `4+4=8` doesn't say which 4 was rolled).
    `log_text` joins a header plus every `log_entry_line` into the exact
    string `export` writes to a file; `render_log` draws the same lines
    into a `Canvas`, coloured by side (`Color::Us`/`Color::Ussr`, `Muted`
    for a debug edit or note) — `render_log(..).render(ColorMode::Never)`
    is byte-identical to `log_text`, so the on-screen `log` command and
    the exported file are guaranteed to be one format, not two.

  `render_region`, `render_world_map`, and `render_country` all take an
  optional `&Operation` (`region.rs`/`worldmap.rs`/`country.rs`). `None`
  reproduces the plain view byte-for-byte (every `render_world` call, and
  every static call with no session open); `Some` swaps in the
  operation's speculative board where it has one (`Operation::board()` —
  placement does, realignment and coup don't, since their rolls are
  already on the real board), adds a `+N`/`-N` net badge (region) or
  turns a chip's flag into `+`/`!`/`#` for placement/realignment/coup
  (world map), dims an illegal target (region only), and shows the
  operation's balance — `operation_balance_line` as one footer line
  (region, world map, and `main.rs`'s dashboard banner), or split into
  `operation_header` (a panel title) and `operation_touched_line` (its
  first row) on the country view, which has room to give the balance a
  panel of its own instead of gluing it into one line. A realignment
  additionally adds, once a country is also selected (region) or is the
  one being viewed (country), that country's modifier breakdown for both
  sides and its odds (`modifier_line`/`odds_line`, shared by both views
  and the REPL's `roll` output); a coup adds its target number and
  success odds instead (`coup_target_line`/`coup_odds_line`) — no
  per-side modifiers to itemise. All three views build their footer or
  panels as `Vec<(String, Style)>` before drawing, so the row count and
  the draw loop can't drift apart the way hand-maintained parallel
  tallies could.
- **`main.rs`** — a REPL (`cargo run`) plus one-shot mode
  (`cargo run -- <command>`). Type `help` inside the REPL for the command
  list. `Session` holds a `Game` (`src/game.rs`), so turns are enforced
  everywhere the REPL touches the board: `influence`/`realign`/`coup` take
  no arguments any more — the side and the 4 ops are always `Game::begin`'s,
  never typed in — and `place`/`undo`/`confirm`/`cancel`/`roll` all read
  and write through `Session.game` rather than a bare `Board` and
  `Option<Operation>`. `influence`/`place`/`undo`/`confirm`/`cancel` stage and
  commit an influence placement; `realign`/`roll`/`confirm`/`cancel` run a
  realignment and `coup`/`roll`/`confirm`/`cancel` a coup, where `roll`
  resolves either kind immediately (a coup's `roll` spends every op on its
  one attempt) without ending the turn, and `undo` always refuses for
  either. `confirm` and `cancel` are the only two ways to close an
  operation, and both hand the turn to the other side (reported in the
  next prompt, which names the active side and the AR counter); `pass`
  does the same handover with no operation open, refused if one is.
  `status` reports the turn/AR/active side and the open operation's
  balance. Either way, `set`/`add`/`remove`/`load` are refused while a
  session (`Game::operation()`) is open, since they'd shift the board an
  operation was judged legal against; when they do run, they call
  `Game::record_edit`/`record_note` themselves, since `Game::board_mut`
  can't observe what a caller does with it. `log [n]` (or `history`)
  prints the game's history so far, or just the last `n` entries, via
  `render::render_log`; `export <path>` writes it to a file as plain text
  via `render::log_text` — the same bytes `log` shows, minus colour — and
  is the crate's first use of `std::fs`, since every other data file is
  `include_str!`-embedded rather than read or written at runtime.
  `--seed <n>` (or the REPL's `seed <n>`) controls `Session.dice`, which
  stays outside `Game` so it can be seeded independently and so
  `Game::roll` stays deterministic given its inputs.
- **`interactive.rs`** — the terminal-driving code for `worldmap`/`wm`
  when run interactively (a real TTY, not one-shot mode): raw mode, the
  alternate screen, and the arrow/Enter/Esc event loop over three screens
  (`Screen::World`/`Region`/`Country`). On the world map it moves a
  `Region` selection (`Region::step`, in `country.rs`); Enter zooms into
  `render_region`, where arrow keys move a country selection on that
  region's display grid instead (`MapLayout::step_country`, in
  `layout.rs` — a nearest-in-that-direction search over `Cell` positions,
  not a hand-written table, since the grids are sparse with interior
  holes). Since `step_country` treats a region's guest chips as
  candidates too, a step can land on a country native to a *different*
  region; `step_or_jump` is what follows it there, on both the region and
  country screens' arrow keys alike — it rewrites `Screen::Region`'s (or
  `Screen::Country`'s) `region` field to match the moment that happens,
  keeping the invariant that the selection is always native to whichever
  region is on screen, and records the region just left in
  `last_selected` first (the same bookkeeping `Esc` already does), so
  coming back to it later re-selects the country left behind rather than
  resetting to the region's top-left-most one. This is what makes the
  whole map walkable by arrow keys alone, never forced back out to the
  world map, and is also why interactive mode's region screen has no
  separate "leave the region" key of its own beyond `Esc`. From there,
  Enter *or* `r` opens that country's own detail screen (`render_country`,
  `ViewMode::Interactive`) — the region screen no longer rolls or attempts
  a coup directly; it only gets you to the country screen, where the full
  modifier/odds calculation is on screen above the key that resolves it.
  Each region remembers its last-selected country across visits, and the
  country screen carries the same selection back to `Region` on `Esc`.
  `run` takes `&mut Game` (not a bare `Board`/`Option<Operation>`), so
  every key handler goes through it and turns stay enforced here too: `c`
  confirms/closes the open operation via `Game::confirm` and `X`
  cancels/closes it via `Game::cancel` — either way handing the turn to
  the other side, which is also why interactive mode itself still can't
  *open* an operation (that stays a REPL-only `influence`/`realign`/`coup`, so
  there's no side to infer from a keypress alone). An `InfluencePlacement`
  binds `+`/`=` to place one point and `u` to undo the last one on
  *both* the region and country screens — placement is undoable, so it
  never needs the country screen's confirmation step, and stays a fast,
  stay-on-one-screen action from the region grid too. A `Realignment` or
  `Coup`, by contrast, only binds `r` — to roll (or attempt the coup) on
  the selected country — on the country screen, and `u` always refuses
  there: a resolved roll or attempt can't be taken back. `Esc`/`q` leave a
  still-open session untouched rather than clearing it, so it can be
  resumed from the REPL or by reopening the map. `run`'s return value
  (`Outcome`) tells the REPL which of those happened. A roll's outcome
  is shown in the same message row as a refusal, but — unlike a refusal
  — survives the next keypress, since it's the one thing a player most
  wants to keep reading.
  The only place in the crate that touches the terminal directly —
  everything it draws still comes from `render::render_world_map`/
  `render_region`/`render_country`, which stay pure `Canvas` producers.
  Uses `crossterm`, the one non-serde dependency.

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
