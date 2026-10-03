# Twilight Struggle

A Rust CLI implementation of the board game *Twilight Struggle*. Currently
focused on the data model and terminal display, a handful of the
ops-spending actions (influence placement, realignment, coups), enforced
alternating turns, playing a card from each side's hand for its ops
value, and — the first slices of card *events* — the seven scoring cards
twenty-one fixed-effect cards (influence/VP/DEFCON/China Card only, no choices or
rolls), twenty-one *choice* cards (the card's own side picks the
countries to add/remove influence in, whoever is phasing), and ten
*turn-long* effects ("for the remainder of this turn" — Containment,
Chernobyl, …, held in `GameStatus::effects` until the turn rolls over),
the five *war* cards (a die roll against a target, Military Ops tracked), and eight *lasting* cards (NATO, US/Japan Pact, Formosan Resolution, We Will Bury You, Willy Brandt, Flower Power, Shuttle Diplomacy — held in `GameStatus::lasting`, never cleared by a turn rolling over — plus Solidarity's prerequisite), which can end the game outright (VP reaching ±20, DEFCON
reaching 1, or Europe Scoring's Control tier), and the *Space Race* (`src/space.rs`, below). `CARDS.md` tracks which of
the 110 cards have their event implemented (`tests/cards_progress.rs`
keeps it honest). Every other card's text, DEFCON degradation *other than a battleground
coup's* (rule 6.3.4), Military Operations, and
redealing are still out of scope, as is the headline phase and an
opponent's card's ops-and-event dual use. A first AI opponent plays
uniformly random legal moves.

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
- **`cards`** (`src/cards.rs`) — the static `CardCatalog` (all 110 cards,
  loaded from `data/cards.json`) and each side's `Hands`. Mirrors
  `map.rs`'s own split: `CardCatalog` is immutable, validated-at-load data
  (unique/contiguous ids 1-110, `scoring` iff `ops == 0`, fail-loud
  `CardError`) with a `find` lookup (`CardFound`) like `WorldMap::find`'s,
  except a numeric query matches a card's id (its printed number) before
  falling back to name matching. `Hands` is the mutable per-game state — a
  `[Vec<CardId>; 2]` indexed by `Superpower`, plus a shared `discard` pile
  a played card lands in once its operation (or an event that isn't
  `removed_after_event`) closes, and a second `removed` pile for one that
  is — kept here rather than its own module since it's still a thin
  wrapper with nothing else to say about it; cheap to clone like `Board`,
  for the same reason (`Game` needs to stay clonable for AI lookahead).
  `Hands::remove`/`insert` move a card
  out of (and back into, at the same index) a hand — the mechanics
  `Game::play_card`/`return_card` below drive, not a policy of their own.
  Deliberately never includes `CHINA_CARD` (id 6) in a hand: it changes
  hands outside the normal draw/discard cycle, so it's tracked via
  `GameStatus::china_card`/`china_card_face_up` instead and always shown
  as its own slot. `Card::ops_label` is the one small piece of display
  logic that lives on the data type rather than in `render/`: how a card's
  ops value reads wherever space is tight — its digit, `S` for a scoring
  card, or `★{ops}` for the China Card — shared by the hand strip's
  mini-card boxes and (everywhere else) nothing, since the zoom view's own
  title has room to spell "Ops" out instead. No card *event* behaviour
  lives here either way — this module only ever moves a `CardId` around
  (plus the one exception `removed_after_event` cards need:
  `Hands::remove_from_game`, a second pile alongside `discard` that a
  card lands in once *its event* — never its ops — has it leave the game
  for good); `Game` below (`play_card`'s own doc, and `play_event`'s) is
  what actually reads a card's text, via `events::resolve`, and what its
  ops value means in the first place.
- **`Scenario`** (`src/scenario.rs`) / **`GameStatus`** (`src/status.rs`)
  — a named starting state (currently just `data/demo_state.json`): a
  `Board` snapshot, starting `Hands` (`Scenario::from_json` resolves each
  hand's card names against a `CardCatalog`, rejecting an unknown name, a
  card dealt to more than one hand/discard/removed pile, the China Card
  anywhere but `GameStatus::china_card`, a hand bigger than
  `cards::MAX_HAND_SIZE` (9 — see that constant's own doc), or a status
  `GameStatus::validate` rejects (VP outside ±20, DEFCON outside 1-5, turn
  outside 1-10, or `action_round` outside `1..=action_rounds_per_turn` —
  `StatusError`'s own four variants) — `ScenarioError::{UnknownCard,
  DuplicateCard, ChinaCardInHand, HandTooLarge, InvalidStatus}`), plus
  turn/active-side/DEFCON/VP/space-race/China-card status. `GameStatus`
  itself is plain data with no rules attached — the `Game` type below is
  the only thing that ever mutates `active`, `turn`, or `action_round`
  during real play; debug mode, below, is the one deliberate exception,
  and `GameStatus::validate` is what keeps *that* from landing on a
  nonsensical status, the same check a loaded `Scenario`/test state goes
  through. The on-disk shape
  (`RawScenario`, `pub(crate)`) is split out from name resolution
  (`Scenario::from_raw`) so `states.rs` below can reuse it directly rather
  than re-deriving the same JSON shape; `Scenario::to_raw` is the reverse
  direction, writing a live `Board`/`Hands` back out by name (only a
  country with any influence at all, in `WorldMap`'s own load order) for
  `states.rs`'s `save` to embed. `Scenario::blank` — an empty board,
  default status, empty hands/piles — needs no `CardCatalog`, unlike every
  other constructor here, since there's nothing in it yet to resolve a
  card name against; it's `main.rs`'s debug-mode `blank` command.
- **`states`** (`src/states.rs`) — the named *test*-state library,
  `data/states/<topic>.json`: small hand- or debug-mode-authored
  `Scenario` snapshots (status/board/hands/discard/removed, **no log** —
  see this bullet's own closing paragraph for why that's a deliberate
  difference from an in-progress game's save, not built yet) used to
  exercise a card's event or an ops action without re-creating the board
  by hand every time. A state is referred to as `"<file>/<name>"` (e.g.
  `"scoring/europe-ussr-control-wins"`); `file` names one JSON document
  under the library's directory holding a `{"states": [...]}` array (so
  several related states share one file), `name` one entry in it, found
  via `StateLibrary::load`/`save` (resolving/writing country and card
  *names* through `Scenario::from_raw`/`to_raw`, exactly like
  `Scenario::from_json`'s own lookup) and listed (across every file, read
  fresh every call) via `StateLibrary::list`. **The one file-format
  exception to this crate's `include_str!` convention** (see "Data files"
  below): `StateLibrary::standard()` resolves `data/states/` as a path at
  compile time (`CARGO_MANIFEST_DIR`) but reads its contents at runtime,
  so a state just `save`d from the REPL is loadable by name immediately,
  with no rebuild; `StateLibrary::new` points at any directory instead,
  which is what lets `tests/states.rs` exercise `save`/`load` against a
  throwaway temp directory. `save`'s own JSON writer (`render_state_file`)
  is `serde_json::to_value` plus one small hand-written pretty-printer
  (`render_value`) — needed only because `serde_json`'s own pretty
  formatter puts every array element on its own line, which is right for
  a hand's card list but turns every country's `[us, ussr]` influence
  pair into three lines apiece; `render_value` special-cases exactly that
  shape (a 2-element all-numeric array) back onto one line, matching
  `demo_state.json`'s own hand-authored style. **Convention:** a new card
  implementation adds its own named states here (`data/states/<topic>.json`,
  topic matching the card group, e.g. `scoring.json`) and matching cases
  in `tests/states.rs`, the same way it's expected to add a snapshot
  test — `data/states/scoring.json`'s ten states (one per tier/bonus
  nuance the scoring-event rules actually branch on, plus both VP-cap
  directions) are the first example. A saved test state is a bare
  position to jump *to*, never a sequence of moves to *resume* — that
  second thing (an in-progress game's own save, log included) doesn't
  exist yet, and is deliberately a different type when it does, rather
  than this one growing an optional log.
- **`ongoing`** (`src/ongoing.rs`) — card events that last "for the
  remainder of this turn" (#9 Vietnam Revolts, #25 Containment, #31 Red
  Scare/Purge, #41 Nuclear Subs, #51 Brezhnev Doctrine, #69 Latin American
  Death Squads, #86 North Sea Oil, #93 Iran-Contra, #94 Chernobyl, #109
  Yuri and Samantha). `OngoingEffect` is one card's effect as played (what
  an `EffectResult::ongoing` carries); `TurnEffects` is what's in force —
  plain `Copy` data (one bool/`Option` per card) that is a field of
  `GameStatus` (`#[serde(default, skip_serializing_if = is_empty)]`, so a
  scenario/state file only mentions it when something's active) and is
  reset by `Game::advance` when the turn rolls over. It holds no rules
  beyond small pure queries the ops code asks: `card_ops` (Containment /
  Brezhnev +1, Red Scare −1: the modifiers are *summed*, then clamped to
  1–4, so Containment and Red Scare on the US cancel; also returns each
  card's contribution so a view can itemise it), `coup_roll_mod`,
  `realign_roll_mod`, `placement_banned`, `ops_bonuses` (Vietnam; also the China Card's Asia op — see below),
  `spares_defcon`, `coup_vp`, `extra_rounds`. `Game::begin` reads them
  once and hands each operation what it needs, so views reading an
  `Operation` never need the status: `InfluencePlacement::with_banned_region`
  (Chernobyl; `PlacementError::Banned`, and `is_legal_target` is false so
  the existing dimming works) and `with_bonuses`, `Realignment::with_effects`/`with_bonuses`
  (`Modifiers::iran_contra`, shown in `reasons`), `Coup::with_effects`
  (`CoupResult::modifier`; `coup_resolve_with`/`coup_odds_with`). The
  ops *bonuses* are **dynamic**: `TurnEffects::ops_bonuses(side, card)`
  returns a list of `OpsBonus { area: Area, ops, card }` — Vietnam Revolts
  (USSR, Southeast Asia) and the China Card's own +1 for Asia (#6) — and they
  stack. Each operation keeps a bitmask (`bonus_membership`/`bonus_ops`, one
  bit per bonus) of the bonuses every point/roll so far has stayed inside:
  `ops_total()` is the card's value until something's been spent, then adds
  each surviving bonus (a coup: each one whose area holds the target) — so
  `remaining()` and the AI must ask `InfluencePlacement::can_place` /
  `Realignment::can_afford`, never compare `cost` to `remaining` themselves.
  `Operation::pending_bonus` (a `Vec<OpsBonus>`) is what headers hint at.
  **Lasting effects** (`LastingEffects`/`LastingEffect`, same file) are the game-long sibling of `TurnEffects`: a `GameStatus::lasting` field (`#[serde(default, skip_serializing_if = is_empty)]`) that `Game::advance` never resets. Pure queries only: `protects(map, board, attacker, id)` (NATO for US-controlled Europe, minus France under De Gaulle and West Germany under Willy Brandt; the US/Japan pact for Japan — only the USSR is ever barred; returns the responsible `CardId`), `taiwan_battleground` (Formosan) and `shuttle_applies`. `Coup`/`Realignment` take `with_lasting` and refuse a protected target (`CoupError`/`RealignError::Protected`, and `is_legal_target` is false so the existing dimming works, judged on the live board); `War::with_lasting` does the same for Brush War vs NATO. `events::scoring::resolve` takes the effects too and lists what applied in `ScoringResult::modifiers` (shown in the score modal and log line). `EffectResult::lasting`/`cancels` start or end one (`Ctx::persist`/`cancel`); `Game::finish_effect` applies them. Three triggers live in `Game`, each logged as `Event::Triggered` (`trigger` line): Flower Power (`flower_power_check`, from `discard_played_card` for ops and `finish_war` for events), We Will Bury You (in `advance`, when the US finishes the round it was owed — `skip` is 1 if the US itself played it), and Formosan's cancel on the US playing the China Card. Shuttle Diplomacy isn't discarded when played: the scoring it modifies spends and discards it.
- **`space`** (`src/space.rs`) — the Space Race (rule 6.4), pure rules like
  `ongoing.rs`. `TRACK` holds boxes 1-8 (ops needed, die threshold, VP for
  first/second in, perk); `GameStatus::space_race_us`/`ussr` are the markers (0-8)
  and `space_attempts_us`/`ussr` the attempts made this turn (reset by
  `Game::begin_round`'s turn rollover). **Perks are derived, never stored:**
  `perk_holder` is the side at or past a perk's box while the opponent isn't,
  so reaching the box cancels it with no extra state. `check` is the one
  legality test (`SpaceError`: scoring card, the China Card — which can't be
  spaced — too few ops, no attempts left, track complete; a card with *more*
  ops than the box needs is fine, and the ops compared are the card's
  *effective* ops after Containment etc.); `resolve` is the pure roll.
  `Game::space(dice)` spends the card in play on an attempt: discards it
  directly (not via `discard_played_card`, so Flower Power doesn't see it),
  pays `arrival_vp` on success, logs `Event::Space`, and hands the turn over.
  `Action::Space` is offered when `Game::can_space`. Enforced perks: Animal in
  Space (`attempts_allowed` 2) and Space Station (`GameStatus::rounds_for`
  gives the holder 8 action rounds; `Game::advance`/`begin_round` skip a side
  with no round left, so the holder plays the extra rounds alone — North Sea
  Oil goes through the same path). Man in Earth Orbit (headline first) and
  Eagle/Bear has Landed (discard a held card) are derived and shown but have
  no effect, since there's no headline phase or end-of-turn hand step. Cards
  #18 Captured Nazi Scientist and #80 One Small Step use `Ctx::advance_space`
  and report `EffectResult::space`. Views: `render/space.rs`
  (`render_space_track`, `render_space_result`), a `Space n-m` label and perk
  entries in the status bar, `t` in interactive mode shows the track as an info-only modal (`Modal::SpaceTrack`); `s` (opens `Modal::SpaceConfirm`, `render_space_confirm`: the next box, the roll needed and why a roll is unavailable if it is — Enter rolls only when `Game::can_space`, Esc cancels for free; shown even when the roll is refused), `space`/`spacerace`/
  debug `track us|ussr <n>` in the REPL; test states in `data/states/space.json`.
- **`ops`** (`src/ops/`) — the game's ops-spending operations, plus (as
  `Operation::Event`) a choice card's event in progress. Four kinds
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
    6.1.3 carve-out, Military Operations (rule 6.3.4) are out of scope.
    The *DEFCON degradation* half of 6.3.4 is not done here but in
    `Game::roll` (`Game::coup_aftermath`, see `Game` below), since it
    touches the status, not the board. A coup's die can also carry an
    ongoing event's modifier (`CoupResult::modifier`, Death Squads) and
    its ops a Southeast Asia bonus (`Coup::ops_for`).
    `coup_resolve`/`coup_odds` (and their `_with` variants taking the
    modifier) are free
    functions mirroring realignment's `resolve`/`odds`, just over one die
    (sixths) instead of two (36ths). `Coup::attempt` shares `Realignment`'s
    `&mut Dice` and its `roll <country>` REPL command / `r` interactive
    key — `roll` resolves whichever of the two kinds of session is open.
    A second `attempt` on an already-resolved `Coup` is refused, the same
    way rolling with no ops left is refused for a realignment.

  - `Operation::Event(EventChoice)` (`events/choice.rs`, below) — spends
    no ops (`ops_total`/`remaining` are 0) and stages like placement: a
    speculative board, fully reversible until `Game::confirm`. Its
    `side()` is the *chooser* — the card's own side — which is not
    necessarily `Game::active()`. Everything keyed off `Operation`
    (legal-target dimming, `+N`/`-N` badges, footers, hints) works for it
    unchanged.

  Whether a new operation kind stages like placement or resolves
  immediately like realignment/coup is a per-operation call, not a rule
  of the enum. `Coup` also carries an optional list of banned regions
  (`with_banned_regions`; `CoupError::Banned`) — `Game::begin` bans
  Europe for the USSR once The Reformer (#87) is in `Hands::removed`.

  `InfluencePlacement`, `Realignment`, `Coup`, and `Operation` all derive
  `Clone` — cheap, per `Board`'s own design note — for the same reason:
  `Game` (below) needs to be clonable for AI lookahead.
- **`events`** (`src/events/`) — card *events*, as opposed to ops value
  (which `ops/` above and `Game::begin` already cover). `events::resolve`
  is what `Game::play_event` calls for scoring/fixed-effect cards,
  returning an `EventOutcome` (a choice card instead yields
  `EventOutcome::Pending { card, chooser }` once its session is open);
  `events::is_implemented` is what `Game::play_event` and
  `Game::legal_actions` both check before calling it, or offering
  `Action::Event`, respectively. Three stages so far:
  - `events::choice` (`src/events/choice.rs`) — twenty-one cards where a
    player *picks countries* (CARDS.md lists them; #106 NORAD, an ongoing
    end-of-AR trigger, is not one). `EventChoice` stages the **chooser**'s
    picks against a cloned `Board` (`base` is what eligibility is judged
    against, like placement's presence check; `board` is the speculative
    copy) and is carried by `Operation::Event`. A card's text is one
    function building a `Spec` — modes (`Mode`: a label, `Fixed` changes
    applied on choosing it, and an optional `Rule`) plus `optional` for
    "may" cards (De-Stalinization, Puppet Governments) — and one line in
    `CHOICES`. A `Rule` is a small budgeted vocabulary: add/remove (or
    reallocate), an `Eligible` country filter (`Where` × `Control` ×
    `empty`), a points budget, a per-country cap, a countries budget, and
    a `Chunk` (one point, a fixed amount, *all* of it, or match the
    opponent). `+`/`-` (`Sign`) are the two steps: the direction a rule
    allows is a *forward* move that spends budget; the other, where a
    country already has a staged change, takes the last one back — which
    is what makes `-` remove influence here. `is_complete` — cached by
    `refresh` after every change, since `Game::confirm` has no map —
    follows the rules' "as fully as possible": false while any forward
    step is still legal, except "may" cards (which only need a
    reallocation balanced). A single-mode card with nothing it can do
    (`resolves_immediately`) never opens a session. A multi-mode card's
    mode is chosen with the digits (`mode <n>`, or `mode europe` by label), changeable until the first
    pick. `into_result` produces the same `EffectResult` a fixed-effect
    card does, so `Game::finish_effect` applies and logs both. Clauses
    about other cards (prevents/allows #N, NATO) aren't modelled;
    Special Relationship (#105) only has its "NATO not in effect" branch.
    **Chernobyl (#94)** is the one *designation* (`EventChoice::is_designation`):
    no countries are picked — each of its six modes is a `Region`
    (`Mode::ongoing` carries the `OngoingEffect::Chernobyl { region }` that
    `into_result` hands to `finish_effect`), so it uses the same `1`-`6` /
    `mode <name>` picker as any multi-mode card and is complete as soon as a
    mode is chosen (changeable until `confirm`). While it's open,
    `Operation::is_legal_target` makes every country live until a region is
    chosen and then only that region's, so the existing double-border /
    muting shows the designation; the world map's flag slot stays plain
    (no `+`/`-`) and its legend says "digit: designate that region".
  - `events::war` (`src/events/war.rs`) — the five war cards (#11, #13, #24,
    #36, #102): the first events that **roll a die**, so a `WarSpec` table (beneficiary — fixed or the
    player — target set, success threshold, VP, Military Ops, and whether
    the target itself lowers the roll) plus pure `resolve`/`modifier`/`odds`
    free functions, like `ops::coup`'s. Every war card opens
    `Operation::War(War)` for the player (`EventOutcome::Pending`), which
    `Game::roll` on a target resolves *and closes* (`finish_war`: influence, VP, the beneficiary's
    `military_ops_*` clamped 0–5, `Event::War`, discard/remove, hand the turn
    over — no `confirm`; `confirm`/`cancel` are refused with `WarNotRolled`,
    `abandon` is free before the roll). The AI sees it as `Action::Roll`
    per target. `events::blocked` (prevents *and* requires clauses: #13←#65, #7←#83, #56←#110, #59←#97; NATO needs #16/#23, Solidarity #68; `GameError::EventPrevented`/`EventRequires`) is the modelled "prevents"
    clause (a played card sits in `Hands::removed`, which is all `blocked` reads; `Action::Event` isn't offered). Brush War's NATO clause is (`War::with_lasting`).
    In interactive mode `e` jumps to the country screen of a lone
    target (Korean, Arab-Israeli) or to the region view of the first target
    (preferring the current region) with non-targets dimmed; `r` rolls on the
    country screen. Views: `render::render_war_result` modal (`Modal::War`), `war_line` in
    the region footer/country panel, a `war` log line.
  - `events::effects` (the second stage) resolves twenty-one cards whose text only moves
    influence, VP, DEFCON, or the China Card (`EffectResult::china`, a `ChinaTransfer` that `Game::finish_effect` applies; #58, #71, and #76 via `Mode::china`) by fixed amounts (see `CARDS.md`) into an
    `EffectResult` — `influence: Vec<InfluenceChange>` (country, side,
    before, after; no-op changes omitted), a signed `vp_delta` (positive
    favours the US, like `GameStatus::vp`), and `defcon: Option<(before,
    after)>` — as a pure function of `(map, board, status, card)` that
    applies nothing itself; `Game::play_event` writes it. Countries are
    looked up by name and the lookup panics on a typo, pinned by a unit
    test that resolves every card on a blank board. The "prevents/allows
    card #N" clauses on some cards aren't modelled: they only matter once
    that other card exists, and a played event's card already sits in
    `Hands`'s removed pile. Either side may play any card's event,
    its opponent's included — the event does what the card says whoever
    plays it (Duck and Cover pays the US even when the USSR plays it).
    Playing an opponent's card for its ops *as well* is still out of
    scope. DEFCON reaching 1 ends the game against the phasing player —
    whoever is playing the action round, not the card's side
    (`VictoryReason::Defcon`); DEFCON is applied before VP, so that
    loss outranks any VP the same card awards. A new card of this kind
    adds one function (`fn(&mut Ctx)`, reading as the card's rules text through `Ctx`'s `add`/`remove`/`set`/`take_control`/`award_vp`/`set_defcon` helpers, plus `start` for a turn-long effect — `EffectResult::ongoing`, see `ongoing` above) and one line in `effects`'s `EFFECTS` table — `is_effect_card` and `resolve` both read it, so there's no second list — a state in
    `data/states/events.json` with a test in `tests/states.rs`, and a ✅
    in `CARDS.md`.
  - `events::scoring` (rule 10.1) resolves any of the seven scoring cards
    into a per-side breakdown plus the VP swing it produces — pure
    functions of `(map, board, card)`, mirroring `ops::realign`'s own
    `modifiers`/`odds` split, so a preview needs no session open. Six
    cards (Asia, Europe, Middle East, Central America, Africa, South
    America) share one shape: a `RegionScoring` table entry (the
    presence/domination/control point values rule 10.1's own chart
    gives) and one `score_region` function computing, for each side, its
    `Tier` (`None`/`Presence`/`Domination`/`Control`, rule 10.1's own
    definitions — `Domination` needs more countries *and* more
    battlegrounds than the opponent, plus at least one battleground and
    one non-battleground of its own; `Control` needs more countries and
    *every* battleground in the region) and every VP contribution kept
    itemised in a `SideScore` (`tier_vp`, `battleground_vp`,
    `adjacency_vp`) rather than just summed, so a view can explain the
    number instead of just showing it — the same reasoning
    `realign::Modifiers` follows. The battleground and
    superpower-adjacency bonuses are computed uniformly across every
    card even though the Middle East/Africa/South America cards' own
    text omits the adjacency one — no country in any of those three
    regions actually borders a superpower (pinned by
    `tests/standard_map.rs::superpower_borders_match_the_real_board`), so
    the two are behaviourally identical and special-casing it per card
    would just be more code for the same result. Europe Scoring is the
    one card whose `Control` is an outright win (`ControlVp::AutomaticVictory`)
    rather than a VP value, surfaced as `ScoringResult::automatic_victory`.
    The seventh card, Southeast Asia Scoring, pays 1 VP per controlled
    country in the `SoutheastAsia` sub-region and 2 for Thailand — read
    off `Country::battleground` rather than hand-matched by name, since
    the map fix below leaves Thailand as that sub-region's *only*
    battleground, exactly the one the card's own text singles out for
    the higher value.
- **`Game`** (`src/game.rs`) — turns. Owns the status, the board, whichever
  card is in play, and whichever `Operation` is open, so "a turn plays a
  card, then spends its ops on one operation, then closing that operation
  passes the turn" lives in one place rather than being duplicated between
  the REPL and interactive mode. `play_card(cards, id)` is the first step
  of a turn now: it takes `id` from the active side's hand, making it
  whichever of `begin`/`play_event` funds next — refusing a card not
  actually available to the active side
  (`NotInHand`/`ChinaCardFaceDown`), a second card while one's already in
  play (`CardInPlay`), or anything at all once `Game::winner` is set
  (`GameOver`). The China Card (`CHINA_CARD`) is the one exception to
  "in your hand": it's playable instead whenever `GameStatus::china_card`
  names the active side and `china_card_face_up` is true
  (`ChinaCardFaceDown` otherwise — see `Hands`'s own doc for why it's
  never in a hand list at all). `begin(kind)` is the entire *operation*
  enforcement mechanism: it always opens with `active()` and the played
  card's own ops (refused with `NoCard` if none has been played, or
  `ScoringCard` if it has no ops to spend — a scoring card can only fund
  `play_event` instead), so there's no argument through which a caller
  could name the wrong side or the wrong ops count, and it's refused
  while an operation is already open. One turn spends exactly one card on
  exactly one operation —
  `confirm`/`cancel` are the only two ways to close one *for real*, and
  both discard the played card (sending it to `Hands`'s own discard pile,
  or — for the China Card — passing it face down to the opponent instead,
  via the private `discard_played_card`) and hand the turn to the other
  side via the private `advance` (USSR → USA; USA → USSR plus
  `action_round += 1`, rolling `turn` over — clearing `GameStatus::effects`
  (every "remainder of the turn" event ends there), and flipping the China Card
  face up again, wherever it's landed — once `action_round` exceeds
  `action_rounds_per_turn`). **Events are the exception to "one operation
  spends the card"**: a choice card's `play_event` opens `Operation::Event`
  instead of resolving, and `confirm` finishes it (refused with
  `GameError::EventIncomplete` until `EventChoice::is_complete`), applying
  it through the same private `finish_effect` fixed-effect cards use, then
  handing the turn over. `cancel` is refused (`CannotCancelEvent`);
  `abandon` is allowed only with nothing picked *and* by the phasing side
  that played the card (`CannotAbandonEvent` otherwise — the opponent
  can't undo it). `place` is the `+` step, new `unplace` the `-` step (for
  a placement it takes back a pending point in *that* country; for an
  event it removes influence or takes back a staged add), `choose_mode`
  picks a mode, and `undo` takes back the latest pick.
  **`Game::decider()`** is who has to act next — the open event's chooser,
  else `active()` — and is what every AI hook and the status bar read
  instead of `active()`. `pass` is the same handover with no operation
  opened, refused with `CardInPlay` if a card's already been taken from
  the hand — there's nothing left to "pass" on at that point, so
  `return_card` is the way out instead. Ending a turn with ops unspent is
  allowed and simply forfeits them, the same as `InfluencePlacement` never
  requiring every op to be spent. `abandon` is the third, narrower way to
  close an *operation* — with **no turn cost**, exactly as if `begin` had
  never been called, and leaving the card in play rather than discarding
  it — the free undo for opening the wrong kind of operation by mistake,
  distinct from `cancel`'s "I'm done, whatever happened stands" (which
  always hands the turn over, even with nothing spent). What it refuses
  differs by kind, since what counts as irreversible does: an
  `InfluencePlacement` can always be abandoned, however many points are
  pending — placement never rolls a die, so nothing about it is hidden or
  committed until `confirm` runs, and every pending point is simply
  discarded (the same board effect `cancel` has on a placement, minus the
  turn cost). A `Realignment`/`Coup` can only be abandoned *before* its
  first roll or attempt — refused the instant `Operation::ops_spent() > 0`
  for either of those two kinds, since a roll writes straight to the real
  board and reveals a result that can't be un-rolled; `cancel` is the only
  way to close one from there. `return_card` is `abandon`'s card-level
  counterpart: with no operation open, it puts the card currently in play
  back in the hand at the index it came from (refused, `OperationOpen`, if
  one's still open — abandon that first) — so a mistaken `play_card` has
  the same free, no-turn-cost undo an operation does, just one level
  further out. Neither `abandon` nor `return_card` leaves a log entry — as
  far as the history is concerned, neither ever happened — unlike
  `confirm`/`cancel`, which always push a `Closed` entry (now naming the
  card that funded the operation) even when nothing was spent. `Game`
  takes no `WorldMap`, `Dice`, or `CardCatalog` of its own — all are
  passed per call, matching how the ops modules already split `Board` out
  — and is cheap to `Clone`, which is the whole point: nothing in this
  module touches a terminal, so it's the complete surface a future AI
  opponent drives, and lookahead means cloning a `Game` to try a line of
  play without touching the real one. `Game::lookahead` clones status,
  board, the card in play, and open operation like `Clone` does, but
  starts the copy's log empty, since the log is the one field whose size
  isn't bounded and a search cloning many nodes shouldn't drag a growing
  history through every branch it never plays out. `Game` also carries the
  scenario's starting `Hands`, read-only via `Game::hand(side)` (always
  reflecting what `play_card`/`return_card` have done to it — there's
  still no draw or redeal) and `Game::card_in_play()`, which names
  whichever card `play_card` has taken but not yet discarded.
  `Game::card_in_play_slot()` pairs that id with where it came from
  (`PlayedCard::hand_index` — `None` for the China Card) purely so
  `render::render_hand` can splice it back into its old spot rather than
  just letting it disappear from the strip; see that function's own doc.
  `play_event(map, cards)` sits alongside `begin` as the other thing a
  played card can fund: it resolves the card's own text via
  `events::resolve` (refused with `EventNotImplemented` for anything
  `events::is_implemented` doesn't recognise yet), applies the resulting
  VP (clamped to ±20 rule 5.5's cap, via the private `apply_vp`), and
  either discards the card (or, for a `removed_after_event` card, sends
  it to `Hands::remove_from_game` instead — never reshuffled back in) and
  hands the turn over the same way `confirm`/`cancel` do, or — if the VP
  cap was just hit, or the event is an outright win (Europe Scoring's
  Control tier) — leaves the turn exactly where it ended instead, since
  there's nothing left to hand over. Nothing here yet lets a card be
  played for ops *and* its event in either order (the dual-use rule for
  an opponent's card) — every event implemented so far (scoring cards)
  has no ops to combine with, so `PlayedCard` has no "which half is still
  open" state; a future stage that adds an ops-and-event card will need
  to grow that, not reshape it. `Game::winner()` reports the result once
  either path sets it (a `Victory { side, reason }`, `reason` one of
  `VictoryReason::Vp`/`EuropeControl`) — never clears, and from then on
  `play_card`/`begin`/`pass` all refuse with `GameError::GameOver`.
  **Turn-long effects** (see `ongoing` above) touch `Game` in four places.
  `finish_effect` records an `EffectResult::ongoing` into
  `status.effects` (Chernobyl arrives via `confirm` of its choice session
  like any choice card). `begin` reads `TurnEffects::card_ops` for the
  card's real ops and builds the operation with the matching hooks.
  `roll` calls the private `coup_aftermath` after a coup attempt: a coup
  in a **battleground** lowers DEFCON by 1 (rule 6.3.4 — reaching 1 loses
  the game for the *phasing* side via `apply_defcon`) unless Nuclear Subs
  spares a US one, and Yuri and Samantha pays the USSR 1 VP per US coup;
  DEFCON is applied before VP, and the result is one
  `Event::CoupAftermath(CoupAftermath)` right after the coup's own log
  entry (not logged when nothing happened), readable as
  `Game::last_coup_aftermath`. `advance` implements North Sea Oil: when
  the US finishes the last action round with it in force, the round
  counter goes one past `action_rounds_per_turn` with the US still
  active (the USSR sits it out) and `GameStatus::validate` accepts exactly
  that status; the turn rolls over after that extra round. "Prevents
  #61 OPEC" is not modelled.
- **`action`** (`src/action.rs`) — the surface an AI opponent drives
  instead of calling `Game`'s own methods directly: `Action`, one legal
  forward move (`PlayCard`/`Begin`/`Event`/`Place`/`Roll`/`Confirm`/`Pass`),
  plus `Game::legal_actions` (every legal `Action` for whoever's active
  right now, in a fixed order so a seeded AI's choices stay reproducible)
  and `Game::apply` (a thin dispatch onto the `Game` method each variant
  names — `Event` onto `play_event`; `Place`/`Unplace`/`ChooseMode` are an open
  event's `+`/`-`/mode — `Unplace` and an event's `Place` only where they
  are *forward* steps, never mere take-backs, and an event's `Confirm`
  only once it's complete — all offered for `Game::decider`, not
  `active`). Deliberately forward moves only —
  never `undo`/`abandon`/`return_card` (human-only take-backs an AI never
  needs, since it simply doesn't choose the action it'd be undoing) or
  `cancel` (every board outcome it can produce is already reachable
  through `confirm` alone). With a card in play and no operation open,
  `Event` is offered whenever `events::is_implemented` recognises it
  (whichever side's card it is), and the three `Begin` kinds
  whenever the card isn't a scoring card (one has no ops for `Begin` to
  spend) — so a fixed-effect card offers both. A scoring card offering
  only `Event` is most of what guarantees the list is never empty and a
  random walk through it can't stall: every action either spends ops or
  closes/opens a step, so repeated `legal_actions`/`apply` calls always
  reach a `Confirm` or `Pass` that hands the turn over — except once
  `Game::winner` is set, the one case `legal_actions` returns empty
  outright, since there's nothing left to do.
- **`ai`** (`src/ai/`) — the framework built on `action.rs`: the `Ai`
  trait (`choose`, given a game, its map/cards, and the current
  `legal_actions` list, picks one of them) and `play_turn`, which drives
  one `Ai` through a whole turn — `legal_actions` → `choose` → `apply`,
  looped until `Game::active` or `Game::decider` changes (a choice event
  hands the decision to the card's own side mid-turn, so one call can stop
  before the turn ends — callers loop) *or* `Game::winner` is set (a
  scoring event can end the game mid-turn without `active` ever changing,
  and `legal_actions` would otherwise come back empty, which `Ai::choose`
  is documented to never see) — from any point mid-turn, not just a
  turn's very start. `ai::random::RandomAi` is the first implementation:
  picks uniformly among whatever's legal, via its own `Dice` (seeded
  independently of the game's own, so an AI's choices never shift a
  realignment's or coup's die sequence). Wired into both `main.rs` (an
  `ai`/`ai us|ussr|off` REPL command, `--ai us|ussr` at launch) and
  `interactive.rs` (the same auto-play, driven on every keypress that
  might have handed the turn to the AI's side) — see each file's own notes
  below.
- **`log`** (`src/log.rs`) — `GameLog`, the game's append-only history, a
  `Vec<LogEntry>` built up entirely inside `Game` — the one place every
  mutation already funnels through — so the REPL and `interactive.rs` are
  both covered without either having to remember to log anything. This is
  a record of the actual game, not of every step taken inside the
  application: `Game::play_card` itself writes nothing, since a card
  that's merely *selected* can still be taken back with no trace
  (`Game::return_card`, with or without an abandoned operation in
  between). `Game::log_card_selected` is what actually pushes
  `Event::Selected`, and only once that selection is irrevocable — which
  still lands it, in the finished log, *before* the operation it funds has
  necessarily closed, reading in the order things actually happened
  rather than as a later afterthought. Where "irrevocable" falls differs
  by what the card ends up funding: `Game::roll` calls
  `log_card_selected` itself, right before a realignment's or coup's
  *first* roll — the exact instant `Game::abandon` stops being able to
  undo it (idempotent past that: a second roll in the same realignment
  doesn't log the card again). Everything else — a placement confirmed or
  cancelled with any (or no) points pending, or a realignment/coup that
  never rolled at all — only becomes real at `confirm`/`cancel`, so
  `log_close` calls `log_card_selected` itself, as the first thing it
  does (a no-op if a roll already did). Every operation closes with an
  `Event::Closed` entry (pushed from `confirm`/`cancel`, stamped with the
  turn/AR/side *before* `advance` runs) naming the operation kind and its
  final ops balance — not which card funded it, since the `Selected`
  entry already said so, earlier in the same log — mirroring how a
  realignment's or coup's dice already have their own entries by then
  (`Event::Realign`/`Event::Coup`, pushed the instant `Game::roll`
  resolves, since a roll is irreversible the moment it happens), and an
  influence placement's points get the same treatment: `Event::Placed`,
  pushed by `log_close` immediately before `Closed`, the placement
  analogue of a resolved roll (omitted entirely if nothing was placed, the
  same way zero rolls simply mean zero `Event::Realign` entries) — so
  `log`/`export` show `confirm`/`cancel` as its own line for every
  operation kind, never merged onto the line reporting what happened. An
  *abandoned* operation (`Game::abandon`) still leaves no trace of
  itself — no `Placed`/`Realign`/`Coup`/`Closed`/`Selected` entry, since
  abandon is only ever possible before `log_card_selected` has run for
  either of its two triggers. `Event::Pass` covers `Game::pass`.
  `Game::play_event` pushes its own card's `Selected` entry too (the same
  call, since a card funding an event is just as irrevocable from that
  point on as one funding an operation), then `Event::Scored` — the
  result plus the VP track's new value, since the result alone only
  carries the *change* — and, if the event just ended the game,
  `Event::GameOver` right after it; a fixed-effect card pushes
  `Event::EventResolved { result, vp_after }` in `Scored`'s place
  (`event` line in the rendered log). `Game::board_mut` is the one mutator
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
  `Canvas::blit` opaquely copies one canvas onto another at a given
  anchor (including its blank cells, so it paints over what was there
  rather than leaving a card-shaped hole) — `interactive.rs` uses it to
  draw a zoomed card's detail over whichever map screen is showing.
  - `world.rs` — the six-region dashboard (`map`/`world` command).
  - `chip.rs` — the shared country-box chip (flag, name, influence,
    control glyph, `st<n>`, and the operation badge) and the connector
    glyph drawn between two grid-adjacent chips (`ChipGrid::draw_edge`,
    merging a `╲`/`╱` collision into `╳`), both placed at a pitch derived
    from a caller-chosen chip width and anchored wherever a caller's own
    cell space starts — `anchor` is a signed row/col pair rather than a
    `Cell`, since the country view's mini-map centres on a cell that can
    sit on a region grid's own top or left edge, one row/column short of
    a real (non-negative) `Cell` to anchor on. Pulled out of `region.rs`
    (below) once `country.rs`'s neighbourhood mini-map needed the
    identical drawing at a different (wider, differently-anchored) pitch;
    `ChipRole` (`Selected`/`Native`/`Foreign`) captures the three ways a
    chip is styled — thick bright border, region-tinted,
    region-tinted-but-never-dimmed — that both callers need.
  - `region.rs` — one region zoomed in, with real adjacency connectors
    (`region <name>` / `1`-`6`), its chips drawn via `chip.rs` at
    `REGION_CHIP_W` (`ChipRole::Selected`/`Native` for a native country,
    `ChipRole::Foreign` for a guest). Every adjacency that leaves the
    region — to another region's country, or to a superpower — is drawn
    too, as a guest chip (`MapLayout::guests`): a foreign country tinted
    with `region_color` for *its own* region (never thick-bordered, since
    a guest can't be the selection, and never dimmed for operation
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
    with up to three panels, each sized so navigating between countries
    with arrow keys never moves the box's own borders — see each panel
    below for how. Country (influence, control, sub-regions) always
    reserves a sub-regions row, left blank for a country with none,
    rather than only drawing it sometimes, so the box is the same height
    whether or not the viewed country happens to have one. Neighbours —
    a mini-map (`neighbourhood`), not a text list: the country itself as
    its own centre chip (`ChipRole::Selected`) and each immediate
    neighbour (`Country::adjacent`/`adjacent_superpowers`) placed, via
    `chip.rs`, at whichever cell — native, or a guest cell in this
    country's own region — is really grid-adjacent to it, the same
    geometry `region.rs` draws a whole region's grid from. The mini-map
    is a fixed 3×3 window of that grid centred on the viewed country
    (`ChipGrid`'s `anchor` one row/column above-left of its own cell) —
    every country's neighbours land within it, so the size never changes
    and the country itself is always the middle chip, even sitting on a
    region's own edge with no neighbour on one or more sides. Its chips
    are sized off the longest country name in the *whole game* (not
    `MapLayout::short_name` — this view has the room, and a neighbour
    list previously named each one in full), not just whoever's actually
    a neighbour here, so the box is exactly as wide for a country whose
    neighbours have short names as one whose neighbours don't — what
    changes while arrowing between countries is only which chips are
    filled in, never their size. A neighbour is `ChipRole::Foreign`,
    tinted by its own region exactly like a region view's guest chip, so
    a neighbour in a different region reads as visibly foreign — and
    where a chip sits is where this screen's own arrow keys go, since
    both walk the same `MapLayout::step_country` geometry. Anything that
    couldn't be placed this way (empty on the standard layout — see
    `every_country_places_all_its_neighbours_on_the_country_view_grid` in
    `tests/render.rs`) is footnoted below the grid, alongside — while a
    realignment is open — which neighbours are supplying its
    `adjacent_controlled` modifier (`+1 realign from …`; a chip has no
    free stats-row slot left for that marker itself). Third,
    only while an operation is open, Operation, showing exactly what the
    region footer's own breakdown shows (a placement's cost and pending
    count; a realignment's per-side modifiers and odds; a coup's target
    number and odds), titled with `operation_header` and its first row
    always `operation_touched_line`. `render_country` takes a `MapLayout`
    (for that touched-line and the country/neighbour cells) and a
    `ViewMode` (`Static`/`Interactive`) that gates a key-hint row below
    the box — `Static` for the REPL's one-shot prints, `Interactive` for
    the screen `interactive.rs` opens. Like `region.rs`/`worldmap.rs`, it
    opens by substituting `op.board()` when the operation has a
    speculative one, so a placement's pending influence shows here too.
    Every panel but the mini-map itself is built as a `Vec<(String,
    Style)>` before drawing, mirroring `region.rs`'s `build_footer_lines`;
    two small `Canvas` primitives exist for this layout specifically —
    `draw_divider` for the `├─ Title ─┤` rows between panels, and the free
    function `put_border_title` for the outer box's own
    `┌─ * Poland ─── Europe · stability 3 ─┐` top border.
  - `log.rs` — turns a `GameLog` into text: `log_entry_line` (now taking a
    `CardCatalog` alongside the `WorldMap`, so an `Event::Selected`
    entry's own card can be named) is the canonical rendering of one
    entry, fixed-column and tagged so a roll's numbers can't be mistaken
    for each other (`d6:`
    only ever the actual die; `mod:`/`ops:`/`target:`/`sum:` label
    everything else by where it came from — a bare `4+4=8` doesn't say
    which 4 was rolled). `log_text` joins a header plus every
    `log_entry_line` into the exact string `export` writes to a file;
    `render_log` draws the same lines into a `Canvas`, coloured by side
    (`Color::Us`/`Color::Ussr`, `Muted` for a debug edit or note) —
    `render_log(..).render(ColorMode::Never)` is byte-identical to
    `log_text`, so the on-screen `log` command and the exported file are
    guaranteed to be one format, not two. `Event::Scored` gets its own
    `score` line (each side's tier and VP contributions, the net swing,
    the new VP total) and `Event::GameOver` a `gameover` one (the winner
    and why) — both dense one-liners for this view specifically; the
    interactive map's own modal (`score.rs` below) spells the same result
    out as a titled box instead.
  - `event.rs` — `render_event_result`, `score.rs`'s analogue for a
    fixed-effect card: a titled box with one row per influence change,
    the DEFCON move, the VP swing and new total, and a game-over line if
    it ended the game (`winner` is passed in, since the result alone
    can't say). Same `(n, total)` queue position as the other modals.
  - `statusbar.rs` — the turn/operation bar (turn row, card/operation row,
    then a row listing the turn-long effects in force — each in its
    beneficiary's colour via `ongoing_effect_line`, or a muted "no turn
    effects" so the height never moves — and the rule below) `interactive.rs` draws
    above every map screen: turn/AR/active side (its own colour)/DEFCON/
    VP, then one of four states for the card/operation row — a set
    `Game::winner`, which replaces the row entirely with
    `game_over_line` (shared with `main.rs`'s own `status`/`event`
    commands, so the wording can't drift between the two); otherwise, no
    card in play (a prompt naming the keys to select and play one); a
    card in play with no operation open yet (its name and the keys to
    spend or return it — `e score` in place of the ops keys for a
    scoring card, since it has none to spend; `e event` ahead of them
    for a card whose own event is implemented); or an open operation
    (`operation_balance_line`, reused rather than reworded, prefixed with
    the card's name). The only view besides `world.rs` that reads a
    `GameStatus`, deliberately its own
    compact line rather than the dashboard's (already clipped at width
    104) — the two share only `vp_line`'s
    wording, extracted so they can't drift. Always exactly
    `STATUS_BAR_ROWS` (4) regardless of whether an operation is open, so
    the view drawn below it never shifts; takes `width` as a minimum and
    grows to fit its own content instead of clipping.
  - `hand.rs` — `render_hand`, a side's hand as a strip of mini-card boxes
    (5 per row, 2 rows — a 9-card hand plus the China Card fills all 10
    slots), meant to be drawn below every map screen by `interactive.rs`
    so a player can browse their cards and the board at once. Always
    `HAND_ROWS` (9) tall and `HAND_WIDTH` wide regardless of hand size,
    the same "never shifts what's drawn around it" rule `statusbar.rs`
    follows. Each slot shows its ops value (or `S` for a scoring card),
    its truncated name, its side/phase, and a `*` if it's removed from
    play once played as an event; the selected slot (if any) is drawn
    with `Canvas::draw_thick_box` like a region view's own selection. The
    China Card, when `render_hand`'s `china: Option<bool>` is `Some`
    (whether it's face up), is always the hand's final slot — it's never
    read from the hand list itself, since `Hands` deliberately never
    carries it (see `cards.rs`'s own doc). More than 10 cards paginates
    around whichever one is selected rather than shrinking the slots.
    `render_hand`'s `in_play: Option<(CardId, Option<usize>)>` — exactly
    `Game::card_in_play_slot()`'s own shape — is how a played card stays
    on screen instead of vanishing the instant `Game::play_card` removes
    it from the hand: the id is spliced back into its original index (the
    `Option<usize>`; `None` for the China Card, which was never spliced
    out of the hand list to begin with — it's drawn via `china`
    regardless of whether it's the one in play) and drawn with
    `SlotRole::Played` — a bold, bright `Color::Selected` thick box and an
    `IN PLAY` line in place of its usual side/phase one — while every
    *other* slot drawn that call is `SlotRole::Dimmed` instead of
    `SlotRole::Selected`, so the one card that matters isn't competing
    with a leftover browsing cursor. `in_play` stays non-`None` for as
    long as the card does — through a whole operation, not just the
    keypress that played it — since `Game::confirm`/`cancel` are the only
    things that ever clear it.
  - `card.rs` — `render_card`, one card's full detail: a titled box like
    `country.rs`'s own outer box (the title on the top border via
    `put_border_title`, not a divider — its left half is the card's own
    ops value, `Card::ops_label`'s "Ops N" wording, or `Scoring` for a
    scoring card, since "Ops 0" would be technically true but misleading;
    its id is never repeated up here, only in the footer's own
    `card #N`), with the card's rules text word-wrapped to a fixed width
    (the `wrap` free function in `mod.rs`, used nowhere else in `render/`)
    and a flags line (optional/removed-after-event/ongoing/scoring,
    whichever apply) — wrapped the same way the body text is, since a
    card can carry several at once and the joined line can run well past
    the box's own width — above the card's printed number. This is the
    "zoom" view `interactive.rs` overlays on the current map screen. For
    the China Card specifically, the flags line is replaced with its
    face-up/down status (`china_face_up`), since the normal flags say
    nothing useful about it.
  - `roll.rs` — `render_roll_result`, the interactive map's post-roll
    modal for a resolved realignment roll or coup attempt: a titled box
    (`Realignment · Poland` / `Coup · Iran`) spelling out each side's die
    and modifier breakdown (`Modifiers::reasons`, word-wrapped onto its
    own indented line so a long reason list never widens the box), who
    won and by how much, the resulting influence change in the target
    country, and a `Control: X → Y` line if that flipped — bordered in
    the winner's colour (`Muted` for a tie or a failed coup). Takes a
    `RollReport` (`side`, the `RollOutcome` `Game::roll` returned, and the
    target's influence for *both* sides just before the roll — the one
    thing the result itself doesn't carry, since it only records what
    changed) and an optional `(n, total)` queue position, appended to the
    "Enter to continue" hint when more rolls are queued behind this one.
    Replaces the dense one-line `roll_result_line`/`coup_result_line`
    summaries for interactive mode specifically; those two stay as they
    are for the REPL's own `roll` command, which has no modal to show.
  - `score.rs` — `render_scoring_result`, the Scoring analogue of
    `roll.rs`'s own modal, for a resolved scoring-card event: a titled
    box (named after the card) giving each side's own tier, its
    battleground/adjacency/tier VP contributions and their total
    (`push_side_lines`, wrapped the same "don't widen the box" way
    `roll.rs`'s modifier breakdown is), the net VP change and the VP
    track's new value, and — for Europe Scoring's Control tier — who
    just won the game, bordered in the winning side's colour. Southeast
    Asia Scoring's own `ScoringKind` has no tiers to show, so it lists
    each controlled country and who holds it instead. Takes the same
    `(n, total)` queue position `render_roll_result` does, since
    `interactive.rs` queues the two kinds of modal together.

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
  tallies could. With no operation open, all three views' own hint rows
  also advertise the keys that start one, from a single `BEGIN_HINT`
  const in `render/mod.rs` so the fragment exists once rather than three
  times; with one open, each view's own placement/realign/coup hint
  additionally names Backspace/abandon (`Game::abandon`'s key), alongside
  whichever kind-specific keys (`+ place`/`u undo`/`r roll`/`r coup`) it
  already listed.
- **`main.rs`** — a REPL (`cargo run`) plus one-shot mode
  (`cargo run -- <command>`). Type `help` inside the REPL for the command
  list. `Session` holds a `Game` (`src/game.rs`), so turns are enforced
  everywhere the REPL touches the board. `play <id|name>` is the new first
  step of a turn: it takes a card from the active side's hand (the same
  forgiving `CardCatalog::find` lookup `card` uses) and makes it the card
  `Game::begin` will spend, or `Game::play_event` will resolve instead;
  `influence`/`realign`/`coup`/`event` all take no arguments — the side
  (and, for the first three, the ops) are always the played card's,
  never typed in — and `place`/`undo`/`confirm`/`cancel`/
  `roll` all read and write through `Session.game` rather than a bare
  `Board` and `Option<Operation>`. `influence`/`place`/`undo`/`confirm`/
  `cancel` stage and commit an influence placement; `realign`/`roll`/
  `confirm`/`cancel` run a realignment and `coup`/`roll`/`confirm`/
  `cancel` a coup, where `roll` resolves either kind immediately (a coup's
  `roll` spends every op on its one attempt) without ending the turn, and
  `undo` always refuses for either. `confirm` and `cancel` are the only
  two ways to close an operation, and both discard the card that funded
  it (the China Card instead passes face down to the opponent) and hand
  the turn to the other side (reported in the next prompt, which names
  the active side, the AR counter, and any card still in play); `pass`
  does the same handover with no operation open, refused if one is, or if
  a card's been played but not yet spent. `+ <country> [n]` (alias
  `place`) and `- <country> [n]` (alias `take`) are the REPL's `+`/`-`
  steps — for an open event they add/remove influence (listing what's
  available when the event opens), for a placement `-` takes back pending
  points in that country; `mode <n>` (1-based) picks a multi-mode event's
  mode. The prompt names the chooser while an event is open, and
  `maybe_run_ai_turn` triggers on `Game::decider()`. `abandon` steps back exactly
  one level, the same cascade Backspace drives in the interactive map:
  with an operation open, it closes *that* for free (always for an
  influence placement, discarding any pending points; refused, naming the
  roll already spent, once a realignment or coup has rolled), leaving the
  card in play; with no operation open but a card in play, it returns the
  card to the hand instead (`Game::return_card`) — the free undo for a
  mistaken `play`/`influence`/`realign`/`coup`. `event` resolves the card
  in play's own text (so far, only a scoring card's — anything else is
  refused with `GameError::EventNotImplemented`'s own text), printing
  `render::render_scoring_result`'s same breakdown and, if it just ended
  the game, `render::game_over_line` right after it. `status` reports the
  turn/AR/active side, whichever card is in play, the open operation's
  balance, and — once set — `Game::winner` via that same
  `game_over_line`. `play`/`influence`/`realign`/`coup`/`event`/`pass`/
  `abandon` are no longer REPL-only — the same seven are bound to
  `space`/`i`/`a`/`o`/`e`/`p`/Backspace inside `interactive.rs`, which is
  also where `confirm`/`cancel` stay bound to `c`/`X`; the `wm`
  arm no longer reports a confirm/cancel/pass that happened inside the
  map (interactive mode's own message row already did), only a session
  left open on the way out. Either way, `set`/`add`/`remove`/`load` are refused while a
  session (`Game::operation()`) is open, since they'd shift the board an
  operation was judged legal against; when they do run, they call
  `Game::record_edit`/`record_note` themselves, since `Game::board_mut`
  can't observe what a caller does with it.

  **Debug mode** (`Session.debug`, shown as `[debug] ` in the prompt,
  toggled by `debug [on|off]`) gates every other way of editing state
  directly rather than playing to it: `set`/`add`/`remove` (as above) and
  `clear <country>|all` touch the board the same way `Game::board_mut`
  always did; `vp`/`defcon`/`turn`/`ar`/`active`/`china us|ussr [up|down]`
  touch the status via `Game::status_mut`, but — unlike `board_mut`'s own
  callers — never land a nonsensical value: each goes through
  `debug_apply_status`, which mutates a *copy* of the live status,
  checks it with `GameStatus::validate`, and only commits (via
  `status_mut`) if that passes, printing the `StatusError` and changing
  nothing otherwise (`vp 30`, `defcon 100`, `turn 0`, and `ar` past the
  current `action_rounds_per_turn` are all refused this way). `give
  us|ussr <card>`/`discard <card>`/`exile <card>` move a card between a
  hand and the discard/removed piles via `Game::hands_mut` and
  `Hands::take` (picks a card up from wherever it currently is) paired
  with `push_to_hand`/`discard`/`remove_from_game` — all three refuse the
  China Card, which `Hands` never carries (`china us|ussr` is the way to
  move *that*), and `give` additionally refuses once the destination hand
  is already at `cards::MAX_HAND_SIZE` (unless the card's already there,
  in which case the move is a no-op `take`-then-`push_to_hand` right back
  into the same hand, so it can never be what pushes it over); and
  `blank` resets to `Scenario::blank`. Every one of these is refused
  outright with debug mode off, refused the same way `set` already was
  while an operation is open, and (every one except `set`/`add`/`remove`/
  `clear`, which only ever touched the board) also refused with a card in
  play, since rewriting a hand or the status out from under a played card
  could leave it pointing nowhere sane — see `debug_guard`/
  `debug_guard_strict`. Every edit calls `Game::record_edit`/
  `record_note` itself, the same obligation `set`/`add`/`remove` already
  had, so none of this is invisible in `log`/`export`. Still deliberately
  no range check on `set`/`add`/`remove` themselves — an over-stacked
  country was already possible before debug mode existed, and influence
  has no fixed cap to check against the way VP/DEFCON/hand size do.

  **Named test states** (`src/states.rs`, `data/states/`) are reached
  through `Session.states: StateLibrary`: `states [file]` lists them with
  their own descriptions; `save <file>/<name> [description...]` snapshots
  the live game (`Game::snapshot`, refused with an operation open or a
  card in play — a `Scenario` has no room for either) and writes it,
  creating the file if needed; `load <file>/<name>` (alongside the
  existing `load demo`) resolves one back into a real `Scenario` and
  — unlike `load demo` — turns debug mode on automatically, since that's
  exactly what a named test state is for. `--state <file>/<name>` loads
  one at launch, before one-shot mode's single command runs (if any),
  composing with `--ai`.

  The REPL's own line editor is `rustyline` (the crate's one dependency
  besides `crossterm`/`serde`), not bare `stdin().read_line`, so Tab
  completion is available: `completion::TsHelper` (`src/completion.rs`)
  owns the fixed candidate lists (command names from the `COMMANDS` const,
  every card name, every country name — built once at startup, before
  `map`/`cards` move into `Session`) plus the `StateLibrary` itself,
  re-read fresh on every keystroke so a state `save`d moments ago
  completes right away. The actual matching (which command's argument
  completes against which list, and from what position — a card or
  country name is routinely several words, so completion has to replace
  the *whole* typed-so-far argument, not just the last word) is the pure,
  separately-unit-tested `completion::candidates`, kept apart from
  `TsHelper`'s `rustyline::completion::Completer` impl so it needs no
  terminal, catalog, or file I/O to test. `COMMANDS` is checked against
  `print_help`'s own `HELP_TEXT` by
  `tests::every_command_is_mentioned_in_help`, rather than generating one
  from the other, so a command added to one and forgotten in the other
  fails the build instead of quietly drifting.

  `log [n]` (or `history`)
  prints the game's history so far, or just the last `n` entries, via
  `render::render_log`; `export <path>` writes it to a file as plain text
  via `render::log_text` — the same bytes `log` shows, minus colour — and
  is the crate's first use of `std::fs`, since every other data file is
  `include_str!`-embedded rather than read or written at runtime.
  `--seed <n>` (or the REPL's `seed <n>`) controls `Session.dice`, which
  stays outside `Game` so it can be seeded independently and so
  `Game::roll` stays deterministic given its inputs. `Session` also holds
  a `CardCatalog`; `hand [us|ussr]` (default: the active side) and
  `card <id|name>` print the static, unselected `render_hand`/
  `render_card` views — the same ones `interactive.rs` draws with a
  selection. `Session` also carries a `RandomAi` (`Session.ai`) and
  `Session.ai_side: Option<Superpower>` — at most one side, so the REPL
  loop can never run without a human typing anything. `--ai us|ussr` sets
  `ai_side` at launch; the REPL's `ai` command plays the active side's
  current turn once via `ai::play_turn` regardless of `ai_side`, `ai
  us|ussr` turns auto-play on for that side from here on, and `ai off`
  turns it back off. `maybe_run_ai_turn` runs after every REPL command (and
  once at startup, in case the scenario's starting side is already the
  AI's) and, whenever `ai_side` matches `Game::active`, plays that whole
  turn and echoes each new log line via `render::log_entry_line` — the
  same wording `log` shows. The AI's own `Dice` is seeded independently of
  `Session.dice` (xored with a constant salt when `--seed` is given), so
  `--seed`/`seed <n>` keep controlling realignment and coup rolls only,
  never which moves the AI picks.
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
  every key handler goes through it and turns stay enforced here too, and
  a status bar (`render::render_status_bar`) is drawn above
  whichever screen is showing — turn/AR/active side (in its own colour)/
  DEFCON/VP, then one of its four states for the card/operation row (see
  `statusbar.rs` above) — since a keypress alone carries no "USSR" the way
  the REPL prompt does. A turn now starts with `Space`, which plays the
  selected hand card (`Game::play_card`) — a no-op on an empty hand, or
  once a card's already in play. Only then do `i`/`a`/`o` open an
  influence placement, realignment, or coup for the active side, spending
  that card's own ops, `e` resolves its event instead
  (`Game::play_event` — so far, the only way to play a scoring card,
  since it has no ops for `i`/`a`/`o` to spend), and `p` passes the turn
  (`Game::pass`) — all five global, working from any of the three
  screens, and refused (`GameError`'s own text shown in the message row)
  while an operation is already open, or — for `i`/`a`/`o`/`e` — with no
  card in play yet, or — for `p` — with a card in play but no operation
  open: `Game::begin` and `Game::pass` always act for `Game::active`, so
  there's no side to infer from the key itself, which is what makes a
  keypress binding possible at all. Once `Game::winner` is set — `e` can
  cause that directly, or an AI's turn can (see below) — every one of
  these five is refused the same way (`GameError::GameOver`'s own text),
  and the status bar's own row says why there's nothing left to do.
  `c` confirms/closes the open operation via `Game::confirm` and `X`
  cancels/closes it via `Game::cancel` — either way discarding the card
  that funded it (or, for the China Card, passing it face down to the
  opponent) and handing the turn to the other side *without leaving the
  map*, so the newly active side can immediately play its own card from
  the same screen. Backspace steps back exactly one level: with an
  operation open, it closes *that* via `Game::abandon` — no turn cost,
  always available for a placement (however many points are pending) but
  refused (with `GameError::CannotAbandon`'s own text) once a realignment
  or coup has rolled, where `X`/`cancel` is the only way out — leaving the
  card in play; with no operation open but a card in play, it returns the
  card to the hand instead, via `Game::return_card`. `run` therefore only
  returns (`io::Result<()>`) on `Esc` from the world view, `q`, or Ctrl-C,
  and the REPL reads `Game::operation()` itself to report a session left
  open, rather than switching on a return value naming what happened
  inside. An `InfluencePlacement` binds `+`/`=` to place one point, `-` to take
  back a pending point in the *selected* country, and `u`
  to undo the last one on *both* the region and country screens —
  placement is undoable, so it never needs the country screen's
  confirmation step, and stays a fast, stay-on-one-screen action from the
  region grid too. A `Realignment` or `Coup`, by contrast, only binds `r`
  — to roll (or attempt the coup) on the selected country — on the
  country screen, and `u` always refuses there: a resolved roll or
  attempt can't be taken back. `Esc`/`q` leave a still-open session
  untouched rather than clearing it, so it can be resumed from the REPL
  or by reopening the map. A roll's outcome, and a scoring event's own
  result — unlike a refusal, or a confirm/cancel/abandon/pass/begin/play
  report, all of which the status bar's own next redraw already
  reflects — aren't otherwise recoverable from the screen, so rather
  than a message-row line each opens its own modal
  (`render::render_roll_result`/`render_scoring_result`), blitted centred
  over whichever screen is showing (`blit_centred`, shared with the
  hand's zoomed-card overlay below) and dismissed only by Enter (or Esc,
  as a synonym) — every other key is swallowed while one's up, the same
  modal precedence the zoom overlay has, and the two can never be open
  together (both close the other the instant they'd open). The two share
  one queue (`modal: VecDeque<Modal>`, `Modal::Roll`/`Modal::Score`/`Modal::Event`)
  rather than a single slot, since a single keypress can still only
  produce one at a time but an AI's turn can make several (a multi-op
  realignment's rolls; never, so far, a mix of the two kinds, since a
  scoring card has no ops to open a roll-producing operation with); each
  is shown in order, and the hint row names its position once more than
  one is queued.
  An open `Operation::Event` uses the same `+`/`=`/`-`/`u` keys on the
  region and country screens (`Game::place`/`unplace`/`undo`), and the
  global digits `1`-`9` choose a mode (`Game::choose_mode`). The status bar
  names the *chooser* as the side to act, in their colour; chips show
  `+N`/`-N` for what's been staged (in the moved side's colour) and dim
  `↑N`/`↓N` for what each eligible country can still take. A chip the
  chooser can act on gets a double-line border (`Canvas::draw_double_box`,
  so it reads without colour), bold in its own region's tint; every other chip
  is muted all over, frame and numbers included; on the world map a live
  country's flag slot shows `+`/`-` (the next step's sign, `~` once
  changed) and the rest are muted. The footers carry a legend, and the region footer/country panel spell out what `+`/`-`
  would do on the selected one (`EventChoice::hint`). Confirming queues
  the same `Modal::Event` an immediate effect card does.
  `run` also takes `ai_side: Option<Superpower>` and `&mut RandomAi`,
  threaded through from `Session` — `maybe_run_ai_turn` runs before the
  very first draw and again after every handled keypress, and, whenever
  `ai_side` matches `Game::active` *and* `Game::winner` isn't already set
  (otherwise `Game::active` would still name the AI's side forever, with
  nothing left to play, and this would try to replay an already-finished
  turn on every later keypress), plays that whole turn via
  `ai::play_turn`, folds its log entries into one message line (closing
  any open zoom overlay and marking the message `sticky`, the one
  remaining use of that flag — the one-extra-keypress survival the AI's
  own summary line still needs, now that a roll's or a scoring event's
  outcome has its own modal instead), queues a `Modal` for each
  realignment roll, coup attempt, or scoring event the turn made
  (`queue_turn_modals`, walking the turn's own new log entries forward
  and pairing each roll off against `reconstruct_roll_reports`'s own
  output — which recovers each roll's "before" state, the one thing the
  log doesn't carry, by walking in reverse from the real board — while a
  `Scored` entry needs no reconstruction, since it already carries both
  the result and the VP it landed on) and queues them the same way the
  human's own `r`/`e` do, and leaves the message for that keypress's own
  `draw` call to show.

  The active side's hand (`render::render_hand`) is drawn as a fixed-
  height strip pinned below every screen — global, like the status bar
  above it, not tied to any one screen. `[`/`]` cycle its selection
  (wrapping at either end, and a no-op once a card's in play, since the
  played card's own slot is already gone from the strip) and `z` toggles a
  zoomed detail overlay (`render::render_card`, `Canvas::blit`ted onto the
  current screen, widening the canvas first if the card itself is bigger)
  for the selected card — both no-ops on an empty hand. The `selected_hand_card`
  helper resolves the strip's current selection to a `CardId` (the China
  Card once the index runs past the held hand — see `hand.rs` above),
  shared by `Space` and the zoom overlay so they always agree on which
  card is selected. Each side keeps its own selected index (`hand_selected`,
  indexed via `side_index`) so swapping the active side never loses the
  other side's place. `Esc` closes an open zoom first, before whatever it
  would otherwise do on that screen; `c`/`X`/`p` close it too, but only
  when they actually hand the turn over (a refusal leaves it open) — the
  newly active side's own hand takes its place either way. `[`/`]`/`z`
  are the hand strip's own keys and never trigger a card's event — only
  `e` does that, via `Game::play_event`, and only for whichever card is
  already in play.

  The only place in the crate that touches the terminal directly —
  everything it draws still comes from `render::render_world_map`/
  `render_region`/`render_country`/`render_status_bar`/`render_hand`/
  `render_card`, which stay pure `Canvas` producers.
  Uses `crossterm` — one of two non-serde dependencies, alongside
  `rustyline` (`main.rs`'s own line editor, not this module's).

## Data files (`data/`)

All game/display data is JSON or plain text, embedded into the binary via
`include_str!` rather than read at runtime — the data is part of the
build, not a runtime dependency. `data/states/` (below) is the one
deliberate exception, for debug-mode test states specifically.

- `standard_map.json` — the 84 countries and their adjacency (game rules
  data). Audited once against the real board (two independent open-source
  implementations cross-checked against each other) after it was found to
  drift from it in several ways: Laos and Cambodia were two separate
  countries rather than the single "Laos/Cambodia", an extra "China"
  country existed with no counterpart on the real board, several
  adjacencies were extra or missing (mostly in the Middle East, Africa,
  and Southeast Asia), and several battlegrounds were wrong (Middle East
  was missing Iraq/Libya/Saudi Arabia; Africa was missing South Africa;
  Central America had Nicaragua instead of Mexico; Asia wrongly had
  Vietnam/Indonesia/the Philippines). Pinned by
  `tests/standard_map.rs::{each_region_has_the_expected_country_count,
  battlegrounds_match_the_real_board, superpower_borders_match_the_real_board}`
  so a future edit can't silently drift again.
- `standard_layout.json` — display-only layout data (region-view cells,
  codes, world map positions, superpower boxes).
- `world_background.txt` — pre-rasterized ASCII/Unicode landmass art for
  the world map, generated once from real coastline data (not checked-in
  tooling — see `world_background.md`).
- `world_background.md` — **read this before changing the world map's
  background art, country positions, or region colour tinting.**
- `demo_state.json` — the bundled demo scenario's starting `Board` +
  `Hands` + `GameStatus`.
- `cards.json` — the 110-card standard deck (`CardCatalog`). Converted
  (one-off, not checked-in tooling) from `Twilight Struggle Cards.csv`,
  itself untracked and left in `data/` as the source: Mac-Roman decoded to
  UTF-8, its already-clean "Alternative Text" column used as each card's
  `text` (the CSV's own "Original Card Text" is hard-wrapped PDF text and
  isn't used), a trailing `*` in a card's name lifted into its own
  `removed_after_event` flag, and `scoring` derived as `ops == 0`.
- `backup/` — earlier full snapshots of the world map, kept in case a
  future change needs to compare against or revert to an earlier version.
- `states/` — named test states (`src/states.rs`'s own `StateLibrary`),
  one JSON file per topic (`scoring.json` is the first, then `events.json` for the fixed-effect cards, `choices.json` for the choice cards, `turn-effects.json` for the turn-long ones and `lasting.json` for the game-long ones — each card's own event, plus `*-active` states with an effect already in force), each holding a
  `{"states": [...]}` array of several named `Scenario` snapshots. Read
  from disk at runtime, not `include_str!`-embedded — see `states.rs`'s
  own doc above for why.

## Conventions

- Fail loud at load time: JSON loaders validate everything they can (see
  `MapError`/`LayoutError`/`ScenarioError`/`CardError`/`StateError`)
  rather than silently tolerating bad data.
- No `thiserror`/`anyhow` — plain enums implementing `Display` + `Error`.
- Views are pure functions returning a `Canvas`; never print/touch the
  terminal from inside `render/`.
- Snapshot tests live in `tests/snapshots/`; regenerate by running the
  relevant `cargo run -- --color never <command>` and diffing/replacing
  the snapshot file, not by hand-editing it.
- A new card implementation adds its own named test states
  (`data/states/<topic>.json`, via `src/states.rs`'s `StateLibrary`) and
  matching cases in `tests/states.rs`, the same way it's expected to add
  a snapshot test — see `states.rs`'s own doc above for the shape and
  `data/states/scoring.json`/`tests/states.rs` for the first example.

## Useful commands

```
cargo run                          # interactive REPL
cargo run -- worldmap              # one-shot: the whole-world map
cargo run -- region europe         # one-shot: zoom into a region
cargo run -- --state scoring/europe-ussr-control-wins
                                    # jump straight into a named test state
cargo test                         # all tests (unit + snapshot)
cargo clippy --all-targets         # lint; expected clean
```
